use url::Url;

/// Controls how [`normalize_url`] canonicalizes a URL before it is used as a
/// frontier dedup key. Every rule is opt-in and can be disabled, because some
/// sites use query parameters (or even trailing slashes) as part of content
/// identity — normalization must never silently change what a URL means.
#[derive(Debug, Clone)]
pub struct NormalizationOptions {
    pub strip_fragment: bool,
    pub strip_default_ports: bool,
    pub lowercase_host: bool,
    pub sort_query_params: bool,
    pub strip_tracking_params: bool,
    pub trailing_slash: TrailingSlashPolicy,
    pub tracking_param_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailingSlashPolicy {
    Preserve,
    Add,
    Remove,
}

impl Default for NormalizationOptions {
    fn default() -> Self {
        Self {
            strip_fragment: true,
            strip_default_ports: true,
            lowercase_host: true,
            sort_query_params: false,
            strip_tracking_params: true,
            trailing_slash: TrailingSlashPolicy::Preserve,
            tracking_param_names: default_tracking_params(),
        }
    }
}

fn default_tracking_params() -> Vec<String> {
    [
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "gclid",
        "fbclid",
        "msclkid",
        "mc_cid",
        "mc_eid",
        "igshid",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Canonicalizes a URL for dedup/fingerprinting purposes. Does not mutate
/// query-parameter semantics beyond the tracking-param list and never drops
/// query parameters wholesale, since many sites key content on them.
pub fn normalize_url(url: &Url, opts: &NormalizationOptions) -> Url {
    let mut u = url.clone();

    if opts.strip_fragment {
        u.set_fragment(None);
    }

    if opts.lowercase_host {
        if let Some(host) = u.host_str() {
            let lower = host.to_lowercase();
            if lower != host {
                let _ = u.set_host(Some(&lower));
            }
        }
    }

    if opts.strip_default_ports {
        let default_port = match u.scheme() {
            "http" => Some(80),
            "https" => Some(443),
            _ => None,
        };
        if u.port() == default_port {
            let _ = u.set_port(None);
        }
    }

    let path = u.path();
    if path.contains("//") {
        let collapsed = collapse_slashes(path);
        u.set_path(&collapsed);
    }

    apply_trailing_slash_policy(&mut u, opts.trailing_slash);

    if opts.strip_tracking_params || opts.sort_query_params {
        rewrite_query(&mut u, opts);
    }

    u
}

fn apply_trailing_slash_policy(u: &mut Url, policy: TrailingSlashPolicy) {
    match policy {
        TrailingSlashPolicy::Preserve => {}
        TrailingSlashPolicy::Add => {
            if !u.path().ends_with('/') && !has_file_extension(u.path()) {
                let p = format!("{}/", u.path());
                u.set_path(&p);
            }
        }
        TrailingSlashPolicy::Remove => {
            if u.path().len() > 1 && u.path().ends_with('/') {
                let p = u.path().trim_end_matches('/').to_string();
                u.set_path(&p);
            }
        }
    }
}

fn rewrite_query(u: &mut Url, opts: &NormalizationOptions) {
    let mut pairs: Vec<(String, String)> = u
        .query_pairs()
        .into_owned()
        .filter(|(k, _)| {
            if opts.strip_tracking_params {
                !opts
                    .tracking_param_names
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(k))
            } else {
                true
            }
        })
        .collect();

    if opts.sort_query_params {
        pairs.sort_by(|a, b| a.0.cmp(&b.0));
    }

    if pairs.is_empty() {
        u.set_query(None);
    } else {
        let qs: String = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        u.set_query(Some(&qs));
    }
}

fn collapse_slashes(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut last_was_slash = false;
    for c in path.chars() {
        if c == '/' {
            if last_was_slash {
                continue;
            }
            last_was_slash = true;
        } else {
            last_was_slash = false;
        }
        out.push(c);
    }
    out
}

fn has_file_extension(path: &str) -> bool {
    path.rsplit('/')
        .next()
        .map(|seg| seg.contains('.'))
        .unwrap_or(false)
}

/// A stable 64-bit fingerprint of a (normalized) URL, used as the frontier's
/// dedup key. Not cryptographically strong — collisions are astronomically
/// unlikely at crawl scale but not a security boundary.
pub fn fingerprint(url: &Url) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.as_str().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> NormalizationOptions {
        NormalizationOptions::default()
    }

    #[test]
    fn strips_fragment() {
        let u = Url::parse("https://example.com/page#section").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.as_str(), "https://example.com/page");
    }

    #[test]
    fn strips_default_port() {
        let u = Url::parse("https://example.com:443/page").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.as_str(), "https://example.com/page");
    }

    #[test]
    fn keeps_nonstandard_port() {
        let u = Url::parse("https://example.com:8443/page").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.port(), Some(8443));
    }

    #[test]
    fn strips_tracking_params_but_keeps_others() {
        let u = Url::parse("https://example.com/page?id=42&utm_source=x&utm_medium=y").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.as_str(), "https://example.com/page?id=42");
    }

    #[test]
    fn preserves_content_identity_query_params() {
        let u = Url::parse("https://example.com/search?q=rust").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.as_str(), "https://example.com/search?q=rust");
    }

    #[test]
    fn collapses_duplicate_slashes() {
        let u = Url::parse("https://example.com/a//b///c").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.path(), "/a/b/c");
    }

    #[test]
    fn lowercases_host() {
        let u = Url::parse("https://EXAMPLE.com/Page").unwrap();
        let n = normalize_url(&u, &opts());
        assert_eq!(n.host_str(), Some("example.com"));
        assert_eq!(n.path(), "/Page");
    }

    #[test]
    fn sort_query_params_when_enabled() {
        let mut o = opts();
        o.sort_query_params = true;
        let u = Url::parse("https://example.com/page?b=2&a=1").unwrap();
        let n = normalize_url(&u, &o);
        assert_eq!(n.query(), Some("a=1&b=2"));
    }

    #[test]
    fn fingerprint_is_stable_and_normalization_sensitive() {
        let a = Url::parse("https://example.com/page?utm_source=x").unwrap();
        let b = Url::parse("https://example.com/page").unwrap();
        let na = normalize_url(&a, &opts());
        let nb = normalize_url(&b, &opts());
        assert_eq!(fingerprint(&na), fingerprint(&nb));
    }
}
