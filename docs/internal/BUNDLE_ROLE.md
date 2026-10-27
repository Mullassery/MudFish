# Bundle role — MarTech customer-data loop

This repository is part of the **MarTech customer-data loop** bundle. Portfolio-level rationale,
reuse map, and phases are in [`Mullassery/BUNDLES.md`](../Mullassery/blob/main/BUNDLES.md).

- **Role:** Shared crawling, fetch, robots, SSRF-safe networking
- **Upstream/downstream:** PyTagManager, PrismNote, PyReverseETL
- **Next integration step:** Publish versioned Rust crates and Python wheels; add auth/session support; keep the SSRF guard consistent across browser/static paths.
- **Non-goal:** Build a CDP or activation layer.

## Contract checklist

- [ ] Versioned interface (package/crate/API), not an unpinned git URL.
- [ ] Optional extra or separate service; standalone use remains possible.
- [ ] Shared event/profile/observation contract documented where relevant.
- [ ] Integration test or demo proves the boundary.
- [ ] Competitive context recorded in `AUDIT_FINDINGS.md`.

See `COMPETITIVE_INTELLIGENCE.md` for the required check before roadmap or
feature work on this bundle.
