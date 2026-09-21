# Security Policy

## Reporting a vulnerability

Please report security issues privately via
[GitHub Security Advisories](https://github.com/Mullassery/MudFish/security/advisories/new)
for this repository, rather than opening a public issue.

This is a single-maintainer, early-stage project (see `ROADMAP_HONEST.md`).
There is no formal SLA on response time — best-effort, but realistically
expect days, not hours.

## What's actually been checked

- **SSRF protection is implemented and tested**: `crates/fetch/src/security.rs`
  implements a custom DNS resolver that filters resolved IPs (not hostnames)
  against loopback/private/link-local/multicast/unspecified ranges, including
  IPv4-mapped IPv6 addresses and cloud metadata endpoints. This is covered by
  unit tests in that file and exercised end-to-end in both the Rust test
  suite and the Python binding's `test_ssrf_guard_blocks_loopback_by_default`.
  It has **not** been reviewed by anyone other than the author, and has not
  had adversarial/fuzz testing.
- **Response size limits are enforced by streaming**, not by trusting
  `Content-Length` (which can be absent or lied about) — see
  `crates/fetch/src/fetcher.rs::read_body_capped`. This mitigates
  straightforward decompression-bomb / oversized-response scenarios but has
  not been specifically fuzzed against compression-bomb payloads.
- **No sandboxing of fetched content.** Mudfish parses HTML with `scraper`/
  `html5ever` and does not execute any script, CSS, or embedded content from
  crawled pages (there is no browser engine in Phase 1). Malformed HTML is
  handled gracefully (tested), but the HTML parsing dependency chain itself
  has not been independently security-audited by this project.

## What has NOT been checked or hardened

- No dependency vulnerability scanning is currently wired into CI (no
  `cargo audit` / `cargo deny` / `pip-audit` step). This is a real gap — see
  `ROADMAP_HONEST.md` for the full technical-debt list.
- No fuzzing (HTML parser, URL normalization, robots.txt parsing).
- Authentication/session/cookie-based crawling is untested — see the main
  README's "What's built but not verified" section.
- The `--allow-private-networks` and `--no-robots` flags exist and do exactly
  what they say (disable SSRF protection / ignore robots.txt); using them
  against infrastructure you don't own or control is your responsibility, not
  something this tool prevents.

## Supported versions

There are no tagged releases yet beyond the `0.1.0` PyPI package for the
Python bindings. Security fixes will land on `main`; there is no LTS branch
or backport policy at this stage.
