use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(
    name = "mudfish",
    version,
    about = "Mudfish Crawler: a Rust-native web crawling and intelligence engine"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Crawl a website starting from a seed URL.
    Crawl(CrawlArgs),
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    /// Human-readable crawl summary (default).
    Summary,
    /// A single JSON object with all pages, errors, and stats.
    Json,
    /// One JSON object per fetched page, newline-delimited.
    Jsonl,
}

#[derive(clap::Args)]
pub struct CrawlArgs {
    /// Seed URL to start crawling from.
    pub url: String,

    /// Maximum link-following depth from the seed.
    #[arg(long, default_value_t = 3)]
    pub depth: usize,

    /// Maximum number of requests in flight across the whole crawl.
    #[arg(long, default_value_t = 50)]
    pub concurrency: usize,

    /// Maximum number of requests in flight to a single host.
    #[arg(long, default_value_t = 4)]
    pub per_host_concurrency: usize,

    /// Follow links to other domains too (default: stay on the seed's domain).
    #[arg(long)]
    pub allow_cross_domain: bool,

    /// Stop after fetching this many URLs.
    #[arg(long, default_value_t = 10_000)]
    pub max_urls: u64,

    /// Stop after this many seconds, however far the crawl has gotten.
    #[arg(long)]
    pub max_duration_secs: Option<u64>,

    /// Per-request timeout, in seconds.
    #[arg(long, default_value_t = 30)]
    pub timeout_secs: u64,

    /// Minimum delay between consecutive requests to the same host, in milliseconds.
    #[arg(long, default_value_t = 0)]
    pub request_delay_ms: u64,

    /// Maximum size of a single response body, in bytes.
    #[arg(long, default_value_t = 20 * 1024 * 1024)]
    pub max_response_bytes: u64,

    /// Do not consult or honor robots.txt. Off by default — only disable
    /// this for infrastructure you own or have explicit permission to crawl.
    #[arg(long)]
    pub no_robots: bool,

    /// Permit fetching loopback/private/link-local addresses. Only for
    /// crawling your own internal infrastructure or local test fixtures.
    #[arg(long)]
    pub allow_private_networks: bool,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Summary)]
    pub output: OutputFormat,

    /// Write output to this file instead of stdout.
    #[arg(long)]
    pub out_file: Option<PathBuf>,
}
