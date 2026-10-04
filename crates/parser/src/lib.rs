use mudfish_core::{Link, PageMetadata};
use scraper::{Html, Selector};
use url::Url;

#[derive(Debug, Clone, Default)]
pub struct ParsedPage {
    pub metadata: PageMetadata,
    pub links: Vec<Link>,
}

/// Parses an HTML document, resolving every discovered link and the
/// canonical URL against `base_url`. Non-http(s) schemes (`mailto:`,
/// `javascript:`, `tel:`, ...) are dropped — they are not crawlable.
pub fn parse_html(html: &str, base_url: &Url) -> ParsedPage {
    let document = Html::parse_document(html);

    let title = select_text(&document, "title");
    let meta_description = select_meta_content(&document, "description");
    let canonical =
        select_link_href(&document, "canonical").and_then(|href| resolve(base_url, &href));

    let links = extract_links(&document, base_url);

    ParsedPage {
        metadata: PageMetadata {
            title,
            meta_description,
            canonical,
        },
        links,
    }
}

fn select_text(doc: &Html, selector_str: &str) -> Option<String> {
    let selector = Selector::parse(selector_str).ok()?;
    doc.select(&selector).next().and_then(|el| {
        let text: String = el.text().collect();
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn select_meta_content(doc: &Html, name: &str) -> Option<String> {
    let selector = Selector::parse(&format!(r#"meta[name="{name}"]"#)).ok()?;
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.to_string())
}

fn select_link_href(doc: &Html, rel: &str) -> Option<String> {
    let selector = Selector::parse(&format!(r#"link[rel="{rel}"]"#)).ok()?;
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("href"))
        .map(|s| s.to_string())
}

fn extract_links(doc: &Html, base_url: &Url) -> Vec<Link> {
    let selector = match Selector::parse("a[href]") {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    doc.select(&selector)
        .filter_map(|el| {
            let href = el.value().attr("href")?;
            let url = resolve(base_url, href)?;
            if url.scheme() != "http" && url.scheme() != "https" {
                return None;
            }
            let anchor_text = {
                let text: String = el.text().collect();
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            };
            let rel = el.value().attr("rel").map(|s| s.to_string());
            Some(Link {
                url,
                anchor_text,
                rel,
            })
        })
        .collect()
}

fn resolve(base: &Url, href: &str) -> Option<Url> {
    base.join(href).ok()
}

/// Minimum visible body text (non-whitespace characters, `<script>`/
/// `<style>` excluded) below which a page with at least one `<script>`
/// tag is flagged as likely needing JavaScript execution to show its real
/// content.
const JS_DEPENDENT_TEXT_THRESHOLD: usize = 200;

/// Heuristic signal for "this page's static HTML is probably a near-empty
/// shell that JavaScript fills in after load" (a client-rendered SPA, or a
/// page whose tags/content are injected by a tag-management script) --
/// the decision `mudfish_browser::BrowserRenderer` exists to act on.
///
/// This is deliberately a cheap, imperfect heuristic, not a real rendering
/// check: a page can have substantial static text *and* still inject
/// additional tags/content via JS (this heuristic would say "no" there),
/// and a page can have sparse text with no real JS dependency at all
/// (e.g. a mostly-image gallery) and still be flagged "yes". Callers that
/// need certainty should render and compare, not trust this alone --
/// treat it as a cost-saving filter (skip the expensive browser render
/// when it's obviously unnecessary), not a correctness guarantee.
pub fn static_html_looks_js_dependent(html: &str) -> bool {
    let document = Html::parse_document(html);

    let has_script = Selector::parse("script")
        .map(|sel| document.select(&sel).next().is_some())
        .unwrap_or(false);
    if !has_script {
        return false;
    }

    // `ElementRef::text()` walks all descendant text nodes, which includes
    // `<script>`/`<style>` contents (html5ever still represents those as
    // text nodes) -- without subtracting them, a near-empty SPA shell
    // with one large inline bootstrap script would be miscounted as
    // "plenty of text" and never escalated.
    let count_chars = |iter: &mut dyn Iterator<Item = &str>| -> usize {
        iter.flat_map(str::chars)
            .filter(|c| !c.is_whitespace())
            .count()
    };

    let body_text_len = Selector::parse("body")
        .ok()
        .and_then(|sel| document.select(&sel).next())
        .map(|body| count_chars(&mut body.text()))
        .unwrap_or(0);

    let script_style_len = Selector::parse("script, style")
        .ok()
        .map(|sel| {
            document
                .select(&sel)
                .map(|el| count_chars(&mut el.text()))
                .sum::<usize>()
        })
        .unwrap_or(0);

    body_text_len.saturating_sub(script_style_len) < JS_DEPENDENT_TEXT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/blog/post").unwrap()
    }

    const SAMPLE: &str = r#"
        <html>
          <head>
            <title>  My Great Post  </title>
            <meta name="description" content="A post about things.">
            <link rel="canonical" href="https://example.com/blog/post-canonical">
          </head>
          <body>
            <a href="/about">About</a>
            <a href="https://other.com/page">External</a>
            <a href="mailto:hi@example.com">Email us</a>
            <a href="javascript:void(0)">Nope</a>
            <a href="../index.html">  Home  </a>
          </body>
        </html>
    "#;

    #[test]
    fn extracts_title() {
        let page = parse_html(SAMPLE, &base());
        assert_eq!(page.metadata.title.as_deref(), Some("My Great Post"));
    }

    #[test]
    fn extracts_meta_description() {
        let page = parse_html(SAMPLE, &base());
        assert_eq!(
            page.metadata.meta_description.as_deref(),
            Some("A post about things.")
        );
    }

    #[test]
    fn extracts_and_resolves_canonical() {
        let page = parse_html(SAMPLE, &base());
        assert_eq!(
            page.metadata.canonical.as_ref().map(Url::as_str),
            Some("https://example.com/blog/post-canonical")
        );
    }

    #[test]
    fn resolves_relative_links_against_base() {
        let page = parse_html(SAMPLE, &base());
        let hrefs: Vec<String> = page.links.iter().map(|l| l.url.to_string()).collect();
        assert!(hrefs.contains(&"https://example.com/about".to_string()));
        assert!(hrefs.contains(&"https://other.com/page".to_string()));
        assert!(hrefs.contains(&"https://example.com/index.html".to_string()));
    }

    #[test]
    fn drops_non_http_schemes() {
        let page = parse_html(SAMPLE, &base());
        assert!(!page
            .links
            .iter()
            .any(|l| l.url.scheme() == "mailto" || l.url.scheme() == "javascript"));
    }

    #[test]
    fn captures_anchor_text() {
        let page = parse_html(SAMPLE, &base());
        let home = page
            .links
            .iter()
            .find(|l| l.url.path() == "/index.html")
            .unwrap();
        assert_eq!(home.anchor_text.as_deref(), Some("Home"));
    }

    #[test]
    fn handles_malformed_html_without_panicking() {
        let broken = "<html><body><a href='/x'>unterminated<div><p>hi";
        let page = parse_html(broken, &base());
        assert!(!page.links.is_empty());
    }

    #[test]
    fn flags_near_empty_spa_shell_with_script_as_js_dependent() {
        let spa_shell = r#"<html><body>
            <div id="root"></div>
            <script>console.log("bootstrap");</script>
        </body></html>"#;
        assert!(static_html_looks_js_dependent(spa_shell));
    }

    #[test]
    fn does_not_flag_content_rich_page_even_with_a_large_inline_script() {
        let big_script = "x".repeat(5000);
        let content_rich = format!(
            r#"<html><body>
                <article>{}</article>
                <script>var data = "{}";</script>
            </body></html>"#,
            "A real article with plenty of substantive text content. ".repeat(10),
            big_script
        );
        assert!(
            !static_html_looks_js_dependent(&content_rich),
            "a large inline script's source text must not count toward \
             visible body text"
        );
    }

    #[test]
    fn does_not_flag_sparse_page_with_no_script_at_all() {
        let no_script = "<html><body><img src=\"/hero.jpg\"></body></html>";
        assert!(!static_html_looks_js_dependent(no_script));
    }
}
