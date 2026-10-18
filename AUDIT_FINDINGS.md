# Audit findings — 2026-10-19

This file records the repository-specific findings from the portfolio audit
conducted on 2026-10-19. It is documentation only; no code or release artifacts
were changed in this pass unless explicitly noted under "Implementation status".

Sources: the repository's own README/source/workflows, GitHub metadata and
Actions runs, PyPI metadata, and official competitor documentation. Claims are
marked as verified, inferred, or open questions. "Absent" means not found in the
inspected repository/docs, not proof that it cannot exist.

## What it is

Rust-native asynchronous web crawler with Python bindings: deterministic HTTP crawling first, opt-in browser rendering, SSRF-safe defaults, and budget caps.

## Verified strengths

- Real Rust crawler with budget caps, DNS-rebinding-resistant SSRF protection, robots.txt, per-host politeness, HTTP/2/compression/retries, URL normalization, frontier, and opt-in Chrome rendering.
- Python sync/async API; 42 Rust + 4 Python tests; all-platform wheels (macOS/Linux glibc+musl/Windows) — the portfolio's packaging model.
- Honest phase-1 scope; 1.3x Scrapy benchmark committed.

## Verified gaps and user-facing risks

- No auth/session/cookie handling, no structured extraction, no distributed crawling.
- Browser render path is unbenchmarked and, per the README, does not go through MudFish's SSRF guard upstream.
- 1.3x vs Scrapy is a weak differentiator; `main` CI failed on 2026-10-18.
- No MCP/agent tooling.

## Direct competitors

- Scrapy (mature, 64k stars), Crawlee (HTTP + browser, 26k), Firecrawl (managed extraction), Katana.

## Ranked next actions

- P1: auth/session/cookie support + structured extraction (JSON schema/LLM-optional) (L).
- P1: make `main` CI green and gating (S).
- P2: benchmark against Crawlee/Firecrawl and publish results (M).
- P2: MCP server for agent workflows (M).

## Do not build

- Distributed crawling; operationally heavy and no demand evidence.

## Evidence gaps / validation needed

- No modern competitor benchmark; no large-crawl memory/CPU profile; auth/session behavior unexercised.

## Recommended disposition

Invest; it is one of the strongest engineering bases in the portfolio. Packaging is already ahead of the rest.

## Implementation status

No code changes in this pass.
