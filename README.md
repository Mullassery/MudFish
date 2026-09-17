# Mudfish Crawler

A Rust-native web crawling and intelligence engine — deterministic crawling first, browser rendering only when required, AI reasoning only when useful.

Mudfish is not a clone of any existing crawler or scraping product. It is being built from first principles as a wedge across three ideas that are usually separate products: fast async HTTP crawling, developer-facing structured extraction, and automatic API/website intelligence discovery.

## Problem

Most crawling tools force a choice: fast-but-shallow link crawlers (recon-oriented), or slow browser-based scrapers that render everything by default. Neither gives you a single, safe, resource-bounded engine that does cheap HTTP crawling by default and only escalates to a browser (or an LLM) when the page actually needs it.

## Solution

Mudfish's core principle: **deterministic crawling first, browser rendering only when required, AI reasoning only when useful.** The crawler must be able to run entirely without a browser or an LLM in the loop — those are optional enhancements layered on top of a fast, correct, safety-conscious HTTP crawl engine.

## Current scope: Phase 1 (HTTP crawler MVP)

This repository currently implements **Phase 1 only**: a production-quality asynchronous HTTP crawler, matching the project's own phased build plan (see `ROADMAP_HONEST.md`). Browser rendering, structured/AI extraction, website graphs, and distributed crawling are deliberately not started yet — building those before the HTTP core is solid and benchmarked would violate the project's own first principle.

## Use cases (today)

- Crawl a site over plain HTTP, respecting robots.txt and rate limits, and get back structured JSON/JSONL of every page fetched (title, meta description, canonical URL, outbound links).
- Map same-domain link structure up to a depth limit, with hard budgets (max URLs, max duration, max response size) so a crawl can never run away.
- Safely point the crawler at arbitrary third-party URLs without risking SSRF against your own infrastructure (loopback/private/link-local/cloud-metadata addresses are refused by default, verified via actual DNS resolution, not a hostname blocklist).

## Installation

Requires a recent stable Rust toolchain (developed against 1.97).

```bash
git clone https://github.com/Mullassery/MudFish.git
cd MudFish
cargo build --release
./target/release/mudfish crawl https://example.com --depth 2
```

## Usage

```bash
mudfish crawl <URL> [OPTIONS]

Options:
      --depth <DEPTH>                      Max link-following depth [default: 3]
      --concurrency <N>                    Max requests in flight, whole crawl [default: 50]
      --per-host-concurrency <N>           Max requests in flight per host [default: 4]
      --allow-cross-domain                 Follow links off the seed's domain
      --max-urls <N>                       Stop after fetching this many URLs [default: 10000]
      --max-duration-secs <SECS>           Hard wall-clock budget for the crawl
      --timeout-secs <SECS>                Per-request timeout [default: 30]
      --request-delay-ms <MS>              Min delay between requests to the same host
      --max-response-bytes <BYTES>         Per-response size cap [default: 20MB]
      --no-robots                          Ignore robots.txt (only for infra you own)
      --allow-private-networks             Permit loopback/private targets (internal use only)
      --output <summary|json|jsonl>        Output format [default: summary]
      --out-file <PATH>                    Write to a file instead of stdout
```

Example:

```bash
mudfish crawl https://example.com --depth 2 --concurrency 30 --output json > crawl.json
```

## What's working now (verified)

Verified by 40 automated tests (`cargo test --workspace`: 9 core, 9+7 fetch, 5 frontier, 7 parser, 3 CLI integration) plus manual crawls against live sites (example.com, rust-lang.org):

- Async HTTP crawling with a bounded worker pool (`tokio`), global + per-host concurrency limits.
- URL frontier: pluggable priority scheduling (depth-based by default, not FIFO), URL-level dedup via normalized fingerprinting, and a correctness-tested termination protocol (workers shut down cleanly on drain, on a hard `stop()` signal for budget limits, or hang-free under concurrent load — see `crates/frontier`'s test suite for the specific race conditions this closes).
- URL normalization: fragment stripping, default-port removal, tracking-parameter stripping (`utm_*`, `gclid`, etc.), duplicate-slash collapsing — configurable, and deliberately does **not** strip arbitrary query parameters, since many sites key content identity on them.
- robots.txt handling (fetch, cache with TTL, per-Google's documented 2xx/4xx/5xx status handling) and per-host politeness (request spacing + concurrency caps).
- HTTP fetch layer: gzip/brotli/deflate/zstd, HTTP/2, redirects, retries with exponential backoff + jitter, `Retry-After` support, and a hard cap on response size enforced by streaming (not trusting `Content-Length`, which is absent or lies for chunked/compressed responses).
- SSRF protection: a custom DNS resolver filters out loopback/private/link-local/multicast-resolved addresses *after* resolution, which is what actually stops DNS-rebinding SSRF (a public hostname resolving to `127.0.0.1` or a cloud metadata IP) — a hostname-string blocklist alone cannot catch that.
- HTML parsing: link extraction (with relative-URL resolution, anchor text, `rel`), title/meta-description/canonical extraction, non-crawlable scheme filtering (`mailto:`, `javascript:`, etc.), and graceful handling of malformed HTML.
- Same-domain scoping that correctly follows the seed's *resolved* host through redirects (e.g. `www.example.com` → `example.com`) rather than the literal host typed on the command line — this was a real bug caught during manual testing against rust-lang.org and is now covered by a regression test.
- JSON/JSONL/human-readable summary output.

## What's built but not verified

- Behavior against sites requiring authentication, cookies-based sessions, or HTTP/3 has not been exercised (HTTP/3 support is compiled in via `reqwest`'s feature flags but not benchmarked or tested here).
- Large-crawl memory/CPU behavior (thousands of pages) has not been profiled; the architecture streams responses but has not been load-tested at that scale.
- Distributed/multi-worker deployment, browser rendering, structured/LLM extraction, website-graph construction, and MCP/agent tooling are not implemented — see `ROADMAP_HONEST.md`.

## Architecture

See `ARCHITECTURE.md` for the workspace layout and the reasoning behind the key design decisions (frontier termination protocol, SSRF resolver design, same-domain scoping).

## License

Apache-2.0. See `LICENSE`.
