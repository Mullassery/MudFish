use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chromiumoxide::cdp::browser_protocol::network::{
    EnableParams, EventRequestWillBeSent, EventResponseReceived,
};
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;
use url::Url;

use crate::error::BrowserError;

/// Quiet window with no observed network activity after which the settle
/// phase is considered done (bounded above by the caller's `settle`).
const IDLE_QUIET: Duration = Duration::from_millis(500);
/// Poll granularity while waiting for the network to go idle.
const IDLE_POLL: Duration = Duration::from_millis(50);

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
    ///
    /// When `chrome_path` is `Some`, that binary is used verbatim instead of
    /// autodetection -- this is how bundled distributions point at the
    /// `chrome-headless-shell` they ship. The binary is made executable
    /// first if a packaged wheel dropped the mode bit.
    pub async fn launch(chrome_path: Option<&std::path::Path>) -> Result<Self, BrowserError> {
        let mut config = BrowserConfig::builder();
        if let Some(path) = chrome_path {
            ensure_executable(path)?;
            config = config.chrome_executable(path);
        }
        let config = config.build().map_err(BrowserError::Launch)?;
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
    /// navigation start through the settle phase (many analytics tags fire
    /// XHR/fetch calls shortly after `load`, not during it).
    ///
    /// Rather than always blocking for the full fixed `settle` duration,
    /// the settle phase is network-idle driven: it returns as soon as no
    /// request/response has been observed for `IDLE_QUIET` (a short quiet
    /// window), and otherwise falls back to the fixed `settle` ceiling so a
    /// page that never goes quiet still terminates. This makes extraction
    /// on dynamic pages both faster (idle pages return early) and more
    /// correct (tag requests that land late still extend the wait up to the
    /// ceiling).
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
        let started = Instant::now();
        // Milliseconds-since-`started` of the last observed network event;
        // the settle loop uses this to detect the idle window. Written by
        // the collector task, read by the render task.
        let last_activity_ms = Arc::new(AtomicU64::new(0));
        let last_activity_for_task = last_activity_ms.clone();
        let started_for_task = started;
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let collector = tokio::spawn(async move {
            loop {
                let touch = |slot: &AtomicU64| {
                    slot.store(
                        started_for_task.elapsed().as_millis() as u64,
                        Ordering::Relaxed,
                    );
                };
                tokio::select! {
                    Some(ev) = request_events.next() => {
                        touch(&last_activity_for_task);
                        requests_for_task.lock().unwrap_or_else(|p| p.into_inner()).push(NetworkRequestRecord {
                            url: ev.request.url.clone(),
                            method: ev.request.method.clone(),
                            resource_type: format!("{:?}", ev.r#type),
                            status: None,
                            mime_type: None,
                        });
                    }
                    Some(ev) = response_events.next() => {
                        touch(&last_activity_for_task);
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

        // Network-idle settle: poll until no request/response has been seen
        // for `IDLE_QUIET`, capped at the caller's `settle` ceiling. The
        // first poll always waits at least `IDLE_QUIET` so a page whose
        // tags fire just after `load` isn't captured mid-flight.
        let idle_start = Instant::now();
        let quiet = IDLE_QUIET.min(settle);
        loop {
            let elapsed = idle_start.elapsed();
            if elapsed >= settle {
                break;
            }
            let since_activity = elapsed.as_millis() as u64
                - last_activity_ms
                    .load(Ordering::Relaxed)
                    .min(elapsed.as_millis() as u64);
            if elapsed >= quiet && since_activity >= quiet.as_millis() as u64 {
                break;
            }
            tokio::time::sleep(IDLE_POLL.min(settle.saturating_sub(elapsed))).await;
        }

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

/// Ensures `path` is executable, restoring the mode bit if a packaged
/// distribution (e.g. a wheel) dropped it. No-op on non-Unix platforms.
fn ensure_executable(path: &std::path::Path) -> Result<(), BrowserError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(path)
            .map_err(|e| BrowserError::Launch(format!("chrome binary {}: {e}", path.display())))?;
        let mut perms = metadata.permissions();
        if perms.mode() & 0o111 == 0 {
            perms.set_mode(perms.mode() | 0o755);
            std::fs::set_permissions(path, perms).map_err(|e| {
                BrowserError::Launch(format!(
                    "could not mark chrome binary {} executable: {e}",
                    path.display()
                ))
            })?;
        }
    }
    let _ = path;
    Ok(())
}
