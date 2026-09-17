# mudfish

Python bindings for [Mudfish Crawler](https://github.com/Mullassery/MudFish), a Rust-native web crawling and intelligence engine.

This package wraps the Rust crawl engine directly (via [PyO3](https://pyo3.rs)) — there is no subprocess or network hop to a separate server. It currently exposes one function, synchronous/blocking crawling; the Rust project itself is at an early stage (Phase 1: HTTP-only crawling — see the main repo's `ROADMAP_HONEST.md` for full status).

## Install

```bash
pip install mudfish
```

## Usage

```python
import mudfish

result = mudfish.crawl("https://example.com", depth=2, concurrency=30)

print(result["stats"])
for page in result["pages"]:
    print(page["status_code"], page["url"], page["metadata"]["title"])
```

`crawl()` blocks until the crawl finishes (or hits `max_urls` / `max_duration_secs`) and returns a plain `dict`:

```python
{
    "crawl_id": str,
    "pages": [
        {
            "url": str, "final_url": str, "status_code": int,
            "content_type": str | None, "depth": int,
            "metadata": {"title": str | None, "meta_description": str | None, "canonical": str | None},
            "links": [{"url": str, "anchor_text": str | None, "rel": str | None}, ...],
            "body_bytes": int, "fetched_via": "Http",
        },
        ...
    ],
    "errors": [{"url": str, "message": str}, ...],
    "stats": {
        "urls_discovered": int, "urls_fetched": int, "urls_skipped": int,
        "errors": int, "bytes_downloaded": int, "duration_ms": int,
    },
}
```

### Options

All keyword-only, matching the CLI's flags:

| Parameter | Default | Meaning |
|---|---|---|
| `depth` | `3` | Max link-following depth |
| `concurrency` | `50` | Max requests in flight, whole crawl |
| `per_host_concurrency` | `4` | Max requests in flight per host |
| `same_domain` | `True` | Restrict crawl to the seed's resolved domain |
| `max_urls` | `10000` | Hard cap on URLs fetched |
| `max_duration_secs` | `None` | Optional wall-clock budget |
| `timeout_secs` | `30` | Per-request timeout |
| `request_delay_ms` | `0` | Min delay between requests to the same host |
| `max_response_bytes` | `20 MB` | Per-response size cap |
| `respect_robots` | `True` | Honor robots.txt |
| `allow_private_networks` | `False` | Permit loopback/private targets (internal use only) |

## Limitations

- **Blocking, not async.** `crawl()` spins up its own Tokio runtime internally and blocks until done. The GIL is released while it runs (other Python threads keep going), but there is no `asyncio` integration in this version.
- **Single-platform wheels initially.** Built and published from the maintainer's machine, not a cross-platform CI matrix yet — check PyPI for which platforms have wheels; others will need a Rust toolchain to build from source.

## License

Apache-2.0.
