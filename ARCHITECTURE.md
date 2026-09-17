# Architecture

This document covers what's actually built (Phase 1 — see `ROADMAP_HONEST.md`), not the full platform vision. It explains *why* the non-obvious decisions were made, since that's the part that doesn't survive re-reading the code alone.

## Workspace layout

```
MudFish/
  Cargo.toml            workspace manifest
  crates/
    core/                domain types, URL normalization, CrawlConfig — no I/O
    frontier/             priority queue, dedup, termination protocol — no I/O
    fetch/                HTTP fetching, robots.txt, politeness, SSRF guard
    parser/               HTML parsing: links, metadata
  apps/
    mudfish/              CLI + async crawl controller (the only crate that wires
                          the others together); ships as both a lib (for testing
                          the controller without a subprocess) and a bin.
```

`core` has zero dependencies on the other crates and no I/O — everything else depends on it. `frontier`, `fetch`, and `parser` don't depend on each other. This isn't speculative layering: it's what let each one be tested in isolation (`cargo test -p mudfish-frontier` doesn't need a network), and it's what will let a future browser-backed fetcher slot in beside `HttpFetcher` without touching `frontier` or `parser` at all.

## The frontier's termination protocol

A pool of N async workers all pull from one `Frontier`. The hard problem: how does a worker know when to stop? Not "queue is empty right now" — another worker might be about to push more work. Not "block forever" — eventually there really is no more work.

The solution (`crates/frontier/src/frontier.rs`):

- `pending: AtomicI64` counts URLs that are enqueued but not yet fully processed. `try_push` increments it *before* the item is visible to any worker; `complete` decrements it.
- **Invariant**: a caller must push all of an item's children before calling `complete` on that item. This is what prevents the actual race — if `complete` ran first, a sibling worker could observe `pending == 0` and shut down while this worker was still about to enqueue more work.
- `pop()` uses `tokio::sync::Notify` with the documented safe pattern (create the `Notified` future *before* checking state, not after) — otherwise a push racing between the check and the await would be a lost wakeup, and a worker could block forever even though work exists.
- `stop()` exists as a separate hard-shutdown signal from organic drain, because budget limits (`max_urls`, `max_duration`) create a real scenario organic drain can't handle: one worker decides to stop (limit hit), but `pending` never reaches zero (the items other workers are mid-processing are still "pending", and nothing will ever complete them if those workers also stop without draining). Without `stop()`, that leaves workers permanently blocked in `pop()`. This was caught by a dedicated test (`stop_wakes_blocked_workers_immediately`) simulating exactly that scenario — an item popped and never completed, with three sibling workers blocked waiting.

Priority is pluggable (`PriorityScorer` trait) specifically because the spec that motivated this project calls out "do NOT simply use FIFO" — the default (`DepthPriority`) approximates BFS, but a future crawl could prioritize by sitemap weight, content type, or a user-supplied score without touching the frontier's internals.

## SSRF protection: resolver-based, not hostname-based

A hostname blocklist (reject `localhost`, `127.0.0.1`, etc. as literal strings) does not stop DNS rebinding: an attacker-controlled hostname can resolve to `127.0.0.1` or `169.254.169.254` (cloud metadata) at request time, after any hostname check has already passed. `crates/fetch/src/security.rs` instead implements `reqwest::dns::Resolve`, performs real DNS resolution, then filters the *resolved IPs* against loopback/private/link-local/multicast/unspecified ranges before the connection is made. This is enforced at the transport layer, so it applies to every request `HttpFetcher` makes (including its own robots.txt fetches), not just the top-level crawl target.

This is opt-out, not opt-in: `CrawlConfig::allow_private_networks` (default `false`) exists because legitimate crawls of internal infrastructure or local test fixtures need a way around it — the fetch integration tests themselves set this to `true` to talk to a local `wiremock` server, which is exactly the kind of trusted, deliberate exception the flag is for.

## Same-domain scoping follows redirects, not the literal seed URL

`same_domain` filtering has to compare a discovered link's host against *something*. The naive choice — the host string the user typed on the command line — breaks the moment the seed itself redirects (very common: `www.example.com` → `example.com`). Every link on the real page would then compare against the wrong host and get filtered out as "cross-domain," silently reducing a crawl to a single page.

`apps/mudfish/src/crawl.rs` instead tracks `scope_host` as an `Arc<RwLock<String>>`, initialized to the seed's literal host, and updates it once — when the seed (depth 0) finishes fetching — to the *resolved* (`final_url`) host. This update is race-free without any special synchronization because no depth-1 item can exist in the frontier until the seed has been processed at least once; by the time any other worker reads `scope_host` to filter its own discovered links, the seed's update (if any) has already happened.

This was found by manual testing against `rust-lang.org` (which redirects `www` → apex) after the automated test suite had already gone green on synthetic fixtures — a reminder that the synthetic test suite doesn't automatically cover every real-world HTTP quirk. A regression test (`same_domain_scoping_follows_seed_redirect` in `apps/mudfish/tests/crawl.rs`) now covers it directly.

## Response size limits are enforced by streaming, not `Content-Length`

`Content-Length` is absent for chunked responses and is just a header a server can lie about. `crates/fetch/src/fetcher.rs::read_body_capped` instead reads the body in chunks via `reqwest::Response::chunk()` and aborts as soon as the accumulated size exceeds the configured cap — this is what actually defends against oversized responses and decompression bombs, regardless of what headers claim.

## What's deliberately not abstracted yet

- No `Fetcher` trait exists yet — `HttpFetcher` is a concrete type used directly by the crawl controller. The eventual browser-backed fetcher (Phase 4) will need one, but adding a trait with a single implementation now would be speculative; it's a mechanical refactor once there's a second implementation to abstract over.
- No storage trait/backend exists — `CrawlResult` lives in memory for the process's lifetime. Adding a `Storage` trait before there's more than one backend (in-memory vs. SQLite vs. Postgres) would be guessing at a shape that hasn't been tested against real requirements yet.
