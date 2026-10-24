# mudfish

Python bindings for [Mudfish Crawler](https://github.com/Mullassery/MudFish), a Rust-native web crawling and intelligence engine.

This package wraps the Rust crawl engine directly (via [PyO3](https://pyo3.rs)) — there is no subprocess or network hop to a separate server. It exposes a typed, Pythonic API over the engine; the Rust project itself is at an early stage (see the main repo's `../../docs/internal/ROADMAP_HONEST.md` for full status).

## Install

```bash
pip install mudfish
```

## Usage

```python
import mudfish

result = mudfish.crawl("https://example.com", depth=2, concurrency=30)

print(result.stats.urls_fetched, result.stats.duration_secs)
for page in result:                          # CrawlResult is iterable over its pages
    print(page.status_code, page.url, page.title)
```

`crawl()` blocks until the crawl finishes (or hits `max_urls` / `max_duration_secs`) and returns a typed `CrawlResult`:

| Object | Fields |
|---|---|
| `CrawlResult` | `crawl_id`, `pages: list[Page]`, `errors: list[CrawlError]`, `stats: CrawlStats` — iterable over `pages`, `len()` is the page count, plus `ok_pages` |
| `Page` | `url`, `final_url`, `status_code`, `content_type`, `depth`, `metadata: PageMetadata`, `links: list[Link]`, `body_bytes`, `fetched_via: FetchMethod` — helpers `title`, `is_ok` |
| `PageMetadata` | `title`, `meta_description`, `canonical` |
| `Link` | `url`, `anchor_text`, `rel` |
| `CrawlError` | `url`, `message` |
| `CrawlStats` | `urls_discovered`, `urls_fetched`, `urls_skipped`, `errors`, `bytes_downloaded`, `duration_ms` — helpers `duration_secs`, `throughput_per_sec` |
| `FetchMethod` | `FetchMethod.HTTP` (`"Http"`) / `FetchMethod.BROWSER` (`"Browser"`) |

Every typed object also supports the raw engine dict shape (`result["stats"]`, `page["status_code"]`, `page.to_dict()`), so code written against earlier versions keeps working. If you only want the raw dict, use `mudfish.crawl_dict(...)`.

### Async

`crawl_async()` runs the blocking engine on a worker thread, so it composes with `asyncio` and several crawls can run concurrently:

```python
import asyncio
import mudfish

async def main() -> None:
    results = await asyncio.gather(
        mudfish.crawl_async("https://a.example"),
        mudfish.crawl_async("https://b.example"),
    )
    for result in results:
        print(result.stats.urls_fetched)

asyncio.run(main())
```

### Reusable configuration

```python
crawler = mudfish.Crawler(depth=1, concurrency=10)
home = crawler.crawl("https://example.com")
blog = await crawler.crawl_async("https://example.com/blog")
```

### Errors

```python
try:
    mudfish.crawl("not-a-url")
except mudfish.InvalidURLError:      # also a ValueError
    ...
except mudfish.CrawlFailedError:     # also a RuntimeError
    ...
```

Everything derives from `mudfish.MudfishError`.

### Options

All keyword-only, matching the CLI's flags (also available as the `CrawlOptions` dataclass):

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
| `per_host_rate` | `2.0` | Sustained requests/sec per host (token bucket); `0` disables |
| `per_host_burst` | `2` | Per-host token-bucket burst |
| `max_retries` | `2` | Retries after 429/5xx/transport errors |
| `retry_base_ms` | `500` | First retry delay; doubles with jitter |
| `retry_max_ms` | `30000` | Ceiling for computed exponential backoff |
| `max_response_bytes` | `20 MB` | Per-response size cap |
| `respect_robots` | `True` | Honor robots.txt |
| `allow_private_networks` | `False` | Permit loopback/private targets (internal use only) |
| `render_js` | `False` | Re-render JS-dependent pages in headless Chrome (needs Chrome on the host; much slower) |
| `render_settle_ms` | `2000` | When `render_js` is on, settle time before capturing a rendered page |
| `proxies` | `()` | Proxy URLs to route requests through; supply several to rotate a pool |
| `proxy_rotate_after` | `50` | Requests through a proxy before rotating to the next |
| `chrome_path` | `None` | Explicit Chrome/`chrome-headless-shell` binary for `render_js`; defaults to the bundled binary, then host autodetection |

## Limitations

- **Chrome is not bundled in the PyPI wheel.** With `render_js=True` the engine drives a headless Chrome that must already be installed on the host; if it cannot be launched, the crawl logs a warning and falls back to static HTML. A separate **bundled** wheel that ships a `chrome-headless-shell` (so `render_js` works with no host install) is attached to the GitHub release for each version — look for the asset named `mudfish-<version>+chrome-*.whl`.
- **Single-platform wheels initially.** Built and published from the maintainer's machine, not a cross-platform CI matrix yet — check PyPI for which platforms have wheels; others will need a Rust toolchain to build from source.

## Bundled-Chrome builds

To build the bundled wheel locally:

```bash
python3 tools/fetch_chrome_headless_shell.py   # pins + downloads chrome-headless-shell
cd crates/python && maturin build --release
```

The binary is fetched (never committed) into `python/mudfish/_chrome/`, the wheel picks it up automatically, and at runtime `mudfish.bundled_chrome_path()` returns it (overridable with `MUDFISH_CHROME`).

## License

Apache-2.0.
