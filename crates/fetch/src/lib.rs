pub mod challenge;
pub mod error;
pub mod fetcher;
pub mod health;
pub mod politeness;
pub mod robots;
pub mod security;
pub mod status;

pub use challenge::{detect_challenge, ChallengeSignal};
pub use error::FetchError;
pub use fetcher::{FetchResponse, HttpFetcher};
pub use health::{CircuitState, DomainHealthTracker, DomainStats};
pub use politeness::PolitenessManager;
pub use robots::RobotsManager;
pub use status::{classify_status, CrawlStatus};
