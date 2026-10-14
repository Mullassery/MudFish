//! Native, deterministic adaptive element recovery.
//!
//! The problem this solves: a caller extracts an element (a price, a title, a
//! table row) using some selector today, and the site's DOM changes shape
//! tomorrow. A brittle selector silently returns nothing — or worse, the
//! wrong element. Mudfish has no saved-fingerprint recovery mechanism today:
//! [`crate::parse_html`] extracts a fixed set of fields and that is all.
//!
//! This module lets a caller snapshot a *structural fingerprint* of an
//! element that mattered, then later locate the equivalent element in a
//! changed document and get back a *ranked, explained* list of candidates
//! rather than a bare element or `None`. It is the native Rust equivalent of
//! the "save a matched element, relocate it after the DOM changes" idea
//! popularized by Scrapling (`ElementFingerprint` / `relocate`), implemented
//! independently and with first-class confidence, ambiguity, and diagnostic
//! reporting that the reference does not expose.
//!
//! Design guarantees:
//!
//! - **Deterministic.** Same inputs produce the same report, always.
//! - **No silent substitution.** If no candidate reaches the configured
//!   [`RelocateOptions::threshold`], the report says so
//!   ([`RelocateStatus::BelowThreshold`]) instead of returning the closest
//!   element as if it were a match. Ties are reported as
//!   [`RelocateStatus::Ambiguous`], never resolved arbitrarily.
//! - **No remote model.** Scoring is a closed-form function of the DOM.
//! - **Bounded.** A hard element-scan cap, capped text/token sizes, and a
//!   capped ancestor/sibling/child window protect against pathological or
//!   oversized documents.
//!
//! ```no_run
//! use mudfish_parser::adaptive::{fingerprint, relocate, RelocateOptions};
//! use scraper::{Html, Selector};
//!
//! let old = Html::parse_document("<ul><li class='price'>$19.99</li></ul>");
//! let sel = Selector::parse(".price").unwrap();
//! let fp = fingerprint(old.select(&sel).next().unwrap());
//!
//! let new = Html::parse_document("<div><ul><li class='amount'>$19.99</li></ul></div>");
//! let report = relocate(&fp, &new, &RelocateOptions::default());
//! ```

use std::collections::{HashMap, HashSet};

use scraper::{ElementRef, Html, Selector};

/// Deepest ancestor window considered when comparing the structural path.
/// Documents deeper than this only have their innermost `MAX_PATH_DEPTH`
/// ancestors compared, which bounds per-candidate cost on pathological
/// nesting while keeping the most position-relevant part of the path.
const MAX_PATH_DEPTH: usize = 32;
/// Most sibling tag names kept on each side of the target. Kept deliberately
/// small: sibling/child signals together carry only 10% of the weight, and
/// ordered-sequence similarity is quadratic in the window size, so a large
/// window spends the whole budget on the least-important signals.
const MAX_SIBLINGS: usize = 32;
/// Most direct-child tag names kept.
const MAX_CHILDREN: usize = 32;
/// Most candidates returned in a report (best-ranked first). Ambiguity at the
/// top is detected over the *whole* scan, not just this window.
const MAX_REPORT_CANDIDATES: usize = 256;
/// Default cap on characters of element text snapshotted into a fingerprint.
const DEFAULT_MAX_TEXT_CHARS: usize = 4096;
/// Default cap on tokens compared for any single text/path signal.
const DEFAULT_MAX_TOKENS: usize = 256;
/// Token cap for individual attribute values (shorter than prose).
const ATTR_VALUE_TOKENS: usize = 64;
/// Attributes given extra weight because they tend to survive redesigns and
/// carry meaning.
const KEY_ATTRS: [&str; 4] = ["class", "id", "href", "src"];

// Fixed signal weights. They sum to 1.0 so confidence is a percentage.
const W_TAG: f64 = 0.15;
const W_TEXT: f64 = 0.20;
const W_ATTRS: f64 = 0.20;
const W_KEY_ATTRS: f64 = 0.10;
const W_PATH: f64 = 0.15;
const W_PARENT: f64 = 0.10;
const W_SIBLINGS: f64 = 0.05;
const W_CHILDREN: f64 = 0.05;

/// An owned structural snapshot of an element and its immediate context.
///
/// All fields are plain owned data (no borrow of the source document), so a
/// fingerprint can be held, compared, or persisted by the caller across
/// documents and across time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementFingerprint {
    /// Lowercase element name, e.g. `"span"`.
    pub tag: String,
    /// Attributes in document order, values whitespace-normalized and capped.
    pub attrs: Vec<(String, String)>,
    /// Whitespace-normalized, length-capped text of the element subtree.
    pub text: String,
    /// Ancestor element names from the document root down to (excluding) the
    /// element, innermost `MAX_PATH_DEPTH` only.
    pub ancestor_tags: Vec<String>,
    /// Parent element name, if any.
    pub parent_tag: Option<String>,
    /// Parent attributes (same normalization as `attrs`).
    pub parent_attrs: Vec<(String, String)>,
    /// Whitespace-normalized, capped text of the parent subtree.
    pub parent_text: String,
    /// Tag names of the element's siblings, in document order.
    pub sibling_tags: Vec<String>,
    /// Tag names of the element's direct children, in document order.
    pub child_tags: Vec<String>,
    /// Count of ancestor elements (same as `ancestor_tags.len()`).
    pub depth: usize,
}

/// Tunables for [`relocate`].
#[derive(Debug, Clone)]
pub struct RelocateOptions {
    /// Minimum confidence (0–100) a candidate must reach to be accepted.
    /// Values above this can still be reported as [`RelocateStatus::Ambiguous`]
    /// when several candidates tie. Default `40`.
    pub threshold: u8,
    /// Hard cap on how many elements are scanned. Exceeding it aborts the
    /// scan and yields [`RelocateStatus::TooLarge`] rather than a partial
    /// ranking. Default `50_000`.
    pub max_elements: usize,
    /// Max characters of element text considered per candidate. Default
    /// `4096`.
    pub max_text_chars: usize,
    /// Max tokens compared for any single sequence signal. Default `256`.
    pub max_tokens: usize,
    /// Whether the direct-children tag-name signal participates. The signal
    /// is cheap; disabling it lowers the weight's contribution. Default
    /// `true`.
    pub include_children_tag_names: bool,
}

impl Default for RelocateOptions {
    fn default() -> Self {
        Self {
            threshold: 40,
            max_elements: 50_000,
            max_text_chars: DEFAULT_MAX_TEXT_CHARS,
            max_tokens: DEFAULT_MAX_TOKENS,
            include_children_tag_names: true,
        }
    }
}

/// Per-signal similarity scores (each in `0.0..=1.0`) behind a candidate's
/// confidence, for provenance and diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub struct SignalBreakdown {
    pub tag: f64,
    pub text: f64,
    pub attrs: f64,
    pub key_attrs: f64,
    pub path: f64,
    pub parent: f64,
    pub siblings: f64,
    pub children: f64,
}

/// One ranked relocation candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct RelocateCandidate {
    /// Zero-based position of the element in document order across the whole
    /// document (stable within a single scan).
    pub index: usize,
    /// The candidate's ancestor tag path with its own tag appended.
    pub tag_path: Vec<String>,
    /// Weighted confidence, 0–100.
    pub confidence: u8,
    /// The individual signal scores that produced `confidence`.
    pub signals: SignalBreakdown,
}

/// How a relocation attempt concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocateStatus {
    /// Exactly one candidate reached the threshold as the top scorer.
    Found,
    /// Several candidates tied at the top score, all at/above the threshold.
    /// The caller must decide; nothing is resolved for them.
    Ambiguous,
    /// Candidates exist but none reached the threshold. The report lists the
    /// closest ones for inspection, but none is presented as a match.
    BelowThreshold,
    /// The document contained no elements to scan.
    NotFound,
    /// The scan hit [`RelocateOptions::max_elements`] and was aborted. No
    /// ranking is reported (a partial one would be misleading).
    TooLarge,
}

/// The result of a [`relocate`] call.
#[derive(Debug, Clone, PartialEq)]
pub struct RelocateReport {
    pub status: RelocateStatus,
    /// Best-ranked candidates (at most `MAX_REPORT_CANDIDATES`). Empty for
    /// [`RelocateStatus::TooLarge`] and [`RelocateStatus::NotFound`].
    pub candidates: Vec<RelocateCandidate>,
    /// Number of elements actually scanned.
    pub scanned: usize,
    /// The threshold that was applied.
    pub threshold: u8,
    /// Highest confidence observed, if any element was scanned.
    pub top_confidence: Option<u8>,
}

/// Snapshots an element into an owned [`ElementFingerprint`] using default
/// caps. Never panics; unreadable/malformed nodes simply yield empty fields.
pub fn fingerprint(element: ElementRef<'_>) -> ElementFingerprint {
    let max_chars = DEFAULT_MAX_TEXT_CHARS;
    let value = element.value();
    let ancestor_tags = collect_ancestor_tags(&element);
    let depth = ancestor_tags.len();

    let (parent_tag, parent_attrs, parent_text) = match element.parent().and_then(ElementRef::wrap)
    {
        Some(parent) => {
            let pv = parent.value();
            (
                Some(pv.name().to_string()),
                collect_attrs(pv.attrs(), max_chars),
                collect_text(&parent, max_chars),
            )
        }
        None => (None, Vec::new(), String::new()),
    };

    ElementFingerprint {
        tag: value.name().to_string(),
        attrs: collect_attrs(value.attrs(), max_chars),
        text: collect_text(&element, max_chars),
        ancestor_tags: to_owned_tags(ancestor_tags),
        parent_tag,
        parent_attrs,
        parent_text,
        sibling_tags: to_owned_tags(collect_sibling_tags(&element)),
        child_tags: to_owned_tags(collect_child_tags(&element)),
        depth,
    }
}

/// Scans `doc` and ranks elements by structural similarity to `fp`.
///
/// The scan is deterministic and pure. It never returns a sub-threshold
/// element as a match: see [`RelocateStatus`] for the explicit outcomes.
pub fn relocate(fp: &ElementFingerprint, doc: &Html, opts: &RelocateOptions) -> RelocateReport {
    let threshold = opts.threshold.min(100);
    let max_elements = opts.max_elements.max(1);
    // `*` is a compile-time-valid selector; parsing cannot fail.
    let selector = Selector::parse("*").expect("* is a valid CSS selector");

    let pre = Precomputed::new(fp, opts);
    // Parent context (tag/attrs/text + tokens) is cached by node id: a listing
    // page has thousands of rows sharing one parent, and re-walking that
    // parent's subtree + re-tokenizing its text for every row is what made a
    // naive scan quadratic. Key type is inferred (`ego_tree::NodeId`) so this
    // module needs no direct dependency on `ego-tree`.
    let mut parent_cache = HashMap::new();

    let mut scanned = 0usize;
    let mut too_large = false;
    let mut reported: Vec<RelocateCandidate> = Vec::new();
    let mut top_confidence: Option<u8> = None;
    let mut tied_at_top = 0usize;

    for (index, element) in doc.select(&selector).enumerate() {
        if scanned >= max_elements {
            too_large = true;
            break;
        }
        scanned += 1;
        let parent: Option<&ParentContext> = match element.parent().and_then(ElementRef::wrap) {
            Some(parent) => {
                let context: &ParentContext =
                    parent_cache.entry(parent.id()).or_insert_with(|| {
                        build_parent_context(&parent, opts.max_text_chars, opts.max_tokens)
                    });
                Some(context)
            }
            None => None,
        };

        let candidate = candidate_context(&element, parent, opts);
        let (confidence, signals) = score(&pre, &candidate, opts);

        match top_confidence {
            None => {
                top_confidence = Some(confidence);
                tied_at_top = 1;
            }
            Some(top) if confidence > top => {
                top_confidence = Some(confidence);
                tied_at_top = 1;
            }
            Some(top) if confidence == top => tied_at_top += 1,
            Some(_) => {}
        }

        insert_candidate(
            &mut reported,
            RelocateCandidate {
                index,
                tag_path: candidate.tag_path.clone(),
                confidence,
                signals,
            },
        );
    }

    let status = if too_large {
        reported.clear();
        RelocateStatus::TooLarge
    } else if scanned == 0 {
        RelocateStatus::NotFound
    } else {
        match top_confidence {
            None => RelocateStatus::NotFound,
            Some(top) if top < threshold => RelocateStatus::BelowThreshold,
            Some(_) if tied_at_top > 1 => RelocateStatus::Ambiguous,
            Some(_) => RelocateStatus::Found,
        }
    };

    RelocateReport {
        status,
        candidates: reported,
        scanned,
        threshold,
        top_confidence,
    }
}

// ---------------------------------------------------------------------------
// Context extraction
// ---------------------------------------------------------------------------

/// Owned parent snapshot, cached once per distinct parent node during a scan.
struct ParentContext {
    tag: String,
    attrs: Vec<(String, String)>,
    text_tokens: Vec<String>,
}

/// Fingerprint-side values precomputed once for the whole scan so they are not
/// re-tokenized/rebuilt for every candidate.
struct Precomputed<'f> {
    tag: &'f str,
    attrs: &'f [(String, String)],
    text_tokens: Vec<String>,
    has_text: bool,
    /// The fingerprint's ancestor tags with its own tag appended.
    own_path: Vec<String>,
    parent_tag: Option<&'f str>,
    parent_attrs: &'f [(String, String)],
    parent_text_tokens: Vec<String>,
    has_parent_text: bool,
    sibling_tags: &'f [String],
    child_tags: &'f [String],
}

impl<'f> Precomputed<'f> {
    fn new(fp: &'f ElementFingerprint, opts: &RelocateOptions) -> Self {
        let mut own_path = fp.ancestor_tags.clone();
        own_path.push(fp.tag.clone());
        Precomputed {
            tag: &fp.tag,
            attrs: &fp.attrs,
            text_tokens: tokenize(&fp.text, opts.max_tokens),
            has_text: !fp.text.is_empty(),
            own_path,
            parent_tag: fp.parent_tag.as_deref(),
            parent_attrs: &fp.parent_attrs,
            parent_text_tokens: tokenize(&fp.parent_text, opts.max_tokens),
            has_parent_text: !fp.parent_text.is_empty(),
            sibling_tags: &fp.sibling_tags,
            child_tags: &fp.child_tags,
        }
    }
}

/// Scan-side view of one candidate element. Tag *names* are borrowed directly
/// from the parsed tree (`&'a str`) so a full scan allocates no `String` per
/// tag name; only text/attrs (which need normalization) are owned. The parent
/// view borrows the per-scan cache (`'p`).
struct CandidateContext<'a, 'p> {
    tag: &'a str,
    attrs: Vec<(String, String)>,
    text_tokens: Vec<String>,
    tag_path: Vec<String>,
    sibling_tags: Vec<&'a str>,
    child_tags: Vec<&'a str>,
    parent: Option<&'p ParentContext>,
}

fn build_parent_context(
    parent: &ElementRef<'_>,
    max_text_chars: usize,
    max_tokens: usize,
) -> ParentContext {
    let value = parent.value();
    let text = collect_text(parent, max_text_chars);
    ParentContext {
        tag: value.name().to_string(),
        attrs: collect_attrs(value.attrs(), max_text_chars),
        text_tokens: tokenize(&text, max_tokens),
    }
}

fn candidate_context<'a, 'p>(
    element: &ElementRef<'a>,
    parent: Option<&'p ParentContext>,
    opts: &RelocateOptions,
) -> CandidateContext<'a, 'p> {
    let tag = element.value().name();

    let ancestor_tags: Vec<&'a str> = collect_ancestor_tags(element);
    let mut tag_path: Vec<String> = ancestor_tags
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    tag_path.push(tag.to_string());

    CandidateContext {
        tag,
        attrs: collect_attrs(element.value().attrs(), opts.max_text_chars),
        text_tokens: tokenize(&collect_text(element, opts.max_text_chars), opts.max_tokens),
        tag_path,
        sibling_tags: collect_sibling_tags(element),
        child_tags: collect_child_tags(element),
        parent,
    }
}

fn collect_attrs<'a>(
    attrs: impl Iterator<Item = (&'a str, &'a str)>,
    max_text_chars: usize,
) -> Vec<(String, String)> {
    attrs
        .map(|(name, value)| {
            (
                name.to_ascii_lowercase(),
                normalize_value(value, max_text_chars),
            )
        })
        .collect()
}

fn collect_text(element: &ElementRef<'_>, max_chars: usize) -> String {
    // `.text()` is a lazy traversal; stopping after `max_chars` characters
    // bounds the work even when the subtree is enormous.
    let mut raw = String::new();
    let mut count = 0usize;
    for segment in element.text() {
        for ch in segment.chars() {
            if count >= max_chars {
                break;
            }
            raw.push(ch);
            count += 1;
        }
        if count >= max_chars {
            break;
        }
    }
    normalize_ws(&raw)
}

fn collect_ancestor_tags<'a>(element: &ElementRef<'a>) -> Vec<&'a str> {
    // `ancestors()` yields nearest-first; taking the first `MAX_PATH_DEPTH`
    // keeps the innermost, position-relevant part of the path without walking
    // the full chain on pathologically deep documents. Reversed to root-first
    // to match `ElementFingerprint::ancestor_tags`.
    let mut tags: Vec<&'a str> = element
        .ancestors()
        .take(MAX_PATH_DEPTH)
        .filter_map(ElementRef::wrap)
        .map(|ancestor| ancestor.value().name())
        .collect();
    tags.reverse();
    tags
}

fn to_owned_tags(tags: Vec<&str>) -> Vec<String> {
    tags.into_iter().map(str::to_string).collect()
}

fn collect_sibling_tags<'a>(element: &ElementRef<'a>) -> Vec<&'a str> {
    // Cap *before* collecting: a card in a 10k-item listing has thousands of
    // siblings, and materializing them all would make a single fingerprint
    // O(document) and the whole scan O(n^2). Keep the nearest local window.
    let mut tags: Vec<&'a str> = element
        .prev_siblings()
        .take(MAX_SIBLINGS)
        .filter_map(ElementRef::wrap)
        .map(|sibling| sibling.value().name())
        .collect();
    tags.reverse(); // document order (nearest-first -> root-first window)
    let remaining = MAX_SIBLINGS.saturating_sub(tags.len());
    tags.extend(
        element
            .next_siblings()
            .take(remaining)
            .filter_map(ElementRef::wrap)
            .map(|sibling| sibling.value().name()),
    );
    tags
}

fn collect_child_tags<'a>(element: &ElementRef<'a>) -> Vec<&'a str> {
    element
        .child_elements()
        .map(|child| child.value().name())
        .take(MAX_CHILDREN)
        .collect()
}

// ---------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------

fn score(
    pre: &Precomputed<'_>,
    candidate: &CandidateContext<'_, '_>,
    opts: &RelocateOptions,
) -> (u8, SignalBreakdown) {
    let tag = if pre.tag == candidate.tag { 1.0 } else { 0.0 };

    let text = if pre.has_text {
        sequence_similarity(&pre.text_tokens, &candidate.text_tokens)
    } else {
        // No text to compare (e.g. an icon/container). Treat as neutral rather
        // than penalizing a candidate that has since gained text.
        1.0
    };

    let attrs = attr_similarity(pre.attrs, &candidate.attrs);
    let key_attrs = key_attr_similarity(pre.attrs, &candidate.attrs);

    let path = sequence_similarity(&pre.own_path, &candidate.tag_path);

    let parent = match (pre.parent_tag, candidate.parent) {
        (None, None) => 1.0,
        (Some(a), Some(p)) => {
            let parent_tag = if a == p.tag { 1.0 } else { 0.0 };
            let parent_attrs = attr_similarity(pre.parent_attrs, &p.attrs);
            let parent_text = if pre.has_parent_text {
                sequence_similarity(&pre.parent_text_tokens, &p.text_tokens)
            } else {
                1.0
            };
            parent_tag * (0.4 + 0.3 * parent_attrs + 0.3 * parent_text)
        }
        _ => 0.0,
    };

    let siblings = sequence_similarity_by(pre.sibling_tags, &candidate.sibling_tags, |a, b| a == b);
    let children = if opts.include_children_tag_names {
        sequence_similarity_by(pre.child_tags, &candidate.child_tags, |a, b| a == b)
    } else {
        1.0
    };

    let raw = tag * W_TAG
        + text * W_TEXT
        + attrs * W_ATTRS
        + key_attrs * W_KEY_ATTRS
        + path * W_PATH
        + parent * W_PARENT
        + siblings * W_SIBLINGS
        + children * W_CHILDREN;

    let confidence = (raw.clamp(0.0, 1.0) * 100.0).round() as u8;
    (
        confidence,
        SignalBreakdown {
            tag,
            text,
            attrs,
            key_attrs,
            path,
            parent,
            siblings,
            children,
        },
    )
}

fn attr_similarity(fp: &[(String, String)], candidate: &[(String, String)]) -> f64 {
    if fp.is_empty() && candidate.is_empty() {
        return 1.0;
    }
    let fp_keys: HashSet<&str> = fp.iter().map(|(k, _)| k.as_str()).collect();
    let cand_keys: HashSet<&str> = candidate.iter().map(|(k, _)| k.as_str()).collect();
    let intersection = fp_keys.intersection(&cand_keys).count() as f64;
    let union = fp_keys.union(&cand_keys).count() as f64;
    let key_score = if union == 0.0 {
        1.0
    } else {
        intersection / union
    };

    let mut value_sum = 0.0;
    let mut value_count = 0usize;
    for (key, fp_value) in fp {
        if let Some((_, cand_value)) = candidate.iter().find(|(ck, _)| ck == key) {
            value_sum += value_similarity(fp_value, cand_value);
            value_count += 1;
        }
    }
    let value_score = if value_count == 0 {
        1.0
    } else {
        value_sum / value_count as f64
    };

    0.5 * key_score + 0.5 * value_score
}

fn key_attr_similarity(fp_attrs: &[(String, String)], cand_attrs: &[(String, String)]) -> f64 {
    let mut sum = 0.0;
    let mut considered = 0usize;
    for key in KEY_ATTRS {
        if let Some(fp_value) = lookup_attr(fp_attrs, key) {
            considered += 1;
            if let Some(cand_value) = lookup_attr(cand_attrs, key) {
                sum += value_similarity(fp_value, cand_value);
            }
        }
    }
    if considered == 0 {
        1.0
    } else {
        sum / considered as f64
    }
}

fn lookup_attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}

fn value_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    jaccard(
        &tokenize(a, ATTR_VALUE_TOKENS),
        &tokenize(b, ATTR_VALUE_TOKENS),
    )
}

// ---------------------------------------------------------------------------
// String / sequence primitives
// ---------------------------------------------------------------------------

fn normalize_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !out.is_empty() {
                pending_space = true;
            }
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(ch);
        }
    }
    out
}

fn normalize_value(s: &str, max_chars: usize) -> String {
    let normalized = normalize_ws(s);
    truncate_chars(&normalized, max_chars)
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => s[..byte_idx].to_string(),
        None => s.to_string(),
    }
}

fn tokenize(s: &str, max_tokens: usize) -> Vec<String> {
    let mut tokens = Vec::new();
    for word in s.split_whitespace() {
        let mut token = String::new();
        for ch in word.chars() {
            if ch.is_alphanumeric() {
                token.extend(ch.to_lowercase());
            }
        }
        if !token.is_empty() {
            tokens.push(token);
            if tokens.len() >= max_tokens {
                break;
            }
        }
    }
    tokens
}

fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: HashSet<&str> = a.iter().map(String::as_str).collect();
    let set_b: HashSet<&str> = b.iter().map(String::as_str).collect();
    let intersection = set_a.intersection(&set_b).count() as f64;
    let union = set_a.union(&set_b).count() as f64;
    if union == 0.0 {
        1.0
    } else {
        intersection / union
    }
}

/// Bounded ordered-sequence similarity: `LCS(a, b) / max(len(a), len(b))`.
///
/// Uses O(min) rolling rows over the (already capped) inputs and is therefore
/// linear in memory. Both empty is a perfect match; one empty is not.
fn sequence_similarity<T: PartialEq>(a: &[T], b: &[T]) -> f64 {
    sequence_similarity_by(a, b, |x, y| x == y)
}

/// As [`sequence_similarity`], but the two sequences may have different element
/// types as long as a comparator says which pairs are equal. Used to compare a
/// fingerprint's owned tag names (`String`) against a candidate's borrowed ones
/// (`&str`) without allocating.
fn sequence_similarity_by<T, U>(a: &[T], b: &[U], eq: impl Fn(&T, &U) -> bool) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let n = a.len();
    let m = b.len();
    let mut previous = vec![0usize; m + 1];
    let mut current = vec![0usize; m + 1];
    for item_a in a {
        current[0] = 0;
        for j in 0..m {
            current[j + 1] = if eq(item_a, &b[j]) {
                previous[j] + 1
            } else {
                previous[j + 1].max(current[j])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let lcs = previous[m];
    lcs as f64 / n.max(m) as f64
}

fn insert_candidate(best: &mut Vec<RelocateCandidate>, candidate: RelocateCandidate) {
    let position = best.iter().position(|existing| {
        existing.confidence < candidate.confidence
            || (existing.confidence == candidate.confidence && existing.index > candidate.index)
    });
    match position {
        Some(index) => best.insert(index, candidate),
        None => best.push(candidate),
    }
    if best.len() > MAX_REPORT_CANDIDATES {
        best.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A realistic single target: `<span class="price">` inside a product card.
    const BASE: &str = r#"
        <html><body>
          <main class="content">
            <article class="post">
              <h1 class="title">Rust Native Crawling</h1>
              <div class="product-meta">
                <span class="price" data-currency="USD">$19.99</span>
                <a class="buy" href="/checkout">Buy now</a>
              </div>
            </article>
          </main>
        </body></html>
    "#;

    fn doc(html: &str) -> Html {
        Html::parse_document(html)
    }

    fn fingerprint_price(html: &str) -> ElementFingerprint {
        let document = doc(html);
        let selector = Selector::parse(".price").unwrap();
        fingerprint(document.select(&selector).next().expect("price element"))
    }

    /// Document-order index of the first element matching `css`.
    fn index_of(document: &Html, css: &str) -> usize {
        let selector = Selector::parse(css).unwrap();
        let target = document.select(&selector).next().expect("target element");
        let all = Selector::parse("*").unwrap();
        document
            .select(&all)
            .position(|element| element == target)
            .expect("target present in scan order")
    }

    fn relocate_into(html: &str, fp: &ElementFingerprint) -> RelocateReport {
        relocate(fp, &doc(html), &RelocateOptions::default())
    }

    #[test]
    fn unchanged_document_is_found_at_the_price_element() {
        let fp = fingerprint_price(BASE);
        let new = doc(BASE);
        let report = relocate(&fp, &new, &RelocateOptions::default());

        assert_eq!(report.status, RelocateStatus::Found);
        assert!(
            report.top_confidence.unwrap() >= 90,
            "identical DOM should score very high, got {:?}",
            report.top_confidence
        );
        assert_eq!(report.candidates[0].index, index_of(&new, ".price"));
    }

    #[test]
    fn minor_structural_change_is_recovered() {
        // The target is now wrapped in an extra container and preceded by an
        // unrelated section (common "someone added a banner" change).
        const CHANGED: &str = r#"
            <html><body>
              <header class="banner"><nav>menu</nav></header>
              <main class="content">
                <section class="wrapper">
                  <article class="post">
                    <h1 class="title">Rust Native Crawling</h1>
                    <div class="product-meta">
                      <span class="price" data-currency="USD">$19.99</span>
                      <a class="buy" href="/checkout">Buy now</a>
                    </div>
                  </article>
                </section>
              </main>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let new = doc(CHANGED);
        let report = relocate(&fp, &new, &RelocateOptions::default());

        assert_eq!(report.status, RelocateStatus::Found, "report: {report:?}");
        assert_eq!(report.candidates[0].index, index_of(&new, ".price"));
    }

    #[test]
    fn attribute_change_is_recovered_via_text_and_structure() {
        // class + data attribute changed entirely; text and location intact.
        const CHANGED: &str = r#"
            <html><body>
              <main class="content">
                <article class="post">
                  <h1 class="title">Rust Native Crawling</h1>
                  <div class="product-meta">
                    <span class="amount" data-old="true">$19.99</span>
                    <a class="buy" href="/checkout">Buy now</a>
                  </div>
                </article>
              </main>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let new = doc(CHANGED);
        let report = relocate(&fp, &new, &RelocateOptions::default());

        assert_eq!(report.status, RelocateStatus::Found, "report: {report:?}");
        assert_eq!(report.candidates[0].index, index_of(&new, ".amount"));
        assert!(
            report.candidates[0].signals.text > 0.9,
            "text signal should dominate when only attrs changed"
        );
    }

    #[test]
    fn nested_element_change_is_recovered() {
        // A `<b>` wrapper is added inside the value; own text is unchanged.
        const CHANGED: &str = r#"
            <html><body>
              <main class="content">
                <article class="post">
                  <h1 class="title">Rust Native Crawling</h1>
                  <div class="product-meta">
                    <span class="price" data-currency="USD"><b>$19.99</b></span>
                    <a class="buy" href="/checkout">Buy now</a>
                  </div>
                </article>
              </main>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let new = doc(CHANGED);
        let report = relocate(&fp, &new, &RelocateOptions::default());

        assert_eq!(report.status, RelocateStatus::Found, "report: {report:?}");
        assert_eq!(report.candidates[0].index, index_of(&new, ".price"));
    }

    #[test]
    fn missing_target_is_never_silently_substituted() {
        const WITHOUT_TARGET: &str = r#"
            <html><body>
              <main class="content">
                <article class="post">
                  <h1 class="title">Rust Native Crawling</h1>
                  <div class="product-meta">
                    <a class="buy" href="/checkout">Buy now</a>
                  </div>
                </article>
              </main>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let report = relocate_into(WITHOUT_TARGET, &fp);

        assert_ne!(report.status, RelocateStatus::Found);
        assert_ne!(report.status, RelocateStatus::Ambiguous);
        assert!(
            report.top_confidence.unwrap() < report.threshold,
            "nothing should reach the threshold once the target is gone: {report:?}"
        );
    }

    #[test]
    fn multiple_identical_matches_are_reported_ambiguous() {
        const DUPLICATED: &str = r#"
            <html><body>
              <main class="content">
                <article class="post">
                  <h1 class="title">Rust Native Crawling</h1>
                  <div class="product-meta">
                    <span class="price" data-currency="USD">$19.99</span>
                    <span class="price" data-currency="USD">$19.99</span>
                    <a class="buy" href="/checkout">Buy now</a>
                  </div>
                </article>
              </main>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let report = relocate_into(DUPLICATED, &fp);

        assert_eq!(
            report.status,
            RelocateStatus::Ambiguous,
            "report: {report:?}"
        );
        let top = report.top_confidence.unwrap();
        let tied = report
            .candidates
            .iter()
            .filter(|candidate| candidate.confidence == top)
            .count();
        assert!(tied >= 2, "expected at least two tied top candidates");
    }

    #[test]
    fn unrelated_document_does_not_match() {
        const UNRELATED: &str = r#"
            <html><body>
              <table><tr><th>Name</th><th>Value</th></tr>
              <tr><td>alpha</td><td>beta</td></tr></table>
              <footer><p>nothing to see here</p></footer>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let report = relocate_into(UNRELATED, &fp);

        assert_eq!(
            report.status,
            RelocateStatus::BelowThreshold,
            "report: {report:?}"
        );
    }

    #[test]
    fn a_similar_decoy_does_not_outrank_the_true_target() {
        // Both are `<span class="price">`, but only one is in the original
        // structural context with the original text.
        const DECOY: &str = r#"
            <html><body>
              <main class="content">
                <article class="post">
                  <h1 class="title">Rust Native Crawling</h1>
                  <div class="product-meta">
                    <span class="price" data-currency="USD">$19.99</span>
                    <a class="buy" href="/checkout">Buy now</a>
                  </div>
                </article>
                <aside class="promo">
                  <span class="price">free shipping</span>
                </aside>
              </main>
            </body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let new = doc(DECOY);
        let report = relocate(&fp, &new, &RelocateOptions::default());

        assert_eq!(report.status, RelocateStatus::Found, "report: {report:?}");
        assert_eq!(
            report.candidates[0].index,
            index_of(&new, ".product-meta .price")
        );
    }

    #[test]
    fn malformed_html_is_handled_without_panicking() {
        let broken_old = "<html><body><div class='price'>$19.99<p><span>unterminated";
        let fp = fingerprint_price(broken_old);
        let broken_new = "<html><body><div class='price'>$19.99<p><span>unterminated<div>";
        let report = relocate_into(broken_new, &fp);
        // The exact outcome depends on how html5ever repairs the tree; the
        // contract is only that we get a valid report and never a panic.
        assert!(matches!(
            report.status,
            RelocateStatus::Found
                | RelocateStatus::Ambiguous
                | RelocateStatus::BelowThreshold
                | RelocateStatus::NotFound
                | RelocateStatus::TooLarge
        ));
    }

    #[test]
    fn oversized_document_aborts_with_too_large() {
        let fp = fingerprint_price(BASE);
        let big: String = {
            let mut body = String::from("<html><body><div class='price'>$19.99</div>");
            for i in 0..20 {
                body.push_str(&format!("<span id='s{i}'>item {i}</span>"));
            }
            body.push_str("</body></html>");
            body
        };
        let options = RelocateOptions {
            max_elements: 3,
            ..RelocateOptions::default()
        };
        let report = relocate(&fp, &doc(&big), &options);

        assert_eq!(report.status, RelocateStatus::TooLarge);
        assert_eq!(report.scanned, 3);
        assert!(report.candidates.is_empty(), "no partial ranking on abort");
    }

    #[test]
    fn relocation_is_deterministic() {
        const CHANGED: &str = r#"
            <html><body><main class="content"><article class="post">
              <div class="product-meta"><span class="amount">$19.99</span>
              <a class="buy" href="/checkout">Buy now</a></div>
            </article></main></body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let options = RelocateOptions::default();
        let first = relocate(&fp, &doc(CHANGED), &options);
        let second = relocate(&fp, &doc(CHANGED), &options);
        assert_eq!(first, second);
    }

    #[test]
    fn threshold_is_respected() {
        // Raising the threshold past the achievable score flips Found to
        // BelowThreshold, proving the gate is real.
        const CHANGED: &str = r#"
            <html><body><main class="content"><article class="post">
              <div class="product-meta"><span class="amount" data-x="y">$19.99</span>
              <a class="buy" href="/checkout">Buy now</a></div>
            </article></main></body></html>
        "#;
        let fp = fingerprint_price(BASE);
        let low = relocate(
            &fp,
            &doc(CHANGED),
            &RelocateOptions {
                threshold: 1,
                ..RelocateOptions::default()
            },
        );
        let high = relocate(
            &fp,
            &doc(CHANGED),
            &RelocateOptions {
                threshold: 100,
                ..RelocateOptions::default()
            },
        );
        assert_eq!(low.status, RelocateStatus::Found);
        assert_eq!(high.status, RelocateStatus::BelowThreshold);
    }

    #[test]
    fn sequence_similarity_bounds() {
        let a = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let b = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        assert_eq!(sequence_similarity(&a, &b), 1.0);
        assert_eq!(sequence_similarity::<String>(&[], &[]), 1.0);
        assert_eq!(sequence_similarity(&a, &[] as &[String]), 0.0);
        let c = vec!["a".to_string(), "x".to_string(), "c".to_string()];
        assert!((sequence_similarity(&a, &c) - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn fingerprint_is_owned_and_captures_context() {
        let fp = fingerprint_price(BASE);
        assert_eq!(fp.tag, "span");
        assert_eq!(fp.parent_tag.as_deref(), Some("div"));
        assert_eq!(fp.text, "$19.99");
        assert!(fp.attrs.iter().any(|(k, v)| k == "class" && v == "price"));
        assert!(fp.ancestor_tags.contains(&"article".to_string()));
        assert_eq!(fp.depth, fp.ancestor_tags.len());
    }
}
