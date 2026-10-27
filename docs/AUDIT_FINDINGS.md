# Audit Report: MudFish

## Overview
- **Type**: Rust web crawler with PyO3 bindings
- **Languages**: Rust (workspace), Python bindings
- **Version**: Rust 0.1.3, PyPI mudfish 0.2.3
- **License**: Apache-2.0
- **Purpose**: Deterministic HTTP crawler, opt-in browser rendering, SSRF protection

## Architecture
- **Workspace**: crates/apps structure (see Cargo.toml)
- **Core**: Bounded crawling, DNS rebinding protection, robots.txt, politeness (rate limiting), URL normalization, streaming caps
- **Bindings**: Native Python via PyO3
- **Rendering**: Optional headless Chrome

## Entry Points
- CLI: `mudfish crawl ...`
- Python: `mudfish.crawl()`, `crawl_async()`, `Crawler` class

## Dependencies
- Rust async (tokio implied), HTTP client, DNS resolution, robots parsing
- PyO3 for bindings
- Chrome/Chromium for JS rendering (optional)

## Security Observations
- SSRF protection with post-resolution DNS filtering (good - defeats rebinding)
- Robots.txt respected
- Body caps, timeouts, budgets enforced
- No obvious credential leakage

## Tests/CI
- 100+ Rust tests + 4 Python binding tests
- CI badge present

## Findings

| Severity | Finding | Evidence | Impact | Root Cause Hypothesis | Recommendation |
|----------|---------|----------|--------|----------------------|---------------|
| Medium | HTTP/3 compiled in but untested | README states | Potential bugs/unexpected behavior | Future-proofing | Test or disable until verified |
| Medium | Session support manual (no login automation) | README states | Limited for auth-required sites | Out of scope for Phase 1 | Document clearly |
| Low | Not load-tested at scale | README states | Unknown perf under heavy load | Early phase | Plan load testing |
| Low | Single-maintainer, early stage | README states | Maintenance risk | Project stage | Set expectations |

## Strengths
- Strong security focus (SSRF protection)
- Well-designed budgets/caps
- Polite crawling by default
- Native Python bindings (no subprocess)
- Honest about limitations
- Benchmarked vs Scrapy

## Limitations
- Phase 1 only, many features planned but not implemented
- JS rendering requires external Chrome
- No distributed crawling

## Prioritized Recommendations
1. **Medium**: Test HTTP/3 or gate it
2. **Medium**: Add security tests for SSRF cases
3. **Low**: Document platform coverage for wheels
