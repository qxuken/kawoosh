#!/usr/bin/env python3
"""A language server that speaks just enough JSON-RPC to test the pool:
initialize; a diagnostic on didOpen (the first word of line 0, message
"boom" — and on didChange only when the text has `!!` in it: an error on
every line, the cascade a half-typed line gets, so a test can watch it
wait for the typing to pause); completion
(hello_world, help — or, after a `.` in the text it was last sent,
member_a and member_b, so a test can see the request was made on the
synced text); hover ("the hover"); definition (line 1, character 0 of
the same file); progress: on `initialized` a work-done token
"Loading workspace" begins (after asking to create it) and reports
3/12 at 50%, and the first didChange ends it — and sends a
window/showMessage warning ("the warning") and a window/logMessage
("the log line"), so a test can see where each lands; a line on stderr
at start ("fake server starting"), which is the log's too."""
import json, sys

docs = {}
ended = False
print("fake server starting", file=sys.stderr, flush=True)

def char_before(uri, pos):
    lines = docs.get(uri, "").split("\n")
    line = lines[pos["line"]] if pos["line"] < len(lines) else ""
    c = pos["character"]
    return line[c - 1] if 0 < c <= len(line) else ""

def read():
    length = 0
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        line = line.strip()
        if not line:
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":")[1])
    return json.loads(sys.stdin.buffer.read(length))

def send(msg):
    body = json.dumps(msg).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()

while True:
    m = read()
    if m is None:
        break
    method = m.get("method")
    mid = m.get("id")
    if method is None:
        continue  # a response to a request of ours
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"capabilities": {}}})
    elif method == "initialized":
        send({"jsonrpc": "2.0", "id": 1000, "method": "window/workDoneProgress/create",
              "params": {"token": "ws"}})
        send({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": "ws", "value": {
            "kind": "begin", "title": "Loading workspace", "percentage": 0}}})
        send({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": "ws", "value": {
            "kind": "report", "message": "3/12", "percentage": 50}}})
    elif method == "textDocument/didChange":
        uri = m["params"]["textDocument"]["uri"]
        text = m["params"]["contentChanges"][0]["text"]
        docs[uri] = text
        if not ended:
            ended = True
            send({"jsonrpc": "2.0", "method": "$/progress", "params": {"token": "ws", "value": {
                "kind": "end"}}})
            send({"jsonrpc": "2.0", "method": "window/showMessage", "params": {
                "type": 2, "message": "the warning"}})
            send({"jsonrpc": "2.0", "method": "window/logMessage", "params": {
                "type": 4, "message": "the log line"}})
        if "!!" in text:
            send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
                "uri": uri,
                "diagnostics": [{"range": {"start": {"line": i, "character": 0}, "end": {"line": i, "character": 1}},
                                 "severity": 1, "message": "expected SEMICOLON"}
                                for i, line in enumerate(text.split("\n")) if line]}})
    elif method == "textDocument/didOpen":
        uri = m["params"]["textDocument"]["uri"]
        docs[uri] = m["params"]["textDocument"]["text"]
        send({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {
            "uri": uri,
            "diagnostics": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 3}},
                             "severity": 1, "message": "boom"}]}})
    elif method == "textDocument/completion":
        p = m["params"]
        if char_before(p["textDocument"]["uri"], p["position"]) == ".":
            items = [{"label": "member_a", "kind": 2}, {"label": "member_b", "kind": 2}]
        else:
            items = [{"label": "hello_world", "kind": 3, "insertText": "hello_world"},
                     {"label": "help", "kind": 3}]
        send({"jsonrpc": "2.0", "id": mid, "result": {"isIncomplete": False, "items": items}})
    elif method == "textDocument/hover":
        send({"jsonrpc": "2.0", "id": mid, "result": {"contents": {"kind": "markdown", "value": "the hover"}}})
    elif method == "textDocument/definition":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": mid, "result": [{"uri": uri, "range": {
            "start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 2}}}]})
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "result": None})
