# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
This project does not yet follow Semantic Versioning strictly (see the note
under `[0.1.0]` below) — treat `0.x` as pre-1.0 and potentially
breaking at any point.

## [Unreleased]

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
