use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

use mudfish_core::CrawlResult;

use crate::cli::OutputFormat;

pub fn emit(
    result: &CrawlResult,
    format: OutputFormat,
    out_file: Option<&Path>,
) -> anyhow::Result<()> {
    let mut writer: Box<dyn Write> = match out_file {
        Some(path) => Box::new(File::create(path)?),
        None => Box::new(io::stdout()),
    };

    match format {
        OutputFormat::Summary => write_summary(&mut writer, result)?,
        OutputFormat::Json => {
            serde_json::to_writer_pretty(&mut writer, result)?;
            writeln!(writer)?;
        }
        OutputFormat::Jsonl => {
            for page in &result.pages {
                serde_json::to_writer(&mut writer, page)?;
                writeln!(writer)?;
            }
        }
    }
    Ok(())
}

fn write_summary(writer: &mut dyn Write, result: &CrawlResult) -> anyhow::Result<()> {
    let stats = &result.stats;
    writeln!(writer, "Crawl complete ({})", result.crawl_id)?;
    writeln!(writer)?;
    writeln!(writer, "URLs discovered: {}", stats.urls_discovered)?;
    writeln!(writer, "URLs fetched:    {}", stats.urls_fetched)?;
    writeln!(writer, "URLs skipped:    {}", stats.urls_skipped)?;
    writeln!(writer, "Errors:          {}", stats.errors)?;
    writeln!(
        writer,
        "Bytes:           {}",
        format_bytes(stats.bytes_downloaded)
    )?;
    writeln!(
        writer,
        "Duration:        {:.1}s",
        stats.duration_ms as f64 / 1000.0
    )?;
    writeln!(
        writer,
        "Throughput:      {:.1} URLs/sec",
        stats.throughput_per_sec()
    )?;
    if !result.errors.is_empty() {
        writeln!(writer)?;
        writeln!(writer, "First errors:")?;
        for err in result.errors.iter().take(10) {
            writeln!(writer, "  {} - {}", err.url, err.message)?;
        }
    }
    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit_idx = 0;
    while value >= 1024.0 && unit_idx < UNITS.len() - 1 {
        value /= 1024.0;
        unit_idx += 1;
    }
    format!("{:.1} {}", value, UNITS[unit_idx])
}
