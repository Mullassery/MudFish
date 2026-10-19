# Queue status — 2026-10-19

This file records what was shipped, what is blocked/failed, and what is rejected for this repository. It complements `AUDIT_FINDINGS.md`.

## Shipped
- Cookie sessions; CSS-selector field extraction (0.2.5).

## Blocked / failed
- Distributed crawling; browser-path SSRF parity.

## Rejected / deferred
- Distributed crawling now.

## Policy

Any future feature or upgrade must check `AUDIT_FINDINGS.md` and this file before implementation. A blocked item is retried only when its external dependency is available; a rejected item needs new user evidence or a design change to reopen.
