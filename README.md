# MudFish

A Rust-native web crawling and intelligence engine — deterministic crawling first, browser rendering only when required, AI reasoning only when useful.

[![CI](https://github.com/Mullassery/MudFish/actions/workflows/ci.yml/badge.svg)](https://github.com/Mullassery/MudFish/actions/workflows/ci.yml)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![mudfish on PyPI](https://img.shields.io/pypi/v/mudfish.svg?label=mudfish)](https://pypi.org/project/mudfish/)

## What it is

Mudfish is a fast, safety-conscious asynchronous web crawler built in Rust and exposed through both a CLI and native Python bindings (via PyO3 — no subprocess or server hop). Its core principle: run entirely without a browser or an LLM by default; browser rendering (headless Chrome) is an opt-in escalation path for JavaScript-dependent pages. The engine supports bounded budgets, post-resolution SSRF protection, robots.txt and per-host politeness, URL normalization, and streaming response-cap enforcement. It is Phase 1 of a larger crawling/intelligence platform — the current scope is a production-quality HTTP crawler plus opt-in rendering, not the full vision.

## Features / Highlights

- **Bounded, budget-safe crawling** — hard caps on URLs fetched, wall-clock duration, and per-response bytes, enforced atomically under concurrency.
- **SSRF protection where it counts** — a custom DNS resolver filters resolved addresses (loopback/private/link-local/metadata) *after* resolution, defeating DNS-rebinding that a hostname blocklist can't catch.
- **Polite by default** — robots.txt honored with Google-documented 2xx/4xx/5xx handling and cached TTL, per-host request spacing, and per-host concurrency caps.
- **Modern HTTP** — gzip/brotli/deflate/zstd, HTTP/2, redirects, retries with exponential backoff + jitter, `Retry-After` support, and body caps enforced by streaming.
- **Correct URL handling** — fragment/default-port stripping, tracking-parameter removal (`utm_*`, `gclid`, …), duplicate-slash collapsing, and same-domain scoping that follows the seed's *resolved* host through redirects.
- **Pluggable frontier priority** — depth-based BFS-like scheduling by default (not FIFO), URL-dedup via normalized fingerprinting, and a correctness-tested termination protocol (drain, hard stop, and race-free shutdown).
- **Optional browser rendering** — `--render-js` re-renders JS-dependent pages in a real headless Chrome and parses the post-JS DOM (off by default; needs Chrome on the host).
- **Native Python surface** — `mudfish.crawl()` (blocking, GIL released) and `crawl_async()`/`Crawler` over the same Rust engine, returning typed `CrawlResult`/`Page`/`CrawlStats` objects that remain dict-readable.

## Who it's for / Use cases

- **Quick site audits** — `mudfish crawl <url> --depth 3 --output json` returns every reachable page's status, title, and outbound links for broken/orphaned-link checks.
- **Structured data collection in Python** — crawl into a dict/crawl result straight from a notebook or pipeline, ready for pandas, a database insert, or RAG ingestion.
- **Safe crawling of untrusted URLs** — link-preview and content-moderation pipelines can hand it arbitrary user-supplied URLs without SSRF risk against internal networks.
- **Budget-bound crawl jobs** — hard `--max-urls`/`--max-duration-secs`/`--max-response-bytes` so nothing runs away with CPU, bandwidth, or your bill.

## What it is not

- **Not a browser-first scraper.** JS rendering is opt-in and off by default; it has not been benchmarked and requires a Chrome/Chromium binary on the host.
- **Not the full platform yet.** Structured/AI extraction, website graphs, distributed crawling, and MCP/agent tooling are **not implemented** (that's later-phase work — the roadmap deliberately refuses to build them before the HTTP core is solid).
- **Session support is manual.** `default_headers` can carry `Cookie`/`Authorization`, and the HTTP fetcher now persists `Set-Cookie` values across requests in one crawler instance. There is no login-form automation, CSRF handling, or cookie import/export; HTTP/3 is compiled in but untested.
- **Not load-tested at scale.** Large-crawl memory/CPU behavior (thousands of pages) has not been profiled.
- **Early-stage and single-maintainer.** Phase 1 only; not "production-ready" in the sense of at-scale operation, load testing, or independent security review. Distribution is from the maintainer's machine (single-platform wheels — check PyPI for platform coverage; others build from source).

## Project status

`mudfish` 0.2.3 on PyPI, Rust workspace `0.1.3` (git tag `rust-v0.1.3` for downstream consumers). Phase 1 (HTTP crawler + opt-in browser rendering) is built and verified by 42 Rust tests plus 4 Python-binding tests, and manually validated against live sites (example.com, rust-lang.org). In a head-to-head benchmark against Scrapy 2.13.4 on the same live site it was ~1.3x faster at equal scope. See [ROADMAP_HONEST.md](ROADMAP_HONEST.md) for the full honest breakdown of built/verified/broken.

## Installation

**Python** (fastest way to try it):

```bash
pip install mudfish
```

**Rust CLI** (requires a recent stable Rust toolchain):

```bash
git clone https://github.com/Mullassery/MudFish.git
cd MudFish
cargo build --release
./target/release/mudfish crawl https://example.com --depth 2 --output json
```

## Quick start

```python
import mudfish

result = mudfish.crawl("https://example.com", depth=2, concurrency=30)
print(result.stats.urls_fetched, result.stats.duration_secs)
for page in result:                       # CrawlResult is iterable over its pages
    print(page.status_code, page.url, page.title)
```

Or from the CLI:

```bash
mudfish crawl https://example.com --depth 2 --concurrency 30 --output json > crawl.json
```

Either way you get the same result shape: a crawl ID, every page fetched (status, title, meta description, canonical URL, outbound links), any errors, and aggregate stats (URLs discovered/fetched/skipped, bytes, duration).

## Usage

### CLI

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
      --render-js                          Re-render JS-dependent pages in headless Chrome (needs Chrome; slow)
      --render-settle-ms <MS>              Settle time before capturing a rendered page [default: 2000]
      --output <summary|json|jsonl>        Output format [default: summary]
      --out-file <PATH>                    Write to a file instead of stdout
```

### Python

`mudfish.crawl()` takes the same options as keyword arguments (`depth`, `concurrency`, `per_host_concurrency`, `same_domain`, `max_urls`, `max_duration_secs`, `timeout_secs`, `request_delay_ms`, `max_response_bytes`, `respect_robots`, `allow_private_networks`, `render_js`, `render_settle_ms`). It blocks until the crawl finishes (or a budget is hit) and returns a typed `CrawlResult`; the GIL is released while it runs. `mudfish.crawl_async()` (plus the reusable `mudfish.Crawler`) is the `asyncio`-friendly surface over the same engine. Full object reference and limitations: `crates/python/README.md`.

## Using MudFish from another repository

MudFish is consumed two ways. Both are supported and exercised by a real
downstream user (`PyTagManager`).

**Python** — depend on the wheel and call the same engine the CLI uses:

```bash
pip install mudfish
```

```python
import mudfish
result = mudfish.crawl("https://example.com", depth=1, max_urls=50)
```

**Rust** — depend on the workspace crates by tag (not by raw commit SHA), so
upgrades are deliberate and revertible. The crates are not on crates.io; pin
the tag that matches the workspace version:

```toml
mudfish-core     = { git = "https://github.com/Mullassery/MudFish", tag = "rust-v0.1.3" }
mudfish-frontier = { git = "https://github.com/Mullassery/MudFish", tag = "rust-v0.1.3" }
mudfish-fetch    = { git = "https://github.com/Mullassery/MudFish", tag = "rust-v0.1.3" }
mudfish-parser   = { git = "https://github.com/Mullassery/MudFish", tag = "rust-v0.1.3" }
mudfish-engine   = { git = "https://github.com/Mullassery/MudFish", tag = "rust-v0.1.3" }
mudfish-browser  = { git = "https://github.com/Mullassery/MudFish", tag = "rust-v0.1.3" }
```

Tag naming: `vX.Y.Z` tags track the **Python** package; `rust-vX.Y.Z` tags
track the **Rust workspace** (they version independently). Pin `rust-v*` for
crate dependencies.

Note on upgrades: `rust-v0.1.3` changed `BrowserRenderer::launch` to take a
`chrome_path: Option<&Path>` argument (bundled-Chrome support). Callers moving
from ≤0.1.2 must pass `launch(None)` to keep using host Chrome.

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — workspace layout and the reasoning behind the key design decisions (frontier termination, SSRF resolver, same-domain scoping)
- [crates/python/README.md](crates/python/README.md) — full Python API reference, object table, and limitations
- [ROADMAP_HONEST.md](ROADMAP_HONEST.md) — status buckets, what's verified, what's known-broken or missing
- [CONTRIBUTING.md](CONTRIBUTING.md) — build/test setup and what a good PR looks like
- [SECURITY.md](SECURITY.md) — security posture and how to report a vulnerability

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Security issues: see [SECURITY.md](SECURITY.md) — please don't file those as public issues. Behavior changes are tracked in [CHANGELOG.md](CHANGELOG.md).

## Related projects

- [PrismNote](https://github.com/Mullassery/prismnote) — Jupyter-compatible data-science notebook; a natural destination for crawl results headed into pandas or RAG
- [PyReverseETL](https://github.com/Mullassery/PyReverseETL) — quality-validated data activation with audit trail and lineage, for moving collected data where it's needed
- [PyStreamMCP](https://github.com/Mullassery/PyStreamMCP) — intelligence layer for AI agents: query planning, context discovery, cost optimization
- [PyAPICheck](https://github.com/Mullassery/PyAPICheck) — transparent API security discovery with real-traffic cross-referencing, a sibling security/observation tool
- [MessageBirds](https://github.com/Mullassery/MessageBirds) — open-source, event-driven Customer Data & Engagement Platform (Rust + TypeScript)

[Browse all projects →](https://github.com/Mullassery?tab=repositories)

## License

Apache-2.0. See [LICENSE](LICENSE).