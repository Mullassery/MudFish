# Mudfish Crawler — Honest Status

Last updated: 2026-09-18. This document exists so future work (by any contributor, human or AI) starts from what's actually true, not from the aspirational full-platform spec that motivated this project. See `README.md` for the user-facing summary.

The original project brief describes a 9-phase, ~20-crate platform (browser rendering, API/GraphQL discovery, website intelligence graphs, MCP agent tooling, distributed crawling). Building all of that before a solid HTTP core exists would violate the project's own stated principle ("deterministic crawling first"), so **only Phase 1 has been built.** Everything below phase 1 in the original brief is intentionally not started.

## Built and verified (automated tests + live manual crawls)

- `crates/core` — domain types (`CrawlConfig`, `Page`, `Link`, `CrawlResult`, `CrawlStats`, etc.), URL normalization with configurable tracking-param stripping, 64-bit URL fingerprinting. 9 unit tests.
- `crates/frontier` — priority-queue frontier with pluggable `PriorityScorer`, URL-level dedup, and a termination protocol correct under concurrent load (tested with real race conditions: drain-based shutdown, hard `stop()` for budget limits, and workers blocked on an item that never completes). 5 unit tests including two multi-threaded concurrency tests.
- `crates/fetch` — `HttpFetcher` (gzip/brotli/deflate/zstd, HTTP/2, redirects, retry-with-backoff, `Retry-After`, streamed size-capped bodies), `RobotsManager` (fetch/cache/TTL, Google's documented status-code handling), `PolitenessManager` (per-host delay + concurrency), and a DNS-resolution-based SSRF guard (`security.rs`). 16 tests (9 unit + 7 integration against a real local HTTP server via `wiremock`).
- `crates/parser` — HTML link extraction (relative-URL resolution, anchor text, non-crawlable scheme filtering), title/meta-description/canonical extraction, malformed-HTML tolerance. 7 unit tests.
- `apps/mudfish` — CLI (`clap`) + async crawl controller wiring the above together with a bounded worker pool, same-domain scoping (correctly follows seed redirects), and summary/JSON/JSONL output. 3 integration tests against a mocked HTTP server.
- Manually verified against live sites: `example.com` (basic crawl), `rust-lang.org` (multi-page same-domain crawl through a `www` → apex redirect, 59 pages fetched, 0 duplicates), and a deliberate SSRF attempt against `127.0.0.1` (correctly blocked at the DNS-resolution layer).

40/40 automated tests pass. `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are both clean.

## Built but not verified

- HTTP/3 is compiled in via `reqwest` feature flags but never exercised against a real HTTP/3 endpoint.
- No load testing: memory/CPU behavior on crawls of thousands of pages is unmeasured. The fetch layer streams response bodies (no full-buffering beyond the size cap), but nothing downstream (frontier, page storage) has been profiled at scale.
- Cookie/session-based authentication flows have not been tested; the fetcher has no explicit cookie-jar wiring beyond what `reqwest` does by default (which is: none, unless enabled).
- `NormalizationOptions::sort_query_params` and the `TrailingSlashPolicy::Add`/`Remove` variants are implemented and unit-tested in isolation but not exercised by the CLI (no flag exposes them yet — the CLI always uses `NormalizationOptions::default()`).

## Not built

Everything from Phase 2 onward in the original project brief:

- Browser rendering / adaptive HTTP→browser escalation (Phase 4).
- API/XHR/GraphQL discovery via network interception (requires the browser layer).
- Structured/schema-based extraction, JSON-LD/schema.org parsing, entity extraction, optional LLM-assisted extraction (Phase 5).
- Website graph construction, page classification, technology fingerprinting.
- Incremental crawling / change detection (ETag, Last-Modified, content fingerprints) (Phase 6).
- REST/gRPC server, OpenAPI, streaming job events (Phase 7).
- MCP server / agent tool surface (Phase 8).
- Distributed frontier, worker pool, queue backend abstraction, PostgreSQL storage (Phase 9).
- Any storage layer at all beyond in-memory `CrawlResult` — there is no SQLite/Postgres/object-storage persistence yet; a crawl's results exist only for the lifetime of the process unless redirected to a file via `--out-file`.
- Python bindings/SDK (explicitly deferred per project decision — see below).

## Known gaps / deliberate simplifications

- **Frontier lacks separate retry/delayed/failed queues.** The original brief calls for these as first-class frontier concepts; this build instead handles retries entirely inside `HttpFetcher::fetch` (in-request backoff loop) and records failures directly into `CrawlResult.errors`. This is simpler and sufficient for a single-process crawl but will need revisiting if/when distributed crawling (Phase 9) is built, since retry/failure state would then need to be shared across workers.
- **Same-domain policy is exact-host, not eTLD+1.** `blog.example.com` and `example.com` are treated as different domains. No public-suffix-list dependency has been added; if subdomain-aware scoping is needed, that's a deliberate future addition, not an oversight.
- **robots.txt-unreachable handling is conservative (deny-all) but coarse.** A robots.txt fetch blocked by the SSRF guard is indistinguishable, in current stats, from a robots.txt fetch that failed for any other reason — both surface as `urls_skipped` rather than a distinct category. Not incorrect, but worth a clearer stat if this becomes confusing in practice.
- **No CI configured yet.** `.github/workflows/` does not exist in this repository. `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test --workspace` have all been run manually and pass, but nothing enforces this on push yet.
- **`Cargo.lock` is committed** (not gitignored) per this org's established convention for any workspace that ships binaries — `apps/mudfish` produces the `mudfish` binary.

## Python bindings

Deferred entirely, by explicit decision at project start. The original ask mentioned a Python wrapper, but Phase 1–9 of the project brief never schedules one, and this build is pure Rust. When revisited, the two live options are a PyO3/maturin native extension (tighter integration, couples Python releases to Rust build toolchain) or a thin Python client against the future REST API (Phase 7) — that decision should wait until there's a REST API to wrap or a concrete perf reason to bind directly.
