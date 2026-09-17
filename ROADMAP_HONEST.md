# Mudfish Crawler — Honest Status

Last updated: 2026-09-18 (Python bindings added same day, after initial Phase 1 commit). This document exists so future work (by any contributor, human or AI) starts from what's actually true, not from the aspirational full-platform spec that motivated this project. See `README.md` for the user-facing summary.

The original project brief describes a 9-phase, ~20-crate platform (browser rendering, API/GraphQL discovery, website intelligence graphs, MCP agent tooling, distributed crawling). Building all of that before a solid HTTP core exists would violate the project's own stated principle ("deterministic crawling first"), so **only Phase 1 has been built.** Everything below phase 1 in the original brief is intentionally not started.

## Built and verified (automated tests + live manual crawls)

- `crates/core` — domain types (`CrawlConfig`, `Page`, `Link`, `CrawlResult`, `CrawlStats`, etc.), URL normalization with configurable tracking-param stripping, 64-bit URL fingerprinting. 9 unit tests.
- `crates/frontier` — priority-queue frontier with pluggable `PriorityScorer`, URL-level dedup, and a termination protocol correct under concurrent load (tested with real race conditions: drain-based shutdown, hard `stop()` for budget limits, and workers blocked on an item that never completes). 5 unit tests including two multi-threaded concurrency tests.
- `crates/fetch` — `HttpFetcher` (gzip/brotli/deflate/zstd, HTTP/2, redirects, retry-with-backoff, `Retry-After`, streamed size-capped bodies), `RobotsManager` (fetch/cache/TTL, Google's documented status-code handling), `PolitenessManager` (per-host delay + concurrency), and a DNS-resolution-based SSRF guard (`security.rs`). 16 tests (9 unit + 7 integration against a real local HTTP server via `wiremock`).
- `crates/parser` — HTML link extraction (relative-URL resolution, anchor text, non-crawlable scheme filtering), title/meta-description/canonical extraction, malformed-HTML tolerance. 7 unit tests.
- `crates/engine` — the async crawl orchestration (worker pool, frontier/fetch/parser wiring, same-domain scoping) extracted from the CLI app so it's shared, unchanged, by both the CLI and the Python bindings. No dedicated tests of its own (covered transitively by both consumers' test suites); the extraction was verified safe by all 40 pre-existing tests continuing to pass with zero assertion changes.
- `apps/mudfish` — CLI (`clap`): translates `CrawlArgs` into a `CrawlConfig`, calls `mudfish_engine::crawl`, formats summary/JSON/JSONL output. 3 integration tests against a mocked HTTP server.
- `crates/python` — PyO3 native extension (`pip install mudfish`) exposing `mudfish.crawl()`, built via `maturin` (mixed layout: `python/mudfish/__init__.py` wraps the native `_mudfish` module). Converts `CrawlResult` to a Python dict via `pythonize`. 4 pytest tests (basic crawl, same-domain link-following, invalid-URL → `ValueError`, SSRF guard blocks loopback) against a local `http.server` fixture — offline and fast, since crawl *correctness* is already covered by the Rust engine's own tests; these only verify the PyO3 boundary.
- Manually verified against live sites (both from the CLI and from Python): `example.com` (basic crawl), `rust-lang.org` (multi-page same-domain crawl through a `www` → apex redirect, 59 pages fetched via CLI / 17 via Python, 0 duplicates), and a deliberate SSRF attempt against `127.0.0.1` (correctly blocked at the DNS-resolution layer, from both entry points).

40 Rust tests + 4 Python tests pass. `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are both clean (the `mudfish-python` crate cannot be linked by plain `cargo build`/`test` — `extension-module` deliberately omits libpython at link time, which is normal for PyO3 extensions and is why `maturin develop`/`build` exist; `cargo clippy -p mudfish-python` still runs clean since clippy doesn't need the final link step).

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

## Known gaps / deliberate simplifications

- **Frontier lacks separate retry/delayed/failed queues.** The original brief calls for these as first-class frontier concepts; this build instead handles retries entirely inside `HttpFetcher::fetch` (in-request backoff loop) and records failures directly into `CrawlResult.errors`. This is simpler and sufficient for a single-process crawl but will need revisiting if/when distributed crawling (Phase 9) is built, since retry/failure state would then need to be shared across workers.
- **Same-domain policy is exact-host, not eTLD+1.** `blog.example.com` and `example.com` are treated as different domains. No public-suffix-list dependency has been added; if subdomain-aware scoping is needed, that's a deliberate future addition, not an oversight.
- **robots.txt-unreachable handling is conservative (deny-all) but coarse.** A robots.txt fetch blocked by the SSRF guard is indistinguishable, in current stats, from a robots.txt fetch that failed for any other reason — both surface as `urls_skipped` rather than a distinct category. Not incorrect, but worth a clearer stat if this becomes confusing in practice.
- **No CI configured yet.** `.github/workflows/` does not exist in this repository. `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test --workspace` have all been run manually and pass, but nothing enforces this on push yet.
- **`Cargo.lock` is committed** (not gitignored) per this org's established convention for any workspace that ships binaries — `apps/mudfish` produces the `mudfish` binary.

## Python bindings

Originally deferred entirely at project start (Phase 1–9 of the project brief never schedules Python, and the initial build was pure Rust) — reversed the same day by explicit user request, once there was a stable Rust engine worth wrapping. Built as a PyO3/maturin native extension (`crates/python`, `pip install mudfish`), since a thin REST-client wrapper isn't viable yet (Phase 7's server doesn't exist). See "Built and verified" above for what's tested, and `crates/python/README.md` for the limitations:

- **Synchronous only.** `crawl()` blocks; there's no `asyncio` integration. Each call spins up its own single-use Tokio runtime internally.
- **Single-platform wheels.** Built and published from the maintainer's machine (macOS ARM64), not a cross-platform CI matrix (`cibuildwheel`/`maturin-action`). Other platforms currently need a Rust toolchain to build from source via `pip install` triggering a source build, which will only work if `maturin` + a Rust toolchain are available in that environment.
- **No native `.pyi` type stub** for the underlying `_mudfish` extension module — the public `mudfish.crawl()` wrapper function has full type hints, which covers the actual public API, but IDEs won't see hints if someone reaches into `mudfish._mudfish` directly (not a supported entry point anyway).
- Revisit a REST-client-based Python package once Phase 7 (REST/gRPC server) exists — that would decouple Python releases from the Rust build toolchain entirely, at the cost of a network hop.
