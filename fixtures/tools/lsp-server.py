#!/usr/bin/python3
"""Read-only stdio fixture; all configuration is supplied by the test host."""
import json
import os
import sys
import time

mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
opened = None
opens = 0
closes = 0
root = ""


def send(message):
    body = json.dumps(message, ensure_ascii=False).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def reply(request, result):
    send({"jsonrpc": "2.0", "id": request["id"], "result": result})


def read():
    size = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            raise EOFError()
        if line in (b"\r\n", b"\n"):
            break
        name, value = line.split(b":", 1)
        if name.lower() == b"content-length":
            size = int(value)
    return json.loads(sys.stdin.buffer.read(size))


r = {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}}
while True:
    try:
        request = read()
    except EOFError:
        break
    method = request.get("method")
    params = request.get("params", {})
    if method == "initialize":
        root = params["rootUri"]
        assert params["capabilities"]["general"]["positionEncodings"] == ["utf-16"]
        assert params["processId"] is None
        assert len(params["workspaceFolders"]) == 1
        assert os.environ["LANG"] == "C"
        if mode == "startup-timeout":
            time.sleep(30)
        if mode == "stderr":
            sys.stderr.write("fixture-error" * 1000)
            sys.stderr.flush()
            sys.stdout.buffer.write(b"bad header\r\n\r\n")
            sys.stdout.buffer.flush()
            continue
        if mode == "bad-frame":
            sys.stdout.buffer.write(b"Content-Length: 999999999\r\n\r\n")
            sys.stdout.buffer.flush()
            continue
        if mode == "flood":
            sys.stdout.buffer.write(b"x" * (4 << 20))
            sys.stdout.buffer.flush()
            continue
        capabilities = {"positionEncoding": "utf-8" if mode == "encoding" else "utf-16"}
        if mode not in ("push", "stale"):
            capabilities["diagnosticProvider"] = True
        reply(request, {"capabilities": capabilities})
    elif method == "initialized":
        pass
    elif method == "textDocument/didOpen":
        assert opened is None, "didClose was omitted"
        opened = params["textDocument"]
        opens += 1
        assert opened["version"] == 1
        assert opened["languageId"] == "plain"
        if mode in ("push", "stale", "fallback"):
            send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                "uri": opened["uri"], "version": 0, "diagnostics": [{"range": r, "message": "stale"}]}})
            if mode != "stale":
                send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                    "uri": opened["uri"], "version": 1, "diagnostics": [{"range": r, "code": 42, "message": "published"}]}})
    elif method == "textDocument/didClose":
        assert opened is not None
        opened = None
        closes += 1
    elif method == "workspace/symbol":
        assert opened is None
        query = params["query"]
        if query == "hang":
            time.sleep(30)
        if query == "edits":
            send({"jsonrpc": "2.0", "id": "edit", "method": "workspace/applyEdit", "params": {"edit": {}}})
            denied = read()
            assert denied["result"]["applied"] is False
            send({"jsonrpc": "2.0", "id": "config", "method": "workspace/configuration", "params": {"items": []}})
            assert read()["result"] == []
            send({"jsonrpc": "2.0", "id": "unknown", "method": "client/unknown", "params": {}})
            assert read()["error"]["code"] == -32601
            try:
                with open("forbidden-write", "w") as target:
                    target.write("bad")
            except OSError:
                pass
            else:
                raise RuntimeError("LSP server not confined read-only")
        if query == "queue":
            for index in range(64):
                send({"jsonrpc": "2.0", "id": -index - 1, "result": None})
        reply(request, [{"name": "inside", "kind": 12, "containerName": f"opens={opens};closes={closes}", "location": {"uri": root + "/sample.txt", "range": r}},
                        {"name": "outside", "kind": 12, "location": {"uri": "file:///outside", "range": r}}])
    elif method in ("textDocument/definition", "textDocument/references", "textDocument/implementation", "textDocument/typeDefinition"):
        assert opened is not None
        assert params["position"] == {"line": 0, "character": 3}
        if method == "textDocument/references":
            assert params["context"]["includeDeclaration"] is True
        reply(request, [{"targetUri": opened["uri"], "targetSelectionRange": r},
                        {"uri": "file:///outside", "range": r}])
    elif method == "textDocument/hover":
        assert opened is not None
        reply(request, {"contents": [{"language": "plain", "value": opened["text"]}, "hover docs"], "range": r})
    elif method == "textDocument/documentSymbol":
        reply(request, [{"name": "Outer", "kind": 12, "range": r, "selectionRange": r, "children": [
            {"name": "Inner", "kind": 13, "range": r, "selectionRange": r}]}])
    elif method == "textDocument/diagnostic":
        if mode == "fallback":
            send({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "no pull diagnostics"}})
        else:
            reply(request, {"kind": "full", "items": [{"range": r, "severity": 2, "code": "fake", "source": "fixture", "message": "diagnostic"}]})
    else:
        raise RuntimeError(f"unexpected LSP request: {request}")
