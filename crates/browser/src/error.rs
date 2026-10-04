use thiserror::Error;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("failed to launch browser: {0}")]
    Launch(String),
    #[error("navigation failed: {0}")]
    Navigation(String),
    #[error("CDP command failed: {0}")]
    Cdp(String),
}
