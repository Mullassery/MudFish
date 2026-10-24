# Competitive intelligence in roadmap and upgrade work

**Policy date:** 2026-10-19

Competitive findings are an input to product decisions, not a one-off report.
Every pending roadmap item, new feature, and routine dependency upgrade should
check the repository's [`AUDIT_FINDINGS.md`](AUDIT_FINDINGS.md) before work
starts, and record the decision in the PR or changelog.

## Required check before building

1. **Read `AUDIT_FINDINGS.md`.** Find the relevant competitor row and the
   ranked next actions. If the finding is stale, update it with a dated source.
2. **Name the competitor and the user problem.** A feature is not justified by
   "competitor has it". State the workflow it unlocks, who needs it, and how
   often. Mark unverified demand as a hypothesis.
3. **Prefer reuse over new code.** Check whether an existing Mullassery repo
   already provides the capability (see `BUNDLES.md` in the profile repo and
   the bundle role file in this repo, when present). Typical reuse:
   - crawling/fetching/robots/SSRF → **MudFish**
   - architecture maps / PR diffs → **CodeAtlas**
   - data-quality contracts / validation → **StatGuardian**
   - token counting / cost tables → **PyTokenCalc**
   - dependency graph / advisories / SBOM → **PyDeepMesh**
   - segmentation / CLV / churn → **ClusterAudienceKit**
   - activation / lineage / PII → **PyReverseETL**
   - fleet state sync → **SyncFleet** (only after conflict resolution)
4. **Choose one disposition:** build, integrate, defer, or reject. Write it
   down. A rejection with a reason is a successful outcome.
5. **For routine upgrades**, check whether the upgrade closes a known
   competitor gap or breaks a differentiated capability. Run the tests that
   cover the affected path; do not merge a green build that silently changes a
   user-visible contract.

## PR / changelog checklist

- [ ] Competitor and source URL/date recorded (or "no direct competitor").
- [ ] User impact and frequency stated; unverified demand labelled.
- [ ] Reuse check done against sibling repos/bundles.
- [ ] Failure mode and rollback considered (especially auth, PII, cost).
- [ ] Docs/roadmap status updated honestly.
- [ ] Tests or a manual verification command included.

## Review cadence

- Refresh the competitor row when a release changes the capability.
- Re-check the full `AUDIT_FINDINGS.md` at least quarterly and before a major
  release.
- If a finding is wrong or stale, correct it in the same PR that relies on it.

## Non-goals

- Copying features for parity.
- Adding a dependency just because a competitor uses it.
- Treating star counts or download counts as proof of user need.

## OSS gap analysis — 2026-10-24

**What matters most here:** schema extraction and LLM-ready output. Fetching is solved well enough; the users MudFish loses are RAG and data teams who need structured records and Markdown, plus a browser path that is benchmarked rather than best-effort.

Scope: MudFish's implementable Phase-1 crawler surface compared with Scrapy, Crawlee, Katana, and Crawl4AI — fetching, rendering, extraction, sessions, persistence, and agent surfaces. Only shipped code and documented behavior were scored.

### Comparable OSS projects
| Project | GitHub | License | Last activity | Why it is the comparison |
| --- | --- | --- | --- | --- |
| Scrapy | https://github.com/scrapy/scrapy | BSD-3-Clause | 2.19.0 (2026-09-10) | The mature Python crawling framework MudFish already benchmarks against; Items, sitemaps, robots, JOBDIR, and an MCP-facing remote-control release make it the capability baseline. |
| Crawlee | https://github.com/apify/crawlee | Apache-2.0 | 3.18.2 (2026-09-29); 4.0.0-rc.1 (2026-10-06) | Closest HTTP-plus-browser peer: persistent queues, sessions, proxy rotation, adaptive concurrency, and Playwright/Puppeteer rendering. |
| Katana | https://github.com/projectdiscovery/katana | MIT | v1.8.0 (2026-10-05) | Go endpoint-discovery crawler with standard/headless modes, JS parsing, form/login automation, and network extraction — the security-recon alternative to MudFish. |
| Crawl4AI | https://github.com/unclecode/crawl4ai | Apache-2.0 | v0.9.5 (2026-10-14) | Newer (2024) LLM-oriented crawler that sets the extraction bar: Markdown, CSS/XPath schemas, LLM extraction, deep crawl with resume, and MCP. |

### Capability comparison
| Capability | Ours | Scrapy | Crawlee | Katana | Crawl4AI | Gap |
| --- | --- | --- | --- | --- | --- | --- |
| Core HTTP crawl | Yes | Yes | Yes | Yes | Yes | Parity |
| JS rendering | Partial | No | Yes | Yes | Yes | Behind |
| robots.txt + per-host rate limits | Yes | Yes | Yes | Partial | Partial | Ahead of Katana/Crawl4AI |
| Post-resolution SSRF guard | Yes | No | No | No | Partial | Ahead |
| CSS/XPath schema extraction | Partial | Yes | Partial | Partial | Yes | Behind |
| LLM-ready Markdown output | No | No | No | No | Yes | Behind |
| LLM/JSON-schema extraction | No | No | No | No | Yes | Behind |
| Session/login flows | Partial | Yes | Yes | Yes | Yes | Behind |
| Sitemap ingestion | No | Yes | Yes | Yes | Yes | Behind |
| Network/API discovery | Partial | No | Partial | Yes | Partial | Behind Katana |
| Resumable frontier | Yes | Yes | Yes | Partial | Yes | Parity |
| MCP/agent surface | No | Yes | No | No | Yes | Behind |
| Rust core with native Python bindings | Yes | No | No | No | No | Ahead |

Scrapy's JS cell means core, not the external scrapy-playwright/Splash route. Katana documents rate limits and delay but not robots compliance; its output is endpoint-centric rather than a page/metadata model. Crawl4AI's SSRF Partial reflects Docker-server egress hardening, not an in-library resolver guard. MudFish also has proxy rotation in `CrawlConfig`/CLI, but it is absent from the README feature list.

### Gaps that matter (ranked)
1. **No schema extraction or LLM-ready content output** — MudFish returns title/meta/links plus flat CSS text matches (`crates/parser/src/lib.rs:20`); Scrapy has Items/ItemLoaders, Crawl4AI has nested CSS/XPath schemas and Markdown. This blocks RAG and data-pipeline users who currently leave MudFish after fetching. Effort M for nested CSS/XPath + Markdown, L to add optional LLM extraction.
2. **The browser path is a weaker version, not parity** — rendering is heuristic-triggered, off by default, unbenchmarked, and guarded by a best-effort pre-navigation DNS check rather than pinned resolution (`ROADMAP_HONEST.md`). Crawlee, Katana, and Crawl4AI treat browser rendering, sessions, and stealth as first-class. Hurts JS-shell sites and authenticated apps. Effort L.
3. **Session/auth flows stop at a cookie jar** — `Set-Cookie` persists and default headers work, but there is no form login, CSRF handling, recorded flow, or reusable profile; Katana now ships `-auto-login` and recorded flows, Crawlee has session pools, Crawl4AI persistent contexts. Effort M.
4. **No sitemap ingestion or URL seeding** — link-following is the only discovery path; Scrapy SitemapSpider, Crawlee sitemap helpers, Katana `-kf sitemapxml`, and Crawl4AI `AsyncUrlSeeder` cover complete-site and incremental audits. Effort S.
5. **Captured network events are not classified into APIs** — the renderer records requests/responses, but there is no XHR/REST/GraphQL classification like Katana's `-xhr-extraction`/`-kb-endpoints`; this limits the security/API-discovery use case. Effort M.
6. **No MCP or agent-facing server** — Scrapy 2.19 ships a remote-control extension used by an MCP server; Crawl4AI has MCP in its server. MudFish is CLI/library only. Effort M, and only worth it after extraction exists.

### Where we are differentiated
- **Resolver-level SSRF defense** applies after DNS resolution to every HTTP request, including robots.txt fetches; none of Scrapy, Crawlee, or Katana documents an equivalent target guard, and Crawl4AI's protection is server-level.
- **Deterministic, browser-off default with native Rust + PyO3** gives a no-Node, no-server-hop deployment that none of the four matches; Scrapy needs a Python runtime and plugins, Crawlee needs Node, Katana is a CLI, Crawl4AI is browser-first.
- **Hard URL, wall-clock, and streamed byte budgets** are enforced atomically in one engine, with a SQLite frontier that requeues in-flight leases after a crash.
- **Compliance-oriented anti-bot posture** — challenge detection and a circuit breaker that back off instead of rotating identity or solving CAPTCHAs, unlike Katana's solver route and the stealth framing of Crawlee/Crawl4AI.

### Disposition
- **Build:** nested CSS/XPath schema extraction, Markdown output, sitemap seeding, form-login/session primitives, and API classification over captured network events.
- **Integrate/reuse:** keep `crates/browser` and the `scraper` parser; add Markdown via a focused converter rather than adopting a second crawl stack; expose existing proxy rotation in the README/CLI surface.
- **Defer/reject:** distributed crawling, CAPTCHA solving, fingerprint/stealth evasion, and a hosted API. Revisit MCP after schema extraction ships.

### Sources
- https://github.com/scrapy/scrapy/releases (accessed 2026-10-24)
- https://github.com/apify/crawlee/releases (accessed 2026-10-24)
- https://github.com/projectdiscovery/katana (accessed 2026-10-24)
- https://github.com/unclecode/crawl4ai (accessed 2026-10-24)
- https://github.com/projectdiscovery/katana/blob/main/LICENSE.md (accessed 2026-10-24)
- Repo files inspected: `README.md`, `COMPETITIVE_INTELLIGENCE.md`, `AUDIT_FINDINGS.md`, `ARCHITECTURE.md`, `ROADMAP_HONEST.md`, `Cargo.toml`, `crates/parser/src/lib.rs`, `crates/fetch/src/challenge.rs`, `crates/frontier/src/store.rs`, `crates/core/src/config.rs`, `apps/mudfish/src/cli.rs`
