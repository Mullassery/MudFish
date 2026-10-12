"""Mudfish Crawler — Python bindings for the Rust-native crawl engine.

This package wraps the compiled ``mudfish._mudfish`` extension in a typed,
Pythonic API:

* results are :class:`CrawlResult` / :class:`Page` / :class:`CrawlStats`
  dataclasses instead of ad-hoc dicts (every typed object still supports
  ``obj["key"]`` and ``obj.to_dict()`` for the raw engine shape);
* :class:`CrawlResult` is iterable over its pages and supports ``len()``;
* :func:`crawl_async` and :meth:`Crawler.crawl_async` run the blocking engine
  on a worker thread, so they compose with ``asyncio`` without blocking the
  event loop;
* failures raise :class:`MudfishError` subclasses (which are also the
  historical built-in ``ValueError`` / ``RuntimeError``, so existing
  ``except`` clauses keep working).

Quick start::

    import mudfish

    result = mudfish.crawl("https://example.com", depth=2)
    print(result.stats.urls_fetched)
    for page in result:
        print(page.status_code, page.url, page.metadata.title)
"""

from __future__ import annotations

import asyncio
from dataclasses import asdict, dataclass, fields
from enum import Enum
from typing import Any, Iterator, Mapping, Optional

from ._mudfish import crawl as _crawl

__all__ = [
    "crawl",
    "crawl_dict",
    "crawl_async",
    "Crawler",
    "CrawlOptions",
    "CrawlResult",
    "CrawlStats",
    "CrawlError",
    "Page",
    "PageMetadata",
    "Link",
    "FetchMethod",
    "MudfishError",
    "InvalidURLError",
    "CrawlFailedError",
]

__version__ = "0.2.0"


class MudfishError(Exception):
    """Base class for every error raised by :mod:`mudfish`."""


class InvalidURLError(MudfishError, ValueError):
    """The seed URL could not be parsed.

    Subclasses :class:`ValueError`, so ``except ValueError`` still catches it.
    """


class CrawlFailedError(MudfishError, RuntimeError):
    """The crawl engine failed to run to completion.

    Subclasses :class:`RuntimeError`, so ``except RuntimeError`` still
    catches it.
    """


class FetchMethod(str, Enum):
    """How a page's HTML was obtained."""

    HTTP = "Http"
    """Plain HTTP GET; JavaScript was not executed."""

    BROWSER = "Browser"
    """Loaded in headless Chrome and captured post-JavaScript."""

    @classmethod
    def _from_raw(cls, value: Any) -> "FetchMethod":
        if isinstance(value, cls):
            return value
        try:
            return cls(value)
        except ValueError:
            return value  # type: ignore[return-value]


def _as_raw(value: Any) -> Any:
    if isinstance(value, Enum):
        return value.value
    if isinstance(value, _AsDict):
        return value.to_dict()
    if isinstance(value, (list, tuple)):
        return [_as_raw(item) for item in value]
    if isinstance(value, dict):
        return {key: _as_raw(item) for key, item in value.items()}
    return value


class _AsDict:
    """Mixin giving dataclasses dict-style read access to their raw shape."""

    def to_dict(self) -> dict[str, Any]:
        """Return the plain, JSON-compatible dict for this object."""
        return {f.name: _as_raw(getattr(self, f.name)) for f in fields(self)}

    def __getitem__(self, key: str) -> Any:
        return self.to_dict()[key]

    def get(self, key: str, default: Any = None) -> Any:
        return self.to_dict().get(key, default)

    def keys(self) -> Any:
        return self.to_dict().keys()

    def values(self) -> Any:
        return self.to_dict().values()

    def items(self) -> Any:
        return self.to_dict().items()

    def __contains__(self, key: object) -> bool:
        return key in self.to_dict()


@dataclass(frozen=True)
class Link(_AsDict):
    """A hyperlink discovered on a page."""

    url: str
    anchor_text: Optional[str] = None
    rel: Optional[str] = None


@dataclass(frozen=True)
class PageMetadata(_AsDict):
    """The subset of ``<head>`` metadata the parser extracts."""

    title: Optional[str] = None
    meta_description: Optional[str] = None
    canonical: Optional[str] = None


@dataclass(frozen=True)
class Page(_AsDict):
    """A single fetched page."""

    url: str
    final_url: str
    status_code: int
    content_type: Optional[str]
    depth: int
    metadata: PageMetadata
    links: list[Link]
    body_bytes: int
    fetched_via: FetchMethod

    @property
    def title(self) -> Optional[str]:
        """Convenience alias for ``metadata.title``."""
        return self.metadata.title

    @property
    def is_ok(self) -> bool:
        """True for 2xx responses."""
        return 200 <= self.status_code < 300


@dataclass(frozen=True)
class CrawlError(_AsDict):
    """A URL that could not be fetched (with the reason)."""

    url: str
    message: str


@dataclass(frozen=True)
class CrawlStats(_AsDict):
    """Aggregate counters for a crawl."""

    urls_discovered: int = 0
    urls_fetched: int = 0
    urls_skipped: int = 0
    errors: int = 0
    bytes_downloaded: int = 0
    duration_ms: int = 0

    @property
    def duration_secs(self) -> float:
        """Crawl wall-clock duration in seconds."""
        return self.duration_ms / 1000.0

    @property
    def throughput_per_sec(self) -> float:
        """Fetched pages per second (0.0 if the crawl was instantaneous)."""
        if self.duration_ms == 0:
            return 0.0
        return self.urls_fetched / (self.duration_ms / 1000.0)


@dataclass(frozen=True)
class CrawlResult(_AsDict):
    """The full result of a crawl.

    Iterating a result yields its :class:`Page` objects, and ``len(result)``
    is the number of pages fetched::

        for page in result:
            ...

    The historical dict access (``result["pages"]``, ``result["stats"]``,
    ...) still returns the raw shape for backwards compatibility.
    """

    crawl_id: str
    pages: list[Page]
    errors: list[CrawlError]
    stats: CrawlStats

    def __iter__(self) -> Iterator[Page]:
        return iter(self.pages)

    def __len__(self) -> int:
        return len(self.pages)

    @property
    def ok_pages(self) -> list[Page]:
        """Only the pages that returned a 2xx status."""
        return [page for page in self.pages if page.is_ok]


@dataclass
class CrawlOptions:
    """Tunable options for a crawl; mirrors the CLI flags / engine config.

    Instances are accepted by :func:`crawl`, :func:`crawl_dict`,
    :func:`crawl_async` and :class:`Crawler` via ``**options`` keywords.
    """

    depth: int = 3
    """Maximum link-following depth from the seed."""

    concurrency: int = 50
    """Maximum requests in flight, across the whole crawl."""

    per_host_concurrency: int = 4
    """Maximum requests in flight to any single host."""

    same_domain: bool = True
    """Restrict the crawl to the seed's resolved domain."""

    max_urls: int = 10_000
    """Hard cap on the number of URLs fetched."""

    max_duration_secs: Optional[int] = None
    """Optional wall-clock budget in seconds (``None`` = unbounded)."""

    timeout_secs: int = 30
    """Per-request timeout in seconds."""

    request_delay_ms: int = 0
    """Minimum delay between requests to the same host, in milliseconds."""

    max_response_bytes: int = 20 * 1024 * 1024
    """Per-response size cap in bytes (default 20 MB)."""

    respect_robots: bool = True
    """Honour ``robots.txt``."""

    allow_private_networks: bool = False
    """Permit loopback/private targets. Internal use only — this disables
    the SSRF guard."""

    render_js: bool = False
    """Re-render JavaScript-dependent pages in headless Chrome. Requires
    Chrome on the host and is much slower."""

    render_settle_ms: int = 2000
    """When ``render_js`` is set, how long to let a page settle before
    capturing the rendered DOM."""

    def as_kwargs(self) -> dict[str, Any]:
        """The options as a keyword dict for the Rust binding."""
        return asdict(self)

    @classmethod
    def from_kwargs(cls, options: Mapping[str, Any]) -> "CrawlOptions":
        """Build options from keyword arguments, rejecting unknown names."""
        return cls(**options)


def _run_raw(url: str, options: Mapping[str, Any]) -> dict[str, Any]:
    resolved = CrawlOptions.from_kwargs(options).as_kwargs()
    try:
        return _crawl(url, **resolved)
    except InvalidURLError:
        raise
    except ValueError as exc:
        raise InvalidURLError(str(exc)) from exc
    except RuntimeError as exc:
        raise CrawlFailedError(str(exc)) from exc


def _parse_link(raw: Mapping[str, Any]) -> Link:
    return Link(
        url=raw["url"],
        anchor_text=raw.get("anchor_text"),
        rel=raw.get("rel"),
    )


def _parse_page(raw: Mapping[str, Any]) -> Page:
    meta = raw.get("metadata") or {}
    return Page(
        url=raw["url"],
        final_url=raw.get("final_url", raw["url"]),
        status_code=raw.get("status_code", 0),
        content_type=raw.get("content_type"),
        depth=raw.get("depth", 0),
        metadata=PageMetadata(
            title=meta.get("title"),
            meta_description=meta.get("meta_description"),
            canonical=meta.get("canonical"),
        ),
        links=[_parse_link(link) for link in raw.get("links", [])],
        body_bytes=raw.get("body_bytes", 0),
        fetched_via=FetchMethod._from_raw(raw.get("fetched_via", "Http")),
    )


def _parse_result(raw: Mapping[str, Any]) -> CrawlResult:
    stats = raw.get("stats") or {}
    return CrawlResult(
        crawl_id=str(raw.get("crawl_id", "")),
        pages=[_parse_page(page) for page in raw.get("pages", [])],
        errors=[
            CrawlError(url=err.get("url", ""), message=err.get("message", ""))
            for err in raw.get("errors", [])
        ],
        stats=CrawlStats(
            urls_discovered=stats.get("urls_discovered", 0),
            urls_fetched=stats.get("urls_fetched", 0),
            urls_skipped=stats.get("urls_skipped", 0),
            errors=stats.get("errors", 0),
            bytes_downloaded=stats.get("bytes_downloaded", 0),
            duration_ms=stats.get("duration_ms", 0),
        ),
    )


def crawl(url: str, **options: Any) -> CrawlResult:
    """Crawl ``url`` and return a typed :class:`CrawlResult`.

    Accepts the keywords documented on :class:`CrawlOptions` (e.g. ``depth``,
    ``concurrency``, ``same_domain``, ``render_js``, ...). Blocks until the
    crawl finishes; use :func:`crawl_async` from ``asyncio`` code. The GIL is
    released while the crawl runs.

    Raises:
        InvalidURLError: if ``url`` cannot be parsed.
        CrawlFailedError: if the engine fails to run.
        TypeError: if an unknown option is passed.
    """
    return _parse_result(_run_raw(url, options))


def crawl_dict(url: str, **options: Any) -> dict[str, Any]:
    """Like :func:`crawl`, but return the raw engine dict (untyped).

    This is the stable, JSON-shaped structure — useful if you want to
    serialise the result or avoid constructing the typed objects.
    """
    return _run_raw(url, options)


async def crawl_async(url: str, **options: Any) -> CrawlResult:
    """Async variant of :func:`crawl`.

    Runs the blocking engine on a worker thread, so the event loop keeps
    running and many crawls can be awaited concurrently::

        results = await asyncio.gather(
            mudfish.crawl_async("https://a.example"),
            mudfish.crawl_async("https://b.example"),
        )

    Accepts the same keywords as :func:`crawl`.
    """
    return await asyncio.to_thread(crawl, url, **options)


class Crawler:
    """A reusable crawl configuration.

    Construct once with options, then crawl many seeds without repeating
    them::

        crawler = mudfish.Crawler(depth=1, concurrency=10)
        home = crawler.crawl("https://example.com")
        blog = await crawler.crawl_async("https://example.com/blog")
    """

    def __init__(self, **options: Any) -> None:
        self.options = CrawlOptions.from_kwargs(options)

    def crawl(self, url: str) -> CrawlResult:
        """Crawl ``url`` using this crawler's options."""
        return crawl(url, **self.options.as_kwargs())

    def crawl_dict(self, url: str) -> dict[str, Any]:
        """Like :meth:`crawl`, but return the raw engine dict."""
        return crawl_dict(url, **self.options.as_kwargs())

    async def crawl_async(self, url: str) -> CrawlResult:
        """Async variant of :meth:`crawl`."""
        return await crawl_async(url, **self.options.as_kwargs())

    def __repr__(self) -> str:
        return f"Crawler({self.options!r})"
