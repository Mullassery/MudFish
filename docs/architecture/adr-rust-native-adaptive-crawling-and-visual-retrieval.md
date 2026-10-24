# ADR — Rust-native Adaptive Extraction (and the deferred Visual Retrieval boundary)

Status: **Accepted** (adaptive extraction increment); the visual-retrieval pieces are
accepted as *boundaries/design intent*, explicitly deferred until a concrete inference
decision is made.

Date: 2026-10-13

Companion: `docs/architecture/reference-capability-gap-analysis.md`

## 1. Context

Mudfish is a Rust-native, chromiumoxide- and reqwest-based crawler whose long-term
roadmap (../internal/ROADMAP_HONEST.md) leads toward structured extraction, website graphs, and
agent tooling. Two reference projects (Scrapling, PixelRAG) were studied to identify
capabilities worth implementing natively:

- **Scrapling** demonstrates resilient *adaptive extraction*: save a structural
  fingerprint of a matched element, then relocate the equivalent element after the DOM
  changes by scoring every candidate against the fingerprint and accepting only
  candidates at/above a threshold. Its caller-facing diagnostics are weak (DEBUG logs
  only) and its ambiguity handling silently returns ties.
- **PixelRAG** demonstrates a full *visual retrieval* pipeline: render pages to
  screenshots, split into non-overlapping geometry-tracked tiles, embed tiles with a
  joint text/image model, and retrieve with ANN while keeping provenance to source URL
  and pixel coordinates.

The gap analysis (companion document) classifies Mudfish's posture per capability.
This ADR records the architecture decisions for the parts Mudfish will build, in what
order, and what Mudfish will explicitly **not** build.

## 2. Decision drivers

1. Mudfish owns every implementation; neither reference project becomes a runtime
   dependency (no vendoring, subprocessing, or sidecar wrapping).
2. Follow existing boundaries: `mudfish-core` (data, no I/O), `frontier`/`fetch`/
   `parser` (independent, no I/O beyond HTTP), `engine` (orchestration), `browser`
   (optional headless Chrome).
3. No new abstraction for its own sake; no silent selector/retrieval fallbacks.
4. New capabilities are optional when they cost resources or pull specialized
   dependencies.
5. Deterministic-and-safe before fast-and-clever. Everything testable offline first.
6. No stealth/identity-rotation capabilities, ever. Responsible crawling only.
7. Persistent-storage and inference-stack decisions are deferred until a concrete
   requirement exists (ARCHITECTURE.md doctrine).

## 3. Verified capability gaps selected for this initiative

| Capability | Decided now? | Rationale |
|---|---|---|
| Adaptive extraction (fingerprint → relocate, confidence, ambiguity, diagnostics) | **Yes — this ADR** | Highest value, fully understood, offline-testable, zero new runtime deps. |
| Latency-adaptive per-domain throttle | Next increment | Modest, in-memory, improves reliability of the existing engine. |
| Screenshot capture + tiling + provenance | Design intent only | Requires capture (browser) + image pipeline; interface/geometry contract is fixed here. |
| Embeddings + ANN retrieval | Deferred | Requires an inference/store decision (see §10). |
| Crawl checkpoints / persistence | Deferred | Requires a storage-backend decision explicitly warned about in ARCHITECTURE.md. |

## 4. Component boundaries & module placement

### 4.1 Adaptive extraction → `mudfish_parser::adaptive`

Lives in `crates/parser` (already depends only on `scraper`, `url`, `mudfish-core`).
No new crates. New public module `adaptive` exporting:

- `ElementFingerprint` — owned snapshot of a target element's identity context:
  `tag`, normalized `attrs`, bounded `text`, `ancestor_tags`, `parent_tag`,
  `parent_attrs`, `parent_text`, `sibling_tags`, `child_tags`, `depth`.
  Built by `fingerprint(element: ElementRef) -> ElementFingerprint`.
- `RelocateOptions` — `threshold` (0–100, default 40), `max_elements` (default 50_000,
  a hard scan cap → `TooLarge`), `max_text_chars` (default 4096), `max_tokens`
  (default 256), `include_children_tag_names` (default true, cheap).
- `relocate(&ElementFingerprint, &Html, &RelocateOptions) -> RelocateReport`.
- `RelocateReport` — `status`, ranked `candidates`, `scanned`, `threshold`,
  `top_confidence`. `status` ∈ {`Found`, `Ambiguous`, `BelowThreshold`, `NotFound`,
  `TooLarge`}.
- `RelocateCandidate` — `index` (document-order scan index), `tag_path`, `confidence`
  (u8 0–100), `signals: SignalBreakdown`.
- `SignalBreakdown` — per-signal scores (tag, text, attrs, key_attrs, path, parent,
  siblings, children) for provenance/explainability.

### 4.2 Later: throttle → `mudfish_fetch::throttle` (design intent)

`AdaptiveThrottler` beside `PolitenessManager` in `crates/fetch`; consumed by
`mudfish_engine`. In-memory EWMA per host. No storage. Config fields on `CrawlConfig`
(`throttle_enabled`, `min_delay`, `max_delay`) — additive with defaults preserving
current behavior.

### 4.3 Later: visual pipeline (design intent, boundaries fixed now)

- Capture: extend `mudfish_browser::BrowserRenderer` with `screenshot(url, opts)`.
- Tiling/metadata: pure module (no browser) operating on decoded images, emitting
  geometry-tracked tiles each carrying `source_url`, `page_height`, `tile_height`,
  `x/y offset`, `width/height`, `content_hash`, `indexing_version`. This provenance
  contract mirrors PixelRAG's manifest/`metadata.npz` design but is a native Rust data
  model.
- Retrieval: **defined interface only**. No embedding or ANN dependency is committed.

## 5. Data flow & lifecycle (adaptive extraction)

```
caller matched element (earlier crawl / unchanged DOM)
        │  fingerprint(ElementRef)                 ── owned, no borrow of doc
        ▼
 ElementFingerprint  (can be serialized/stored by caller later if desired)
        │
        │  relocate(&fp, &new_doc, &opts)   [any later time]
        ▼
 RelocateReport { status, ranked candidates, confidence, signal breakdown }
   Found          → exactly one unambiguous top candidate ≥ threshold
   Ambiguous      → multiple candidates tied at top score ≥ threshold
   BelowThreshold → candidates exist but none reach threshold (no silent substitution)
   NotFound       → document has no elements (or fingerprint trivially unmatched)
   TooLarge       → element count exceeded max_elements (scan aborted, report says so)
```

`relocate` is pure/deterministic: same inputs ⇒ same report. It performs no I/O and
never consults any remote model.

## 6. Scoring design (deterministic, bounded)

Confidence = `round(weighted_mean(signal_scores) * 100)` with fixed weights:

| Signal | Weight | Implementation |
|---|---|---|
| tag | 0.15 | exact equality |
| text | 0.20 | bounded ordered-sequence similarity of lowercase alphanumeric tokens (max 256 tokens each); skipped (neutral) if original text empty, like the reference |
| attrs | 0.20 | 0.5·key-set Jaccard + 0.5·mean value similarity over keys present in both |
| key_attrs | 0.10 | mean value-similarity (token Jaccard) over `class`/`id`/`href`/`src` keys present in the fingerprint; neutral when none are |
| path | 0.15 | ordered-sequence similarity over ancestor tag path |
| parent | 0.10 | tag·(0.4 + 0.3·attr + 0.3·text), neutral when neither has a parent |
| siblings | 0.05 | ordered-sequence similarity over sibling tag names (capped) |
| children | 0.05 | ordered-sequence similarity over child tag names (capped) |

Ordered-sequence similarity uses a linear-memory LCS DP over ≤256-length token vectors
→ ratio `lcs / max(len_a, len_b)`; the LCS is allocation-bounded and cannot blow up on
pathological text. Attribute values are whitespace-normalized and length-capped via
`max_text_chars`.

Guard rails: hard `max_elements` scan cap; bounded text/tokens; no recursion over tree
(it uses flat document-order iteration); no user-controlled allocations beyond the
caps.

## 7. Failure handling & recovery

- Unparseable HTML: `scraper` tolerates malformed documents; `relocate` still returns a
  valid report (`NotFound`/`BelowThreshold`) — never a panic.
- Oversized DOM: returns `TooLarge` with `scanned` capped; caller can raise
  `max_elements` knowingly. Escalation is explicit, never silent.
- Ambiguous top score: reports the tie instead of choosing arbitrarily (the core
  "no silent unrelated substitution" invariant).
- No retries/side effects exist inside `relocate` (pure function) — nothing to recover.

## 8. Security & resource limits

- `relocate` executes no code, opens no sockets, and holds no unsafe. Untrusted HTML
  influences only similarity scores.
- Bounded CPU (element cap + LCS caps) and bounded memory (owned, capped fields).
- `fingerprint` reads untrusted DOM attributes/text with the same caps, so storing a
  fingerprint cannot grow without bound.
- Later visual components (if built) must reuse the engine's existing SSRF posture for
  the browser path and enforce image dimension / decompression-bomb limits on decode.

## 9. Backward compatibility & storage

- **No changes** to `CrawlConfig`, `Page`, `CrawlResult`, CLI, or Python bindings for
  the adaptive-extraction increment (additive `mudfish_parser::adaptive` only).
- No persistence is introduced. `ElementFingerprint` is `serde`-capable-by-default
  (derive is left to the caller) so a future storage layer can cache fingerprints
  without a schema migration forced now.
- The visual tiling metadata model is new data; it introduces no migration since
  nothing persisted it before.
- Deferred storage (checkpoints, extraction cache, embedding index) will be the subject
  of a separate ADR with explicit migration analysis at that time.

## 10. External dependency decisions

| Dependency | Status | Notes |
|---|---|---|
| `scraper` 0.27 | already in `mudfish-parser` | reuse; no new version |
| similarity implementation | none (std) | deliberately avoids unbounded `SequenceMatcher`-style memory |
| `image` crate | deferred, only if visual tiling lands | MIT/Apache-2.0, mainstream; license acceptable |
| embedding model / ANN store | **no decision yet** | Rust ONNX runtime vs. external service is an open, separate decision; until then the visual pipeline stops at the proven methodologies + defined interface |
| `Scrapling` / `PixelRAG` | **never** | no vendoring, subprocess, or sidecar runtime dependence |
| `PyVectorHound` / any other personal repo | **never** | §3.3 of the initiative rules: shared concepts are not dependencies |

## 11. Performance objectives (measured, not assumed)

- Adaptive extraction target: `relocate` on a 2 MB document runs in **≤ 150 ms** in
  `--release`. Bench: `crates/parser/benches/adaptive_relocate.rs` (mirrors the existing
  `parse_html` bench). **Measured on the dev machine** against a synthetic 2,000,232-byte
  / ~13k-card listing page: **92–97 ms** (criterion interval [91.4 ms, 97.4 ms], 10
  samples); the same fixture at 20 KB is ~1.08 ms. (Numbers are machine-dependent; the
  target is the contract, this is the current reading.)
- The first, straightforward implementation was ~292 ms on the same 2 MB fixture and,
  before an O(n²) fix, worse. The cost was dominated by, per candidate: re-walking and
  re-tokenizing a shared parent's subtree, and allocating a `String` per tag name. The
  shipped design (a) caches parent context by node id so a page with thousands of rows
  sharing one parent walks it once, (b) borrows tag names as `&str` from the parsed tree
  instead of allocating, and (c) precomputes fingerprint-side tokens/paths once per scan.
- Memory: bounded by `max_elements` and token caps; per candidate only normalized
  text/attrs (owned) plus bounded token vectors; LCS uses two rolling rows.
- Correctness before throughput: false-positive (silent unrelated match) rate is the
  primary metric, and the `BelowThreshold`/`Ambiguous` semantics are the enforcement.

## 12. Explicit non-goals

- No stealth, identity rotation, proxy rotation, or challenge solving, in any increment.
- No remote-AI dependency for basic selector recovery (never).
- No full ML training/inference stack re-implemented from scratch.
- No vector database adopted merely because PixelRAG uses one.
- No checkpoints/storage in this increment.
- No new CLI flags or Python API surface in this increment (primitives only).
- No claim of end-to-end multimodal retrieval until a real inference path exists, and
  it will be disclosed as such.

## 13. Consequences

Positive:
- Mudfish gains the highest-value missing capability with zero new runtime deps and
  full offline determinism, and stays ahead of the reference on diagnostics/ambiguity.
- The `adaptive` module is a primitive future structured extraction (roadmap Phase 5)
  can build on without re-architecting the parser.
- The ADR fixes provenance/geometry contracts for the visual pipeline so that later
  increments don't need to re-design storage shape.

Risks:
- `mudfish_parser::adaptive` is initially a library primitive with no engine/CLI
  consumer; its value is realized when extraction rules land. Mitigated by strong
  fixtures + docs and by it being the documented next building block.
- Deferred visual/inference decisions could later conflict with the fixed metadata
  contract; the contract is deliberately minimal (geometry + hashes + URL) to
  de-risk that.

## 14. Acceptance criteria (this ADR's committed increment)

1. `mudfish_parser::adaptive` exists with `fingerprint` / `relocate` / report types.
2. Deterministic: identical input ⇒ identical output (asserted in tests).
3. Fixture matrix passes: unchanged / minor structural / attribute-change / nested /
   missing / multiple-tie / unrelated / malformed / oversized.
4. `BelowThreshold`, `Ambiguous`, and `TooLarge` are reported, never silently resolved.
5. No new runtime dependencies; `cargo fmt --check` and
   `cargo clippy --workspace --all-targets --exclude mudfish-python -- -D warnings` clean;
   full workspace test suite (excluding `mudfish-python`) stays green (70 tests baseline).