use thiserror::Error;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("request error: {0}")]
    Request(#[from] reqwest::Error),
    #[error("response exceeded max size of {limit} bytes")]
    ResponseTooLarge { limit: u64 },
    #[error("blocked by robots.txt")]
    RobotsDisallowed,
    #[error("blocked by network policy: {0}")]
    PolicyBlocked(String),
}
