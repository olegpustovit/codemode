#!/usr/bin/env python3
"""Verify the embedded frontend through a running Rust binary."""
import argparse
import json
from html.parser import HTMLParser
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlsplit


class Links(HTMLParser):
    def __init__(self):
        super().__init__()
        self.assets = []

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        path = attrs.get("src" if tag == "script" else "href", "")
        if path.startswith("/assets/"):
            self.assets.append(path)


parser = argparse.ArgumentParser()
parser.add_argument("url")
parser.add_argument("hostname")
args = parser.parse_args()


def get(path, method="GET", host=None):
    request = Request(args.url.rstrip("/") + path,
                      headers={"Host": host or args.hostname, "User-Agent": "codemode-smoke/0.1"}, method=method)
    try:
        response = urlopen(request, timeout=5)
    except HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


status, headers, index = get("/")
assert status == 200 and headers["Cache-Control"] == "no-cache"
assert headers["X-Frame-Options"] == "DENY"
assert headers["Content-Security-Policy"] == "frame-ancestors 'none'; base-uri 'self'; object-src 'none'"
assert headers["Content-Type"].startswith("text/html") and b'<div id="root">' in index
for path in ["/sources/00000000-0000-0000-0000-000000000001/accounts?oauth=linked", "/tokens", "/index.html"]:
    status, headers, body = get(path)
    assert status == 200 and body == index and headers["Cache-Control"] == "no-cache", path
status, headers, body = get("/sources", "HEAD")
assert status == 200 and not body and headers["Cache-Control"] == "no-cache"
links = Links()
links.feed(index.decode())
assert any(path.endswith(".js") for path in links.assets)
assert any(path.endswith(".css") for path in links.assets)
for path in links.assets:
    status, headers, body = get(path)
    mime = "text/javascript" if path.endswith(".js") else "text/css"
    assert status == 200 and body and headers["Content-Type"].startswith(mime), path
    assert "immutable" in headers["Cache-Control"], path
    status, headers, body = get(path, "HEAD")
    assert status == 200 and not body
for path in ["/assets/missing.js", "/assets", "/missing.svg", "/oauth/missing", "/mcp-admin/missing", "/.well-known/missing"]:
    status, headers, body = get(path)
    assert status == 404 and b"<!doctype html>" not in body.lower(), path
status, headers, body = get("/api/nope")
assert status == 404 and headers["Content-Type"] == "application/json"
assert json.loads(body) == {"error": "route not found"}
status, headers, body = get("/api/config")
assert status == 200 and json.loads(body)["hostname"] == args.hostname
for resource,scope in [("/mcp","mcp"),("/mcp-admin","admin")]:
    status, headers, body = get(resource)
    assert status == 401 and f"oauth-protected-resource{resource}" in headers["WWW-Authenticate"] and f'scope="{scope}"' in headers["WWW-Authenticate"]
    status,headers,body = get(f"/.well-known/oauth-protected-resource{resource}")
    metadata = json.loads(body)
    assert status == 200 and metadata["resource"] == f"https://{args.hostname}{resource}" and metadata["scopes_supported"] == [scope]
status, headers, body = get("/healthz")
assert status == 200 and json.loads(body)["status"] == "ok"
status, headers, body = get("/site.webmanifest")
assert status == 200 and headers["Content-Type"] == "application/manifest+json"
manifest = json.loads(body)
for icon in manifest["icons"]:
    status, headers, body = get(icon["src"])
    assert status == 200 and headers["Content-Type"] == "image/png" and body.startswith(b"\x89PNG")
if urlsplit(args.url).hostname in {"127.0.0.1", "localhost", "::1"}:
    status, headers, body = get("/", host="unknown.example")
    assert status == 404 and b"<!doctype html>" not in body.lower()
    print("PASS: loopback unknown-host rejection")
print("PASS: embedded HTML/deep links, GET/HEAD assets and cache headers, JSON 404, reserved routes, config, MCP, manifest and icons")
