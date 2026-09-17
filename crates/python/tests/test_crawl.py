"""Tests for the Python bindings layer itself (marshalling, error handling).

Crawl *correctness* (dedup, retries, robots.txt, SSRF guard internals, etc.)
is already covered by the Rust engine's own test suite; these tests exist to
verify the PyO3 boundary — that Rust structs make it across as the expected
dict shape and that Rust errors become the right Python exceptions — using a
local HTTP server so they run offline and fast.
"""

from __future__ import annotations

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
