#!/usr/bin/env python3
"""Exercise a running gateway without printing its bearer token."""
import argparse
import json
import time
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen
from urllib.parse import urlsplit

parser = argparse.ArgumentParser()
parser.add_argument("url", help="Loopback origin, e.g. http://127.0.0.1:2093")
parser.add_argument("hostname")
parser.add_argument("token_file", type=Path)
args = parser.parse_args()
token = args.token_file.read_text().strip()
if len(token) != 46 or not token.startswith("cm_") or any(c.isspace() for c in token):
    raise SystemExit("Token file must contain exactly one gateway token, without log lines")
sequence = 0


def rpc(method, params, version="2026-07-28", authenticated=True, hostname=None, tasks=False):
    global sequence
    sequence += 1
    headers = {
        "Host": hostname or args.hostname,
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        "MCP-Protocol-Version": version,
        "User-Agent": "codemode-smoke/0.1",
    }
    if authenticated:
        headers["Authorization"] = f"Bearer {token}"
    if version == "2026-07-28":
        headers["Mcp-Method"] = method
        if method == "tools/call":
            headers["Mcp-Name"] = params["name"]
        elif method.startswith("tasks/"):
            headers["Mcp-Name"] = params["taskId"]
        params = {**params, "_meta": {
            "io.modelcontextprotocol/protocolVersion": version,
            "io.modelcontextprotocol/clientCapabilities": {
                "extensions": {"io.modelcontextprotocol/tasks": {}}
            } if tasks else {},
            "io.modelcontextprotocol/clientInfo": {"name": "codemode-smoke", "version": "1"},
        }}
    request = Request(args.url.rstrip("/") + "/mcp", json.dumps({
        "jsonrpc": "2.0", "id": sequence, "method": method, "params": params,
    }).encode(), headers)
    with urlopen(request, timeout=65) as response:
        result = json.load(response)
    assert "error" not in result, result
    return result["result"]


rejections = [(False, None, 401)]
if urlsplit(args.url).hostname in {"127.0.0.1", "localhost", "::1"}:
    rejections.append((True, "unknown.example", 404))
for authenticated, hostname, status in rejections:
    try:
        rpc("tools/list", {}, authenticated=authenticated, hostname=hostname)
        raise AssertionError("unauthorized request succeeded")
    except HTTPError as error:
        assert error.code == status, error.code

for version in ["2025-11-25", "2026-07-28"]:
    if version == "2026-07-28":
        result = rpc("server/discover", {}, version)
        assert version in result["supportedVersions"], result
    else:
        result = rpc("initialize", {
            "protocolVersion": version, "capabilities": {},
            "clientInfo": {"name": "codemode-smoke", "version": "1"},
        }, version)
        assert result["protocolVersion"] == version, result
    tools = rpc("tools/list", {}, version)["tools"]
    assert {tool["name"] for tool in tools} == {"search", "execute", "resume"}
    response = rpc("tools/call", {"name": "execute", "arguments": {
        "code": 'print("smoke")\nreturn {"answer": 42}',
    }}, version)
    assert not response.get("isError"), response
    execution = json.loads(response["content"][0]["text"])
    assert execution["status"] == "completed", execution
    assert execution["result"] == {"answer": 42}, execution
    assert execution["output"] == "smoke\n", execution

for code, should_fail in [
    ('raise ValueError("smoke failure")', True),
    ('try:\n    await _call("missing.tool", "{}")\nexcept ToolError:\n    return "caught"', False),
    ('return "\\\"" * 40000', False),
    ('return "x" * (5 * 1024 * 1024)', False),
]:
    response = rpc("tools/call", {"name": "execute", "arguments": {"code": code}})
    assert bool(response.get("isError")) == should_fail, response
    execution = json.loads(response["content"][0]["text"])
    if "caught" in code:
        assert execution["result"] == "caught", execution
    if "40000" in code or "1024" in code:
        assert execution["resultTruncated"], execution
        assert len(json.dumps(execution["result"], ensure_ascii=False).encode()) <= 32768

created = rpc("tools/call", {"name": "execute", "arguments": {"code": "return 42"}}, tasks=True)
assert created["resultType"] == "task", created
assert created["ttlMs"] == 300000, created
until = time.monotonic() + 8
while True:
    task = rpc("tasks/get", {"taskId": created["taskId"]}, tasks=True)
    if task["status"] != "working":
        break
    assert time.monotonic() < until, "task did not complete within smoke deadline"
    time.sleep(0.1)
assert task["status"] == "completed", task
result = json.loads(task["result"]["content"][0]["text"])
assert result["status"] == "completed" and result["result"] == 42, result
print("PASS: both MCP eras, exactly three tools, auth, execution, errors, truncation and MCP task polling")
if len(rejections) == 2:
    print("PASS: loopback unknown-host rejection")
