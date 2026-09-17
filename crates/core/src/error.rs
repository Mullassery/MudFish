use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid URL: {0}")]
    UrlParse(#[from] url::ParseError),
    #[error("{0}")]
    Other(String),
}
