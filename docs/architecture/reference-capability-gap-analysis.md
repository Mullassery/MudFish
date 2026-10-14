# Reference Capability Gap Analysis — Scrapling & PixelRAG vs. Mudfish

Status: **audit** (Phase Zero/One of the Rust-native adaptive crawling & visual retrieval
initiative). Evidence basis: Mudfish source tree (branch `main`, clean tree, commit
`ee6fb9f`), Scrapling v0.4.15 source/docs, PixelRAG v0.4.0 source/docs. Reference
repositories are research references only — Mudfish does not vendor, depend on, or run
either project.

## 0. How to read this document

Every row records:

- **Reference capability** — what the reference project documents/implements.
- **Reference source** — the file/URL the claim is grounded in.
- **Mudfish today** — concrete module/function that implements the equivalent, or "none".
- **Classification** — `implemented`, `partial`, `missing`, `unsuitable`.
- **Proposed native Rust implementation** — what Mudfish should build (its own design).
- **Impact / deps / perf / security / tests / priority**.

Keys about verification: **V** = verified in the reference source, **D** = documented but
not fully confirmed in source, **U** = could not verify. See §5 for the full claim level.

---

## 1. Scrapling-inspired capabilities

### 1.1 Adaptive element / selector recovery

| Field | Content |
|---|---|
| Reference capability | `Selector.css(..., auto_save=True /* save */, adaptive=True /* relocate */)`. When a saved selection later returns nothing via the exact selector, `relocate()` scans every element, scores each against the stored fingerprint, and returns all elements tied at the top score **provided it is ≥ a `percentage` threshold (default 40)**. |
| Reference source | `scrapling/parser.py:530,676-690,822-929`; docs `parsing/adaptive.md` (V) |
| Mudfish today | None. `mudfish_parser::parse_html` (crates/parser/src/lib.rs:14) extracts links + title/meta/canonical against fixed selectors. There is no saved-fingerprint → relocate mechanism and no user-facing extraction-rule API. |
| Classification | **missing** |
| Proposed native implementation | New `mudfish_parser::adaptive` module: `ElementFingerprint` (tag, normalized attrs, own text, ancestor tag path, parent tag/attrs/text, sibling tag names, child tag names), `fingerprint(ElementRef)`, and `relocate(&Fingerprint, &Html, RelocateOptions) -> RelocateReport` with per-candidate confidence (0–100), a weighted deterministic scorer, an acceptance threshold, and explicit ambiguity/below-threshold/not-found statuses. Bounded element/text caps for pathological inputs. |
| Architectural impact | Contained in `mudfish-parser` (pure, no I/O, already depends on `scraper`/`url`/`mudfish-core`). No engine/CLI/binding changes required to land the primitive. |
| Dependencies | None beyond existing `scraper`; similarity implemented in std. |
| Performance | Bounded by `max_elements` (default 50k) and capped text/token sizes; LCS on ≤256-length token sequences. |
| Security | Untrusted HTML only influences scores, never safety decisions; no code exec, no network. |
| Test strategy | Deterministic fixtures: unchanged DOM, minor structural change, attribute change, nested change, missing target, multiple plausible matches, unrelated DOM, malformed HTML, oversized input. Assert never silently returns a sub-threshold unrelated element. |
| Priority | **P1 — first implementation target** (fully understood, offline-testable, highest value). |

### 1.2 Structural fingerprints & similarity matching

| Field | Content |
|---|---|
| Reference capability | Fingerprint = `element_to_dict` (tag, attributes, text, ancestor-tag path, parent name/attribs/text, sibling tag names, child tag names). Similarity = field-wise `difflib.SequenceMatcher.ratio()` combined into a mean score (`parser.py:822-894`). `find_similar()` uses tag+parent+grandparent XPath then attribute-similarity filtering (`parser.py:1030`). |
| Reference source | `scrapling/core/utils/_utils.py:76-114`; `parser.py:822-929,987,1030` (V) |
| Mudfish today | `mudfish_core::fingerprint` (crates/core/src/normalize.rs:178) is a URL dedup hash — unrelated purpose. No DOM structural fingerprint exists. |
| Classification | **missing** |
| Proposed native implementation | Part of the `adaptive` module (a.k.a. §1.1). Similarity implemented as bounded ordered-sequence LCS ratio + token Jaccard, deliberately deterministic and allocation-bounded (256-token caps), avoiding `SequenceMatcher`'s unbounded O(n·m). |
| Impact / deps / perf / security / tests | As §1.1. |
| Priority | P1 (bundled with §1.1) |

### 1.3 Confidence scoring for recovered elements / explicit ambiguity (Scrapling gap)

| Field | Content |
|---|---|
| Reference capability | Only control is the `percentage` acceptance threshold; tied top-scorers are all returned. No structured per-candidate confidence or explanation returned to the caller (top-5 scores only at DEBUG log level). Scrapling does **not** expose an ambiguous-match flag. |
| Reference source | `parser.py:563-577` (V); docs `parsing/adaptive.md` (V) |
| Mudfish today | None. |
| Classification | **missing (and an opportunity to exceed the reference)** |
| Proposed native implementation | `RelocateReport` carries a `status` enum (`Found`/`Ambiguous`/`BelowThreshold`/`NotFound`/`TooLarge`), a ranked `candidates` list with per-candidate `confidence`, and a per-signal `SignalBreakdown` (tag/text/attrs/key-attr/path/parent/siblings/children) for provenance and diagnostics. Multiple candidates tied at the top score ⇒ `Ambiguous`, never a silently-selected single element. |
| Impact / deps / perf / security / tests | As §1.1. |
| Priority | P1 |

### 1.4 Static HTTP fetching

| Field | Content |
|---|---|
| Reference capability | Static fetcher via `curl_cffi` TLS impersonation (optional; base install is parser-only). |
| Reference source | `fetchers/requests.py`; `docs/fetching/choosing.md` (V) |
| Mudfish today | `mudfish_fetch::HttpFetcher` (gzip/brotli/deflate/zstd, HTTP/2, redirects, streaming size caps, retry+backoff, `Retry-After`) — crates/fetch/src/fetcher.rs. |
| Classification | **implemented** (Mudfish's is deliberate, no identity impersonation, matching its responsible-crawling posture) |
| Priority | none |

### 1.5 Browser-backed fetching & JS-rendered content

| Field | Content |
|---|---|
| Reference capability | `DynamicFetcher` (Playwright) and `StealthyFetcher` (Patchright hardened Chromium, Cloudflare Turnstile solving, canvas noise, etc.). |
| Reference source | `fetchers/chrome.py`, `fetchers/stealth_chrome.py`; docs `fetching/{dynamic,stealthy}.md` (V/D) |
| Mudfish today | `mudfish_browser::BrowserRenderer` (chromiumoxide, headless Chrome, CDP) triggered by `mudfish_parser::static_html_looks_js_dependent`, opt-in via `render_js` — crates/browser/src/renderer.rs, crates/engine/src/lib.rs. SSRF hardening is a documented partial (TOCTOU pre-check; root isn't fully closed). |
| Classification | **partial** (renderer real; stealth/identity-rotation is an explicit non-goal for Mudfish) |
| Proposed native implementation | None required now; future work is closing the DNS-pinning gap (already disclosed in ROADMAP_HONEST.md). |
| Priority | P4+ (deferred, disclosed) |

### 1.6 Crawl frontier management & scheduling

| Field | Content |
|---|---|
| Reference capability | Spider `Scheduler`: heapq priority queue + URL+method+body+session fingerprint dedup; `dont_filter`; snapshot/restore checkpoints. `Engine`: global + per-domain concurrency, `download_delay`. |
| Reference source | `spiders/{scheduler,engine,spider}.py`; docs `spiders/architecture.html` (V) |
| Mudfish today | `mudfish_frontier::Frontier` — BinaryHeap priority queue, pluggable `PriorityScorer`, URL fingerprint dedup, correct drain/stop termination protocol; engine enforces global + per-host concurrency and `request_delay` — crates/frontier/src/frontier.rs, crates/engine/src/lib.rs. |
| Classification | **implemented** (per-host vs compacted `concurrent_requests_per_domain`; no checkpoint persistence — *that* part is missing, see §1.11) |
| Priority | none |

### 1.7 Per-domain concurrency & adaptive throttling

| Field | Content |
|---|---|
| Reference capability | `AutoThrottle`: per-domain delay learned from observed latency; `target_delay = latency / target_concurrency`, EWMA-ish average, clamped; blocks double the delay (`BLOCK_BACKOFF_FACTOR=2.0`) or honor `Retry-After`. |
| Reference source | `spiders/throttle.py:10-97` (V) |
| Mudfish today | Static `PolitenessManager` per-host `request_delay` + `per_host_concurrency`; `DomainHealthTracker` circuit breaker (fixed constants, cooldown doubling). No latency-learned adaptive delay. |
| Classification | **partial** (circuit breaker ≈ crude throttle; no latency-based adaptive inter-request delay) |
| Proposed native implementation | Add a `ThrottleController` to `mudfish-fetch` (or `mudfish-engine`) that learns a per-host delay from observed request/response latency (EWMA) applied on top of the passive politeness delay, with configurable `floor`/`max` and block backoff, metrics exposed for explanation. Modest, bounded, no storage. |
| Impact / deps / perf / security / tests | In-memory only; no new deps; unit tests with controlled latencies (deterministic). |
| Priority | P2 (after §1.1) |

### 1.8 Rate-limit & block detection

| Field | Content |
|---|---|
| Reference capability | `spider.is_blocked(response)` default status set {401,403,407,429,444,500,502,503,504}, overridable; AutoThrottle penalty; Cloudflare Turnstile solving (stealth). |
| Reference source | `spiders/engine.py:241-264`; docs `spiders/proxy-blocking.md` (V) |
| Mudfish today | `mudfish_fetch::challenge::detect_challenge` (status + body-marker heuristic), `status::classify_status`, `health::DomainHealthTracker` — separate, documented, deliberately non-bypassing. |
| Classification | **implemented** (detection/classification only; strictly better posture — Mudfish explicitly never bypasses) |
| Priority | none |

### 1.9 Retry policies, backoff, failure classification

| Field | Content |
|---|---|
| Reference capability | `max_blocked_retries=3`; AutoThrottle ×2 on block or `Retry-After`; binary `is_blocked` classification. |
| Reference source | `spiders/spider.py:86`, `spiders/throttle.py`, docs `spiders/proxy-blocking.md` (V/D) |
| Mudfish today | `HttpFetcher::fetch` retries (429/5xx/connect/timeout) with exponential backoff + jitter + `Retry-After`, `max_retries` bounded (fetcher.rs:65-89). Rich status taxonomy. |
| Classification | **implemented** |
| Priority | none |

### 1.10 Persistent checkpoints & resumable crawling

| Field | Content |
|---|---|
| Reference capability | Spider checkpointing: `crawldir=` periodic atomic pickle of pending requests + seen fingerprints; resume skips `start_requests`; cleaned on success. Adaptive storage: SQLite (WAL), `UNIQUE(url,identifier)`. |
| Reference source | `spiders/checkpoint.py`; `core/storage.py`; docs `spiders/architecture.md` (V) |
| Mudfish today | No persistence at all — `CrawlResult` is in-memory; crawl is one-shot and non-resumable. Deliberately deferred (ARCHITECTURE.md "no storage trait/backend yet"). |
| Classification | **missing** (explicitly deferred by project doctrine) |
| Proposed native implementation | Deferred: requires a storage-layer decision (SQLite vs. none), which ARCHITECTURE.md warns against guessing prematurely. Blocked pending an explicit decision — see ADR §9.1. |
| Impact / deps / perf / security / tests | Would add a storage backend + serialization + migration concerns; multiple consumers (frontier, health, extraction cache). |
| Priority | P4 (deferred; requires architecture decision) |

### 1.11 Link & structured content extraction

| Field | Content |
|---|---|
| Reference capability | `Response` (lxml) `.css/.xpath/.find_by_text/.find_similar/.get_all_text/.re`, `LinkExtractor`, chained selection, structured exporters (`ItemList.to_json/to_jsonl/to_csv/to_xml`). |
| Reference source | `scrapling/parser.py`, `spiders/links.py`; README (V) |
| Mudfish today | `mudfish_parser::parse_html` extracts title/meta-description/canonical + `a[href]` links with resolution + scheme filtering (crates/parser/src/lib.rs:14-93). No XPath/loose selection API, no text extraction beyond link anchors, no exporters beyond the CLI's JSON/JSONL. |
| Classification | **partial** (Mudfish's crawl-focused surface is adequate for its mission; no general-purpose extraction API) |
| Proposed native implementation | The `adaptive` module (§1.1) is the natural primitive; a full selector/export surface is future work tied to roadmap Phase 5 (structured extraction). |
| Priority | P1 (primitives) / P3+ (surface) |

### 1.12 Extraction diagnostics / explainability

| Field | Content |
|---|---|
| Reference capability | DEBUG-only top-5 score logging; **no** caller-facing match rationale. |
| Reference source | `parser.py:560-577` (V) |
| Mudfish today | None. |
| Classification | **missing** (opportunity to exceed reference) |
| Proposed native implementation | `RelocateReport` + `SignalBreakdown` per candidate (§1.3). |
| Priority | P1 |

### 1.13 Memory efficiency & high-throughput processing

| Field | Content |
|---|---|
| Reference capability | Streaming/async fetchers; session tab pooling; recommend cap practices. |
| Reference source | fetchers (V) |
| Mudfish today | Streamed body reads with hard byte caps (fetcher.rs:148-157); `concurrency` semaphore; bytes never fully buffered beyond cap. |
| Classification | **implemented** |
| Priority | none |

---

## 2. PixelRAG-inspired capabilities

### 2.1 Page rendering & screenshot capture

| Field | Content |
|---|---|
| Reference capability | `pixelshot`/`render_url`: real Chrome subprocess speaking raw CDP over websocket, `--headless=new`, one throwaway `--user-data-dir` per worker; waits for `load` + `document.fonts.ready` + eager images + settle (rAF race with timeouts); measures content height via JS; `Page.captureScreenshot` with clip rects; JPEG q85 default; PDF backend (pdf2image, DPI 200) and raw image backend. |
| Reference source | `render/src/pixelrag_render/backends/{cdp,pdf}.py`; `render.py` (V) |
| Mudfish today | `mudfish_browser::BrowserRenderer` renders **HTML only** (returns DOM string + network/cookie capture). No screenshot/image output anywhere. |
| Classification | **missing** |
| Proposed native implementation | Extend `mudfish_browser` with an opt-in screenshot capture mode (`BrowserRenderer::screenshot(url, opts) -> Vec<u8>`/PNG), reusing the existing chromiumoxide session. Must be optional and gated, and go through the same SSRF posture as the current render path. |
| Impact / deps / perf / security / tests | chromiumoxide already supports `Page.captureScreenshot`; adds image buffers (memory) — resource limits required. Testable with a live Chrome + local fixture only (same convention as existing browser test). |
| Priority | P3 |

### 2.2 Screenshot tiling & region layout

| Field | Content |
|---|---|
| Reference capability | Two-level split: full-page → non-overlapping vertical **tiles** (`tile_height=8192px`, `viewport_width=875`) → grid **chunks** (`CHUNK_HEIGHT=1024`, `MIN_CHUNK_HEIGHT=28` tail discard; wider pages split along width). `cdp_overlap.py` is navigation pipelining, **not** patch overlap. Chunks carry `x_offset/y_offset/width/height`. |
| Reference source | `embed/src/pixelrag_embed/chunk.py`; `backends/cdp.py`; paper §3.1 (V) |
| Mudfish today | None (no images at all). |
| Classification | **missing** |
| Proposed native implementation | Pure-`image`/`std` tiling module producing tiles with explicit geometry metadata (`page_height`, `tile_height`, `x/y offset`, `width/height`, `source URL`, `content hash`, `indexing version`). Deterministic, testable on synthetic images — no browser required for the tiler itself. |
| Impact / deps / perf / security / tests | `image` crate available (need license check — MIT/Apache2, fine); decompression-bomb/dimension guards required when decoding. Tiler unit tests on generated images; capture path integration test gated on Chrome. |
| Priority | P3 |

### 2.3 Preservation of page layout & visual context

| Field | Content |
|---|---|
| Reference capability | Whole-page screenshots preserve layout by construction; `truncation_reason()` flags captures whose height probe hit the viewport. |
| Reference source | `page_metrics.py`; `backends/cdp.py` (V) |
| Mudfish today | None. |
| Classification | **missing** (bundles into 2.1/2.2) |
| Priority | P3 |

### 2.4 Text extraction alongside visual representations

| Field | Content |
|---|---|
| Reference capability | Optional `--extract-text`: whole-page `document.body.innerText` → `text.md` beside tiles. **No OCR and no DOM-bounding-box→region association** (verified absence). Trafilatura HTML text (1024-token chunks) used for the paper's text baseline. |
| Reference source | `backends/cdp.py`; `eval/README.md` (V) |
| Mudfish today | `mudfish_parser::parse_html` already extracts document text-derived metadata; a whole-page text dump is trivially achievable (`Html::root_element().text()`) but not emitted. |
| Classification | **partial** (parsing exists; page-text emission absent; region-level text association is not present in the reference either) |
| Proposed native implementation | If visual pipeline lands: emit page-level plain text (whole page) alongside tiles; **do not** invent region-OCR (reference doesn't either). |
| Priority | P3 |

### 2.5 Image-aware indexing & retrieval

| Field | Content |
|---|---|
| Reference capability | Single joint text/image embedding model `Qwen/Qwen3-VL-Embedding-2B` (last-token pool, L2 normalize, dim 2048, instruction-prefix for text queries); FAISS `IndexIVFFlat`/IVFPQ (default) or Qdrant; no reranker. |
| Reference source | `embed_cpu.py`; `embed.py`; `index.py`; `serve/api.py` (V) |
| Mudfish today | No indexing/embedding layer anywhere (crawl results are in-memory; roadmap Phase 5 ships AI extraction). |
| Classification | **missing** |
| Proposed native implementation | This is the genuinely heavy piece: requires a Rust/bindings-compatible embedding+ANN option (e.g. a Rust ONNX runtime + HNSW/FAISS-compatible crate, or an optional external service). Per the brief, implement well-defined interfaces + testable pipeline components and document the inference dependency rather than claim end-to-end multimodal retrieval. Deferred to P4. |
| Impact / deps / perf / security / tests | Heavy: model weights, GPU/CPU inference, vector storage. Needs explicit architecture decision. |
| Priority | P4 (interface + pipeline now; inference dependency documented, not faked) |

### 2.6 Tables, charts, diagrams & visually structured docs

| Field | Content |
|---|---|
| Reference capability | No OCR/table-structure recognition; the VLM reads pixels. Support is *implicit* via image retrieval (paper's NQ-Tables results). |
| Reference source | paper §4; `eval/run_bench.py` (V) |
| Classification | **missing** in Mudfish; **unsuitable** to implement as OCR today (reference itself does not do explicit table/chart extraction) |
| Priority | P4 (via visual retrieval if/when that lands) |

### 2.7 Association of image regions with source URL / page metadata (provenance)

| Field | Content |
|---|---|
| Reference capability | Manifest chain: `tiles.json` (source URL, page height, article_id), `chunks.json` (tile/chunk indices, geometry, tile MD5 hashes), `metadata.npz` embedding rows (article_id, tile_index, chunk_index, y_offsets, tile_heights), `articles.json` (title/url). Hits resolve at query time back to URL + pixel coords. |
| Reference source | `backends/cdp.py`, `chunk.py`, `embed_cpu.py` (V) |
| Mudfish today | None. |
| Classification | **missing** |
| Proposed native implementation | Adopt the manifest/geometry/metadata contract into the Mudfish visual module (§2.2) — tile records carry source URL, geometry, content hash, indexing version natively in the Rust data model. |
| Priority | P3 (with the tiler; the whole-point feature) |

### 2.8 Incremental indexing & cache reuse

| Field | Content |
|---|---|
| Reference capability | Render skip when manifest source matches; chunk skip on existing `chunks.json` + `--force` MD5 compare; global vector dedup on `(article_id, tile, chunk)`; Qdrant `--append`/`--recreate`; on-demand render cache in serve. |
| Reference source | `pipelines.py:_needs_render`, `chunk.py` (V) |
| Mudfish today | No persistence/indexing (N/A). |
| Classification | **missing** (bundles into the P3 visual pipeline: content-hash-based cache keys) |
| Priority | P3 |

### 2.9 Resource-aware processing of large documents

| Field | Content |
|---|---|
| Reference capability | Fixed geometry caps, 28px tail discard, `_MAX_WIDTH=4000` downscale, embed width clamp, truncation flag, configurable PDF DPI, sharding, bounded query images. |
| Reference source | `chunk.py`, `pipelines.py`, `api.py` (V) |
| Mudfish today | N/A (no visual pipeline). The HTTP layer already caps bytes/streaming. |
| Classification | **missing** (design requirement to carry into 2.2) |
| Priority | P3 |

### 2.10 Retrieval quality evaluation & relevance testing

| Field | Content |
|---|---|
| Reference capability | `eval/` harness: NQ, NQ-Tables, SimpleQA, MMSearch, LiveVQA, MoNaCo with LLM judges; retrieval modes `naive|traf|base|lora`. Not OCRBench/DocVQA (verified absence). |
| Reference source | `eval/README.md`, `eval/run_bench.py` (V) |
| Mudfish today | None. |
| Classification | **missing** (required as an eval corpus when/if P4 retrieval lands — the brief mandates measured retrieval claims) |
| Priority | P4 |

---

## 3. Consolidated gap summary

| # | Capability | Classification | Priority | Requires new deps? |
|---|---|---|---|---|
| 1 | Adaptive extraction + fingerprints + confidence/ambiguity + diagnostics | missing | **P1** | no |
| 2 | Latency-adaptive per-domain throttle | partial | P2 | no |
| 3 | Screenshot capture | missing | P3 | no (chromiumoxide exists) |
| 4 | Tiling + geometry/provenance metadata | missing | P3 | `image` (MIT/Apache-2.0) |
| 5 | Page-text emission beside visuals | partial | P3 | no |
| 6 | Incremental cache by content hash | missing | P3 | no (part of 4) |
| 7 | Embeddings + ANN retrieval | missing | P4 | yes — needs decision |
| 8 | Checkpoints / resumable crawl | missing | P4 | yes — storage decision |
| 9 | Retrieval evaluation harness | missing | P4 | test-only |

## 4. Implemented where the reference is not (Mudfish already exceeds)

- **Explainability of recovery** (°1.3/1.12): reference logs scores at DEBUG; Mudfish's design returns a first-class report.
- **Responsible crawling**: Mudfish never impersonates identity or solves anti-bot challenges; Scrapling's StealthyFetcher does (unsuitable for Mudfish's posture).
- **Ambiguity honesty**: reference returns ties without a flag; Mudfish's report marks `Ambiguous` explicitly.
- **SSRF**: Mudfish enforces post-resolution address filtering everywhere on the HTTP path; browser path is a disclosed TOCTOU partial.

## 5. Verification & known limitations

- Scrapling claims all marked **V** verified from v0.4.15 source/docs (readthedocs). **U**: exact session-retry default count; StealthyFetcher anti-bot internals were D (documented), not source-verified; `BrowserFetcher` class does not exist in v0.4.15.
- PixelRAG claims marked **V** verified from v0.4.0 source. **U**: per-chunk embed resume within a GPU shard; hidden dim 2048 reported but read dynamically at runtime.
- Live crawling / network access to the reference repos was available via the research pass; both reports were produced from actual source, not README-only.
- This document does not claim Mudfish is or will be a complete replacement for either project; it records capability equivalences for a native, coherent subset.