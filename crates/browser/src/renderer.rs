use std::sync::{Arc, Mutex};
use std::time::Duration;

use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams, EventRequestWillBeSent, EventResponseReceived,
};
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use url::Url;

use crate::error::BrowserError;

/// One request/response pair observed during a browser render, keyed by
/// URL (CDP's `requestId` would be a more precise join key, but URL is
/// sufficient for this crate's purpose: handing network activity to
/// PyTagManager's MarTech vendor-matching, which already matches on URL).
#[derive(Debug, Clone)]
pub struct NetworkRequestRecord {
    pub url: String,
    pub method: String,
    pub resource_type: String,
    pub status: Option<i64>,
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CookieRecord {
    pub name: String,
    pub value: String,
    pub domain: String,
}

#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub url: Url,
    pub final_url: Url,
    pub html: String,
    pub network_requests: Vec<NetworkRequestRecord>,
    pub cookies: Vec<CookieRecord>,
}

/// Wraps a single headless Chrome instance (launched once, reused across
/// `render` calls) for pages that need JavaScript execution to produce
/// their real content -- `mudfish_fetch::HttpFetcher` never executes
/// JavaScript, so a client-rendered SPA looks empty to it. This is the
/// escalation target `mudfish_parser::static_html_looks_js_dependent`
/// exists to decide when to use (see that function's doc comment for the
/// heuristic and its disclosed limits).
pub struct BrowserRenderer {
    browser: Browser,
    _handler_task: tokio::task::JoinHandle<()>,
}

impl BrowserRenderer {
    /// Launches a new headless Chrome instance. Requires a Chrome/Chromium
    /// binary discoverable on the host (chromiumoxide's default autodetect
    /// path, e.g. `/Applications/Google Chrome.app` on macOS, or
    /// `$CHROME` if set) -- this is a real, heavier-weight resource than
    /// `HttpFetcher`, so callers should launch one `BrowserRenderer` and
    /// reuse it across pages rather than launching per-page.
    pub async fn launch() -> Result<Self, BrowserError> {
        let config = BrowserConfig::builder()
            .build()
            .map_err(BrowserError::Launch)?;
        let (browser, mut handler) = Browser::launch(config)
            .await
            .map_err(|e| BrowserError::Launch(e.to_string()))?;
        let handler_task = tokio::spawn(async move { while handler.next().await.is_some() {} });
        Ok(Self {
            browser,
            _handler_task: handler_task,
        })
    }

    /// Navigates to `url`, waits for the load event, and returns the
    /// rendered HTML plus every network request/response observed from
    /// navigation start through an additional `settle` wait (many
    /// analytics tags fire XHR/fetch calls shortly after `load`, not
    /// during it).
    pub async fn render(&self, url: &Url, settle: Duration) -> Result<RenderedPage, BrowserError> {
        let page = self
            .browser
            .new_page("about:blank")
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;

        // Network events must be enabled, and listeners attached, before
        // navigation starts -- otherwise the page's own first-party
        // request (and any synchronously-fired tag requests) would be
        // missed.
        page.execute(EnableParams::default())
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        let mut request_events = page
            .event_listener::<EventRequestWillBeSent>()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        let mut response_events = page
            .event_listener::<EventResponseReceived>()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;

        let requests = Arc::new(Mutex::new(Vec::<NetworkRequestRecord>::new()));
        let requests_for_task = requests.clone();
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let collector = tokio::spawn(async move {
            loop {
                tokio::select! {
                    Some(ev) = request_events.next() => {
                        requests_for_task.lock().unwrap_or_else(|p| p.into_inner()).push(NetworkRequestRecord {
                            url: ev.request.url.clone(),
                            method: ev.request.method.clone(),
                            resource_type: format!("{:?}", ev.r#type),
                            status: None,
                            mime_type: None,
                        });
                    }
                    Some(ev) = response_events.next() => {
                        let mut guard = requests_for_task.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(rec) = guard.iter_mut().rev().find(|r| r.url == ev.response.url) {
                            rec.status = Some(ev.response.status);
                            rec.mime_type = Some(ev.response.mime_type.clone());
                        }
                    }
                    _ = &mut stop_rx => break,
                }
            }
        });

        page.goto(url.as_str())
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;
        page.wait_for_navigation()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;

        tokio::time::sleep(settle).await;

        let html = page
            .content()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;

        let final_url_str = page
            .url()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?
            .unwrap_or_else(|| url.to_string());
        let final_url = Url::parse(&final_url_str).unwrap_or_else(|_| url.clone());

        let raw_cookies = page
            .get_cookies()
            .await
            .map_err(|e| BrowserError::Cdp(e.to_string()))?;
        let cookies = raw_cookies
            .into_iter()
            .map(|c| CookieRecord {
                name: c.name,
                value: c.value,
                domain: c.domain,
            })
            .collect();

        // Signal the collector to stop and wait for it to actually finish
        // before reading `requests`, so this doesn't race a still-running
        // event loop (an `abort()` here would only guarantee cancellation
        // at the task's *next* await point, not that it has already
        // stopped mutating the shared Vec).
        let _ = stop_tx.send(());
        let _ = collector.await;
        let network_requests = requests.lock().unwrap_or_else(|p| p.into_inner()).clone();

        page.close()
            .await
            .map_err(|e| BrowserError::Navigation(e.to_string()))?;

        Ok(RenderedPage {
            url: url.clone(),
            final_url,
            html,
            network_requests,
            cookies,
        })
    }
}
