# Mudfish Crawler

[![CI](https://github.com/Mullassery/MudFish/actions/workflows/ci.yml/badge.svg)](https://github.com/Mullassery/MudFish/actions/workflows/ci.yml)

A Rust-native web crawling and intelligence engine — deterministic crawling first, browser rendering only when required, AI reasoning only when useful.

Mudfish is not a clone of any existing crawler or scraping product. It is being built from first principles as a wedge across three ideas that are usually separate products: fast async HTTP crawling, developer-facing structured extraction, and automatic API/website intelligence discovery.

## Getting started

Pick whichever fits your stack — both call the same Rust engine.

**Python** (fastest way to try it):

```bash
pip install mudfish
```

```python
import mudfish

result = mudfish.crawl("https://example.com", depth=2, concurrency=30)
print(result["stats"])
for page in result["pages"]:
    print(page["status_code"], page["url"], page["metadata"]["title"])
```

**Rust CLI** (requires a recent stable Rust toolchain, developed against 1.97):

```bash
git clone https://github.com/Mullassery/MudFish.git
cd MudFish
cargo build --release
./target/release/mudfish crawl https://example.com --depth 2 --output json
```

Either way you get the same result shape back: a crawl ID, every page fetched (status, title, meta description, canonical URL, outbound links), any errors, and aggregate stats (URLs discovered/fetched/skipped, bytes, duration).

## Use cases

- **Quick site audit.** `mudfish crawl https://yoursite.com --depth 3 --output json` and get back every reachable page's status code, title, and outbound links — useful for finding broken links, orphaned pages, or checking what a search-engine-style crawler would actually see.
- **Structured data collection from Python.** `mudfish.crawl(url)` in a notebook or pipeline script, no subprocess/server to manage — the dict comes back ready for `pandas`, a database insert, or a RAG ingestion step.
- **Safe crawling of untrusted/third-party URLs.** Point it at user-submitted or arbitrary external URLs (e.g. a link-preview service, a content-moderation pipeline) without worrying about SSRF against your own internal network — loopback/private/link-local/cloud-metadata addresses are refused by design, verified via actual DNS resolution rather than a hostname blocklist.
- **Bounded, budget-safe crawling.** Every crawl has hard limits (`--max-urls`, `--max-duration-secs`, `--max-response-bytes`) so a misconfigured depth or an unexpectedly large site can't run away with your CPU, bandwidth, or bill.
- **Polite crawling of sites you don't control.** robots.txt is honored by default, with per-host rate limiting and concurrency caps, so you can crawl third-party sites without hammering them or getting IP-banned.

## Problem

Most crawling tools force a choice: fast-but-shallow link crawlers (recon-oriented), or slow browser-based scrapers that render everything by default. Neither gives you a single, safe, resource-bounded engine that does cheap HTTP crawling by default and only escalates to a browser (or an LLM) when the page actually needs it.

## Solution

Mudfish's core principle: **deterministic crawling first, browser rendering only when required, AI reasoning only when useful.** The crawler must be able to run entirely without a browser or an LLM in the loop — those are optional enhancements layered on top of a fast, correct, safety-conscious HTTP crawl engine.

## Current scope: Phase 1 (HTTP crawler MVP)

This repository currently implements **Phase 1 only**: a production-quality asynchronous HTTP crawler, matching the project's own phased build plan (see `ROADMAP_HONEST.md`). Browser rendering, structured/AI extraction, website graphs, and distributed crawling are deliberately not started yet — building those before the HTTP core is solid and benchmarked would violate the project's own first principle.

## CLI reference

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

## Python reference

`mudfish.crawl()` (see [Getting started](#getting-started) for install) takes the same options as the CLI, as keyword arguments: `depth`, `concurrency`, `per_host_concurrency`, `same_domain`, `max_urls`, `max_duration_secs`, `timeout_secs`, `request_delay_ms`, `max_response_bytes`, `respect_robots`, `allow_private_networks`. It blocks until the crawl finishes and returns a plain `dict` — no `asyncio` integration yet, though the GIL is released while it runs so other Python threads keep going. Full option table, return-value shape, and limitations: `crates/python/README.md`.

## What's working now (verified)

Verified by 40 Rust tests (`cargo test --workspace`: 9 core, 9+7 fetch, 5 frontier, 7 parser, 3 CLI integration) plus 4 Python-binding tests (`pytest` in `crates/python`), plus manual crawls against live sites (example.com, rust-lang.org):

- Async HTTP crawling with a bounded worker pool (`tokio`), global + per-host concurrency limits.
- URL frontier: pluggable priority scheduling (depth-based by default, not FIFO), URL-level dedup via normalized fingerprinting, and a correctness-tested termination protocol (workers shut down cleanly on drain, on a hard `stop()` signal for budget limits, or hang-free under concurrent load — see `crates/frontier`'s test suite for the specific race conditions this closes).
- URL normalization: fragment stripping, default-port removal, tracking-parameter stripping (`utm_*`, `gclid`, etc.), duplicate-slash collapsing — configurable, and deliberately does **not** strip arbitrary query parameters, since many sites key content identity on them.
- robots.txt handling (fetch, cache with TTL, per-Google's documented 2xx/4xx/5xx status handling) and per-host politeness (request spacing + concurrency caps).
- HTTP fetch layer: gzip/brotli/deflate/zstd, HTTP/2, redirects, retries with exponential backoff + jitter, `Retry-After` support, and a hard cap on response size enforced by streaming (not trusting `Content-Length`, which is absent or lies for chunked/compressed responses).
- SSRF protection: a custom DNS resolver filters out loopback/private/link-local/multicast-resolved addresses *after* resolution, which is what actually stops DNS-rebinding SSRF (a public hostname resolving to `127.0.0.1` or a cloud metadata IP) — a hostname-string blocklist alone cannot catch that.
- HTML parsing: link extraction (with relative-URL resolution, anchor text, `rel`), title/meta-description/canonical extraction, non-crawlable scheme filtering (`mailto:`, `javascript:`, etc.), and graceful handling of malformed HTML.
- Same-domain scoping that correctly follows the seed's *resolved* host through redirects (e.g. `www.example.com` → `example.com`) rather than the literal host typed on the command line — this was a real bug caught during manual testing against rust-lang.org and is now covered by a regression test.
- JSON/JSONL/human-readable summary output.
- Python bindings (`pip install mudfish`) exposing the same crawl engine as a native extension — verified against a local test server (basic crawl, same-domain link-following, invalid-URL error handling) and manually against live sites including the SSRF guard.

## What's built but not verified

- Behavior against sites requiring authentication, cookies-based sessions, or HTTP/3 has not been exercised (HTTP/3 support is compiled in via `reqwest`'s feature flags but not benchmarked or tested here).
- Large-crawl memory/CPU behavior (thousands of pages) has not been profiled; the architecture streams responses but has not been load-tested at that scale.
- Distributed/multi-worker deployment, browser rendering, structured/LLM extraction, website-graph construction, and MCP/agent tooling are not implemented — see `ROADMAP_HONEST.md`.

## Architecture

See `ARCHITECTURE.md` for the workspace layout and the reasoning behind the key design decisions (frontier termination protocol, SSRF resolver design, same-domain scoping).

## Contributing

See `CONTRIBUTING.md` for build/test setup and what a good pull request looks like. Security issues: see `SECURITY.md` (please don't file those as public issues). Behavior changes are tracked in `CHANGELOG.md`.

## Status

Early-stage, single-maintainer, Phase 1 only (see "Current scope" above and `ROADMAP_HONEST.md` for the full honest breakdown of what's built, what's verified, and what's known-broken or missing). Not "production-ready" in the sense of having been run at scale, load-tested, or independently security-reviewed — see `SECURITY.md` and `ROADMAP_HONEST.md`'s technical debt section for specifics.

## License

Apache-2.0. See `LICENSE`.
