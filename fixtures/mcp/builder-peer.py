# SPDX-License-Identifier: GPL-3.0-only
"""Offline MCP builder peer: deterministic protocol and host-visible request log."""
import json
import os
import sys
import time

log_path, behavior = sys.argv[1:]
with open(log_path + '.pid', 'w') as output:
    output.write(str(os.getpid()) + '\n')
for line in sys.stdin:
    request = json.loads(line)
    with open(log_path, 'a') as output:
        output.write(json.dumps(request) + '\n')
    method = request['method']
    if 'id' not in request:
        continue
    if method == 'initialize':
        if behavior == 'hang':
            time.sleep(60)
        if behavior == 'fail':
            print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32000, 'message': 'initialization failed'}}), flush=True)
            continue
        result = {'protocolVersion': '2025-03-26', 'capabilities': {'tools': {}, 'resources': {}}}
    elif method == 'tools/list':
        result = {'tools': [{'name': name, 'inputSchema': {'type': 'object'}, 'annotations': {'readOnlyHint': name != 'write'}} for name in ['a?b', 'a b', 'write']]}
    elif method == 'tools/call':
        result = {'content': [{'type': 'text', 'text': json.dumps(request['params'], sort_keys=True)}]}
    elif method == 'resources/list':
        result = {'resources': [{'name': 'allowed', 'uri': 'test://allowed'}]}
        if behavior == 'resources_many':
            result = {'resources': [{'name': str(index), 'uri': 'test://' + str(index)} for index in range(4)]}
    elif method == 'resources/read':
        result = {'contents': [{'uri': request['params']['uri'], 'text': 'resource'}]}
    else:
        result = {}
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
