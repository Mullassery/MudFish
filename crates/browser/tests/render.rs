//! Live integration test for `BrowserRenderer` against a real headless
//! Chrome instance (launched via `chromiumoxide`) and a local mock HTTP
//! server. Requires a Chrome/Chromium binary discoverable on the host --
//! skipped (not `#[ignore]`d, just left to fail loudly) if none is found,
//! since this crate's whole point is driving a real browser, not a mock
//! of one.

use std::time::Duration;

use mudfish_browser::BrowserRenderer;
use url::Url;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn renders_js_injected_content_and_captures_the_fetch_it_fires() {
    let server = MockServer::start().await;

    // The initial HTML is an empty shell -- a naive static-HTML crawl
    // would see none of the real content. The page's own JS injects a
    // marker div and fires a `fetch()` to a second route, simulating what
    // a client-rendered SPA (or an analytics tag loaded via JS) actually
    // does.
    // `set_body_string` hard-codes `Content-Type: text/plain`, which would
    // make Chrome display this as raw text instead of parsing/executing
    // it -- `set_body_raw` with an explicit mime type is required for the
    // browser to actually treat this as HTML.
    Mock::given(method("GET"))
        .and(path("/spa"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"<html><body>
                <div id="root"></div>
                <script>
                    document.getElementById('root').innerHTML =
                        '<p id="injected">rendered by JS</p>';
                    fetch('/beacon');
                </script>
            </body></html>"#,
            "text/html",
        ))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/beacon"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let renderer = BrowserRenderer::launch()
        .await
        .expect("a Chrome/Chromium binary must be discoverable on this host");

    let url = Url::parse(&format!("{}/spa", server.uri())).unwrap();
    let rendered = renderer
        .render(&url, Duration::from_millis(800))
        .await
        .expect("render should succeed");

    assert!(
        rendered.html.contains("rendered by JS"),
        "expected JS-injected content in rendered HTML, got: {}",
        rendered.html
    );

    let beacon_url = format!("{}/beacon", server.uri());
    assert!(
        rendered
            .network_requests
            .iter()
            .any(|r| r.url == beacon_url),
        "expected the fetch('/beacon') call to be captured as a network \
         request, got: {:?}",
        rendered.network_requests
    );
    let beacon = rendered
        .network_requests
        .iter()
        .find(|r| r.url == beacon_url)
        .unwrap();
    assert_eq!(beacon.status, Some(204));
}
