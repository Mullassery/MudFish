# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).
This project does not yet follow Semantic Versioning strictly (see the note
under `[0.1.0]` below) — treat `0.x` as pre-1.0 and potentially
breaking at any point.

## [Unreleased]

### Added
- OSS project scaffolding: `CONTRIBUTING.md`, `SECURITY.md`,
  `CODE_OF_CONDUCT.md`, this changelog, GitHub issue/PR templates, and a
  `.github/workflows/ci.yml` running `cargo fmt`/`clippy`/`test` plus a
  separate `maturin`/`pytest` job for the Python bindings. None of this
  existed before — `ROADMAP_HONEST.md` previously stated "no CI configured
  yet," and that was true until now.
- `.github/dependabot.yml` for Cargo, pip (`crates/python`), and GitHub
  Actions dependency updates.

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
