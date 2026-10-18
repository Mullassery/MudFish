# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
This project does not yet follow Semantic Versioning strictly (see the note
under `[0.1.0]` below) — treat `0.x` as pre-1.0 and potentially
breaking at any point.

## [Unreleased]

### Added
- HTTP fetcher now enables a per-crawler cookie store, so `Set-Cookie` values
  from a login/session response are sent on subsequent requests. Manual
  `Cookie`/`Authorization` headers still work via `default_headers`; no login
  form/CSRF automation is added. Python package `0.2.4`, Rust workspace `0.1.4`.

### Changed
- Rust workspace `0.1.2` → `0.1.3`, tagged `rust-v0.1.3`, so downstream
  repositories can pin crate dependencies to a release tag instead of a raw
  commit SHA. `BrowserRenderer::launch` now takes `chrome_path: Option<&Path>`
  (breaking for Rust callers on ≤0.1.2). Aligned `crates/python/Cargo.toml`
  version (was lagging at `0.2.1`) with `pyproject.toml`/`__version__`
  (`0.2.3`), and documented the two consumption paths in the README.

### Security
- Bumped `rustls` 0.23.45 → 0.23.46 (RUSTSEC-2026-0346/0347/0348/0349).

### Added
- `CrawlConfig::proxies` / `proxy_rotate_after`: an ordered proxy pool the
  fetcher rotates through (one `reqwest::Client` per proxy, round-robin every
  N requests, with immediate rotation on a retryable failure). The SSRF
  resolver hook and redirect policy are applied to every client, so proxy
  rotation can't be used to bypass the guard. Exposed on the CLI as
  repeatable `--proxy` plus `--proxy-rotate-after`, and in the Python
  `CrawlOptions` as `proxies`/`proxy_rotate_after`. (RECHECK-022)
- Network-idle settle for `BrowserRenderer::render`: the post-navigation wait
  now returns as soon as no request/response has been observed for a short
  quiet window, instead of always blocking for the full fixed `settle`
  duration, while still using `settle` as the ceiling for pages that never go
  idle. (RECHECK-021)
- `HttpFetcher::current_proxy_index` (observability/tests).
- **Bundled Chrome.** `tools/fetch_chrome_headless_shell.py` downloads a
  pinned `chrome-headless-shell` (Chrome for Testing) into
  `crates/python/python/mudfish/_chrome/` (gitignored); the Python wheel then
  ships a working `render_js` binary with no host install. `CrawlConfig::chrome_path`
  (CLI `--chrome-path`, Python `CrawlOptions.chrome_path`) selects the binary,
  `mudfish.bundled_chrome_path()` / `MUDFISH_CHROME` expose it, and
  `BrowserRenderer::launch` restores the executable bit if the wheel dropped
  it. The bundled wheel is distributed via GitHub Releases (it exceeds PyPI's
  per-file size cap); the PyPI wheel stays small and uses host Chrome.
- `CrawlConfig::default_headers`: extra headers (e.g. `Cookie`/`Authorization`)
  sent with every request, for crawling pages that require auth.
  `HttpFetcher::new` now validates and attaches them via
  `reqwest::ClientBuilder::default_headers`; an invalid header name/value
  fails fetcher construction with `FetchError::InvalidHeader` instead of
  the fetcher being built in a broken state.
- `crates/browser` (`mudfish-browser`): a real headless-Chrome (CDP, via
  `chromiumoxide`) `BrowserRenderer` for pages that need JavaScript
  execution. Launches once, reused across pages; returns rendered HTML,
  captured network requests (`Network.requestWillBeSent`/
  `responseReceived`), and cookies.
- `mudfish_parser::static_html_looks_js_dependent`: a heuristic for
  deciding whether a static HTTP response is probably a near-empty
  client-rendered shell, as an adaptive-escalation signal for when to use
  `BrowserRenderer` instead of the plain HTTP fetcher.
- `mudfish_fetch::health::DomainHealthTracker`: a per-host circuit
  breaker (opens after repeated 429/403/5xx, cools down with exponential
  backoff, half-opens for one trial request). Wired into
  `mudfish_engine::crawl`'s worker loop.
- `mudfish_fetch::challenge::detect_challenge` /
  `mudfish_fetch::status::{CrawlStatus, classify_status}`: heuristic
  anti-bot-challenge detection and a structured
  `Success`/`Throttled`/`Blocked`/`Challenged`/`Failed` status, for
  reporting only -- never an attempt to bypass or solve a challenge.

### Fixed
- **`--max-urls` was not a hard cap under concurrency.** Workers raced a
  load-then-branch check of `stats.fetched` against `max_urls` before
  fetching; with N workers racing concurrently, all N could observe the
  count still under budget and proceed before any incremented it,
  overshooting the limit by up to `concurrency` (real repro: `--max-urls 5
  --concurrency 10` against a fan-out site fetched 14 pages instead of 5).
  Fixed by reserving each fetch slot atomically via `compare_exchange_weak`
  immediately before the fetch, releasing the reservation on fetch failure
  so it doesn't permanently consume budget a later successful fetch could
  use. New regression test:
  `apps/mudfish/tests/crawl.rs::max_urls_is_a_hard_cap_under_concurrency`.

## [0.1.1] - 2026-09-22

### Added
- OSS project scaffolding: `CONTRIBUTING.md`, `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, this changelog, GitHub issue/PR templates, and a
  `.github/workflows/ci.yml` running `cargo fmt`/`clippy`/`test` plus a
  separate `maturin`/`pytest` job for the Python bindings. None of this
  existed before — `ROADMAP_HONEST.md` previously stated "no CI configured
  yet," and that was true until now.
- `.github/dependabot.yml` for Cargo, pip (`crates/python`), and GitHub
  Actions dependency updates.
- `rust-toolchain.toml` pinning `channel = "1.97"` (matching the version the
  README already claimed) plus `rustfmt`/`clippy` components, so
  contributors and CI resolve the same toolchain automatically.

### Fixed
- **Mutex/RwLock poisoning no longer cascades across the worker pool.**
  `crates/frontier/src/frontier.rs` (the internal queue `Mutex`) and
  `crates/engine/src/lib.rs` (the `scope_host` `RwLock`, now wrapped in
  `update_scope_host`/`read_scope_host` helpers) recover from a poisoned
  lock via `.unwrap_or_else(|poisoned| poisoned.into_inner())` instead of
  panicking again on `.unwrap()`/`.expect(...)`. Previously, one worker
  panicking while holding either lock would poison it and then panic every
  other worker on their next access, taking down the whole crawl instead of
  failing just the one item. Verified with two new tests that deliberately
  poison each lock from a panicking thread and assert subsequent operations
  still succeed.

## [0.1.0] - 2026-09-18

Note: this version number was published to PyPI (`pip install mudfish`) for
the Python bindings, but this repository has no corresponding git tag —
there is no `v0.1.0` tag on `main`. That's a real process gap, not
intentional; a future release should tag the commit it's built from.

### Added
- Phase 1 HTTP crawler MVP: async crawl engine (`crates/engine`) built on
  a URL frontier (`crates/frontier`), HTTP fetch layer (`crates/fetch`),
  and HTML parser (`crates/parser`), all built on shared domain types
  (`crates/core`).
- Rust CLI (`apps/mudfish`, binary name `mudfish`) with `crawl` subcommand:
  configurable depth, concurrency (global and per-host), URL/duration/byte
  budgets, robots.txt handling, and summary/JSON/JSONL output.
- SSRF protection via a DNS-resolution-based guard (`crates/fetch/src/security.rs`)
  that filters resolved IPs, not hostnames, against loopback/private/
  link-local/multicast ranges — closes the DNS-rebinding gap a hostname
  blocklist would miss.
- Python bindings (`crates/python`, PyO3 + `maturin`, published as `mudfish`
  on PyPI): `mudfish.crawl()` exposing the same engine as a synchronous,
  blocking native extension.
- 40 Rust tests and 4 Python tests, all passing as of this writing (see
  `README.md` and `ROADMAP_HONEST.md` for exactly what each covers).

### Known limitations (see `ROADMAP_HONEST.md` for the full list)
- No CI was configured at the time of this release (fixed under
  `[Unreleased]` above).
- No load testing at scale; no HTTP/3, cookie/session, or eTLD+1 subdomain
  support; no storage layer; no browser rendering or extraction — Phase 1
  only, by design.
