"""Tests for the Python bindings layer itself (marshalling, error handling).

Crawl *correctness* (dedup, retries, robots.txt, SSRF guard internals, etc.)
is already covered by the Rust engine's own test suite; these tests exist to
verify the PyO3 boundary — that Rust structs make it across as the expected
dict shape and that Rust errors become the right Python exceptions — using a
local HTTP server so they run offline and fast.
"""

from __future__ import annotations

import asyncio
import http.server
import socketserver
import threading

import pytest

import mudfish

HOME_HTML = b"<html><head><title>Home</title></head><body><a href=\"/about\">About</a></body></html>"
ABOUT_HTML = b"<html><head><title>About</title></head><body><a href=\"/\">Home</a></body></html>"


class _Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 (stdlib method name)
        if self.path == "/":
            body = HOME_HTML
        elif self.path == "/about":
            body = ABOUT_HTML
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, format: str, *args: object) -> None:  # noqa: A002
        pass  # keep test output quiet


@pytest.fixture(scope="module")
def server() -> str:
    httpd = socketserver.TCPServer(("127.0.0.1", 0), _Handler)
    port = httpd.server_address[1]
    thread = threading.Thread(target=httpd.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{port}"
    httpd.shutdown()
    thread.join(timeout=5)


def test_basic_crawl_returns_expected_shape(server: str) -> None:
    result = mudfish.crawl(server + "/", depth=0, allow_private_networks=True)

    assert result["stats"]["urls_fetched"] == 1
    assert result["stats"]["errors"] == 0
    assert isinstance(result["crawl_id"], str) and result["crawl_id"]

    page = result["pages"][0]
    assert page["status_code"] == 200
    assert page["metadata"]["title"] == "Home"
    assert page["links"][0]["url"] == server + "/about"


def test_follows_same_domain_links(server: str) -> None:
    result = mudfish.crawl(server + "/", depth=1, allow_private_networks=True)

    assert result["stats"]["urls_fetched"] == 2
    paths = sorted(p["url"].replace(server, "") for p in result["pages"])
    assert paths == ["/", "/about"]


def test_invalid_url_raises_value_error() -> None:
    with pytest.raises(ValueError):
        mudfish.crawl("not-a-url")


def test_ssrf_guard_blocks_loopback_by_default() -> None:
    result = mudfish.crawl(
        "http://127.0.0.1:1/", depth=0, respect_robots=False, timeout_secs=2
    )

    assert result["stats"]["urls_fetched"] == 0
    assert len(result["errors"]) == 1


def test_returns_typed_result(server: str) -> None:
    result = mudfish.crawl(server + "/", depth=0, allow_private_networks=True)

    assert isinstance(result, mudfish.CrawlResult)
    assert isinstance(result.stats, mudfish.CrawlStats)
    assert result.stats.urls_fetched == 1
    assert result.stats.duration_secs >= 0
    assert isinstance(result.crawl_id, str) and result.crawl_id

    page = result.pages[0]
    assert isinstance(page, mudfish.Page)
    assert page.status_code == 200
    assert page.title == "Home"
    assert page.metadata.title == "Home"
    assert page.is_ok is True
    assert page.fetched_via is mudfish.FetchMethod.HTTP
    assert isinstance(page.links[0], mudfish.Link)
    assert page.links[0].url == server + "/about"


def test_result_is_iterable_over_pages(server: str) -> None:
    result = mudfish.crawl(server + "/", depth=1, allow_private_networks=True)

    assert len(result) == 2
    urls = sorted(page.url for page in result)
    assert urls == [server + "/", server + "/about"]
    assert result.ok_pages == result.pages


def test_typed_result_still_supports_dict_access(server: str) -> None:
    result = mudfish.crawl(server + "/", depth=0, allow_private_networks=True)

    # the historical dict shape must keep working
    assert result["stats"]["urls_fetched"] == 1
    assert result["pages"][0]["status_code"] == 200
    assert result["pages"][0]["metadata"]["title"] == "Home"
    assert result["crawl_id"] == result.crawl_id
    assert "pages" in result


def test_crawl_dict_returns_plain_dict(server: str) -> None:
    raw = mudfish.crawl_dict(server + "/", depth=0, allow_private_networks=True)

    assert type(raw) is dict
    assert raw["stats"]["urls_fetched"] == 1
    assert raw["pages"][0]["metadata"]["title"] == "Home"


def test_crawl_async(server: str) -> None:
    result = asyncio.run(
        mudfish.crawl_async(server + "/", depth=0, allow_private_networks=True)
    )

    assert isinstance(result, mudfish.CrawlResult)
    assert result.stats.urls_fetched == 1


def test_crawler_reuses_options(server: str) -> None:
    crawler = mudfish.Crawler(depth=0, allow_private_networks=True)

    result = crawler.crawl(server + "/")
    assert result.stats.urls_fetched == 1
    assert crawler.options.depth == 0

    raw = crawler.crawl_dict(server + "/")
    assert type(raw) is dict
    assert raw["stats"]["urls_fetched"] == 1

    async_result = asyncio.run(crawler.crawl_async(server + "/"))
    assert async_result.stats.urls_fetched == 1


def test_invalid_url_raises_custom_error() -> None:
    with pytest.raises(mudfish.InvalidURLError):
        mudfish.crawl("not-a-url")

    # backwards compatible with the old built-in exception contract
    with pytest.raises(ValueError):
        mudfish.crawl("not-a-url")

    assert issubclass(mudfish.InvalidURLError, mudfish.MudfishError)


def test_unknown_option_raises_type_error(server: str) -> None:
    with pytest.raises(TypeError):
        mudfish.crawl(server + "/", depthh=2, allow_private_networks=True)


def test_render_options_are_forwarded(server: str) -> None:
    options = mudfish.CrawlOptions(render_js=True, render_settle_ms=500)
    assert options.render_js is True
    assert options.render_settle_ms == 500
    assert mudfish.CrawlOptions().render_js is False

    # pass the render keywords through end-to-end (render_js=False so the
    # test does not require headless Chrome on the host)
    result = mudfish.crawl(
        server + "/",
        depth=0,
        allow_private_networks=True,
        render_js=False,
        render_settle_ms=500,
    )
    assert result.stats.urls_fetched == 1
