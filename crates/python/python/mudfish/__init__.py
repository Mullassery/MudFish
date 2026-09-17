"""Mudfish Crawler — Python bindings for the Rust-native crawl engine."""

from __future__ import annotations

from typing import Any, Optional

from ._mudfish import crawl as _crawl

__all__ = ["crawl"]
__version__ = "0.1.0"


def crawl(
    url: str,
    *,
    depth: int = 3,
    concurrency: int = 50,
    per_host_concurrency: int = 4,
    same_domain: bool = True,
    max_urls: int = 10_000,
    max_duration_secs: Optional[int] = None,
    timeout_secs: int = 30,
    request_delay_ms: int = 0,
    max_response_bytes: int = 20 * 1024 * 1024,
    respect_robots: bool = True,
    allow_private_networks: bool = False,
) -> dict[str, Any]:
    """Crawl a website starting from ``url`` and return the result.

    Blocks until the crawl finishes (or hits ``max_urls`` /
    ``max_duration_secs``); there is no asyncio integration in this version.
    The GIL is released while the crawl runs, so it won't block other
    Python threads.

    Returns a dict with keys ``crawl_id``, ``pages``, ``errors``, ``stats``
    — see the package README for the full shape.
    """
    return _crawl(
        url,
        depth=depth,
        concurrency=concurrency,
        per_host_concurrency=per_host_concurrency,
        same_domain=same_domain,
        max_urls=max_urls,
        max_duration_secs=max_duration_secs,
        timeout_secs=timeout_secs,
        request_delay_ms=request_delay_ms,
        max_response_bytes=max_response_bytes,
        respect_robots=respect_robots,
        allow_private_networks=allow_private_networks,
    )
