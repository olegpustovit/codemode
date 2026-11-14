"""Disposable MCP stdio peer: only the configured environment may reach it."""
import json
import os
import sys

for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    if method == 'initialize':
        result = {'protocolVersion': '2025-11-25', 'capabilities': {'tools': {}}, 'serverInfo': {'name': 'fixture', 'version': '1'}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'probe', 'description': 'Probe the explicitly allowed environment', 'inputSchema': {'type': 'object', 'properties': {'value': {'type': 'string'}}, 'required': ['value']}, 'annotations': {'readOnlyHint': True}}]}
    elif method == 'tools/call':
        value = {'value': request['params']['arguments']['value'], 'environment': sorted(os.environ), 'linked': bool(os.environ.get('FIXTURE_SECRET'))}
        result = {'content': [{'type': 'text', 'text': json.dumps(value)}]}
    else:
        result = {}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
