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
}
