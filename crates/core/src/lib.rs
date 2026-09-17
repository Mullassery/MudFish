pub mod config;
pub mod error;
pub mod normalize;
pub mod types;

pub use config::CrawlConfig;
pub use error::CoreError;
pub use normalize::{fingerprint, normalize_url, NormalizationOptions, TrailingSlashPolicy};
pub use types::*;
