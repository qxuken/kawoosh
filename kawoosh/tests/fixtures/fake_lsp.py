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
at start ("fake server starting"), which is the log's too. Round two
(roadmap step 7): `initialize` declares `.` as the completion trigger
and the rename, references, code action, formatting and type
definition providers; rename answers a WorkspaceEdit renaming the word
at the position and adding a `// renamed` line at the top; references
lists two places (0:0 and 1:4); a code action request offers "Add
semicolon" (an edit: `;` at the end of the line) and "Run the command"
(a bare command `fake.apply`, whose execution sends a
`workspace/applyEdit` putting `// applied` at the top); formatting is
one edit replacing the text with `// formatted` above it; the type
definition is line 0 of the same file. Round three (roadmap step 20):
implementation answers two LocationLinks (lines 0 and 1), declaration
one Location (0:3); documentSymbol a hierarchical `main` (a function,
0:3) holding `inner` (a variable, 1:4); workspace/symbol the symbols
`Widget` (a struct) and `widget_fn` whose names contain the query,
case aside, both in the file last opened, at 1:0 and 0:3; inlayHint a
string label `: i32` at 0:7 and a parts label `x` `y` at 1:0; the
hover's second paragraph names `Widget`."""
import json
import re, sys

docs = {}
ended = False
last_uri = None
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
        send({"jsonrpc": "2.0", "id": mid, "result": {"capabilities": {
            "completionProvider": {"triggerCharacters": ["."]},
            "renameProvider": True, "referencesProvider": True,
            "codeActionProvider": True, "documentFormattingProvider": True,
            "typeDefinitionProvider": True, "implementationProvider": True,
            "declarationProvider": True, "documentSymbolProvider": True,
            "workspaceSymbolProvider": True, "inlayHintProvider": True}}})
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
        last_uri = uri
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
        send({"jsonrpc": "2.0", "id": mid, "result": {"contents": {"kind": "markdown",
              "value": "the hover\n\nreturns Widget"}}})
    elif method == "textDocument/implementation":
        uri = m["params"]["textDocument"]["uri"]
        link = lambda ln: {"targetUri": uri,
                           "targetRange": {"start": {"line": ln, "character": 0}, "end": {"line": ln, "character": 2}},
                           "targetSelectionRange": {"start": {"line": ln, "character": 0}, "end": {"line": ln, "character": 2}}}
        send({"jsonrpc": "2.0", "id": mid, "result": [link(0), link(1)]})
    elif method == "textDocument/declaration":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": mid, "result": {"uri": uri, "range": {
            "start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}}})
    elif method == "textDocument/documentSymbol":
        r = lambda ln, ch: {"start": {"line": ln, "character": ch}, "end": {"line": ln, "character": ch + 4}}
        send({"jsonrpc": "2.0", "id": mid, "result": [
            {"name": "main", "kind": 12, "detail": "fn()", "range": r(0, 0), "selectionRange": r(0, 3),
             "children": [{"name": "inner", "kind": 13, "range": r(1, 4), "selectionRange": r(1, 4)}]}]})
    elif method == "workspace/symbol":
        q = m["params"]["query"].lower()
        loc = lambda ln, ch: {"uri": last_uri, "range": {"start": {"line": ln, "character": ch},
                                                          "end": {"line": ln, "character": ch + 1}}}
        found = [{"name": "Widget", "kind": 23, "location": loc(1, 0), "containerName": "crate"},
                 {"name": "widget_fn", "kind": 12, "location": loc(0, 3)}]
        send({"jsonrpc": "2.0", "id": mid, "result": [s for s in found if q in s["name"].lower()]})
    elif method == "textDocument/inlayHint":
        send({"jsonrpc": "2.0", "id": mid, "result": [
            {"position": {"line": 0, "character": 7}, "label": ": i32", "paddingLeft": False},
            {"position": {"line": 1, "character": 0}, "label": [{"value": "x"}, {"value": "y"}],
             "paddingRight": True}]})
    elif method == "textDocument/definition":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": mid, "result": [{"uri": uri, "range": {
            "start": {"line": 1, "character": 0}, "end": {"line": 1, "character": 2}}}]})
    elif method == "textDocument/rename":
        p = m["params"]
        uri = p["textDocument"]["uri"]
        lines = docs.get(uri, "").split("\n")
        ln, ch = p["position"]["line"], p["position"]["character"]
        line = lines[ln] if ln < len(lines) else ""
        a = ch
        while a > 0 and (line[a - 1].isalnum() or line[a - 1] == "_"):
            a -= 1
        b = ch
        while b < len(line) and (line[b].isalnum() or line[b] == "_"):
            b += 1
        word = line[a:b]
        changes = {uri: [
            {"range": {"start": {"line": ln, "character": a}, "end": {"line": ln, "character": b}},
             "newText": p["newName"]},
            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
             "newText": "// renamed\n"}]}
        # The word in every other document it was sent, as a server that
        # knows the workspace renames it there too — by the text it was
        # sent, so a document it was not told of an edit to is renamed
        # in the wrong place.
        for other, text in docs.items():
            if other == uri or not word:
                continue
            for i, l in enumerate(text.split("\n")):
                for mt in re.finditer(r"\b" + re.escape(word) + r"\b", l):
                    changes.setdefault(other, []).append(
                        {"range": {"start": {"line": i, "character": mt.start()},
                                   "end": {"line": i, "character": mt.end()}},
                         "newText": p["newName"]})
        send({"jsonrpc": "2.0", "id": mid, "result": {"changes": changes}})
    elif method == "textDocument/references":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": mid, "result": [
            {"uri": uri, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}}},
            {"uri": uri, "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 7}}}]})
    elif method == "textDocument/typeDefinition":
        uri = m["params"]["textDocument"]["uri"]
        send({"jsonrpc": "2.0", "id": mid, "result": {"uri": uri, "range": {
            "start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7}}}})
    elif method == "textDocument/codeAction":
        p = m["params"]
        uri = p["textDocument"]["uri"]
        ln = p["range"]["start"]["line"]
        lines = docs.get(uri, "").split("\n")
        end = len(lines[ln]) if ln < len(lines) else 0
        send({"jsonrpc": "2.0", "id": mid, "result": [
            {"title": "Add semicolon", "kind": "quickfix", "edit": {"changes": {uri: [
                {"range": {"start": {"line": ln, "character": end}, "end": {"line": ln, "character": end}},
                 "newText": ";"}]}}},
            {"title": "Run the command", "command": "fake.apply", "arguments": [uri]}]})
    elif method == "workspace/executeCommand":
        uri = m["params"]["arguments"][0]
        send({"jsonrpc": "2.0", "id": mid, "result": None})
        send({"jsonrpc": "2.0", "id": 2000, "method": "workspace/applyEdit", "params": {
            "label": "the command", "edit": {"documentChanges": [{"textDocument": {"uri": uri, "version": None},
                "edits": [{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                           "newText": "// applied\n"}]}]}}})
    elif method == "textDocument/formatting":
        uri = m["params"]["textDocument"]["uri"]
        text = docs.get(uri, "")
        lines = text.split("\n")
        send({"jsonrpc": "2.0", "id": mid, "result": [
            {"range": {"start": {"line": 0, "character": 0},
                       "end": {"line": len(lines) - 1, "character": len(lines[-1])}},
             "newText": "// formatted\n" + text}]})
    elif mid is not None:
        send({"jsonrpc": "2.0", "id": mid, "result": None})
