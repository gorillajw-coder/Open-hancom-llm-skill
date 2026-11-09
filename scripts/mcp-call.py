#!/usr/bin/env python3 -I
"""Drive `oh mcp` over stdio. usage: mcp-call.py OH LIBRARY WORKDIR CALLS.json
CALLS.json: [{"tool": name, "args": {...}}, ...]; "$case" / "$draft" in string args are replaced
by the last case_id / draft_id. Prints one JSON result per call."""
import sys, json, subprocess
oh, lib, wd, calls = sys.argv[1:5]
p = subprocess.Popen([oh, "mcp", "--library", lib, "--workdir", wd], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
def rpc(i, method, params):
    p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": i, "method": method, "params": params}) + "\n"); p.stdin.flush()
    return json.loads(p.stdout.readline())
rpc(0, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "mcp-call", "version": "0"}})
p.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n"); p.stdin.flush()
state = {}
def sub(v):
    if isinstance(v, str):
        for k, x in state.items(): v = v.replace("$" + k, x)
        return v
    if isinstance(v, list): return [sub(x) for x in v]
    if isinstance(v, dict): return {k: sub(x) for k, x in v.items()}
    return v
for i, c in enumerate(json.load(open(calls)), 1):
    r = rpc(i, "tools/call", {"name": c["tool"], "arguments": sub(c["args"])})
    try:
        body = json.loads(r["result"]["content"][0]["text"])
    except Exception:
        body = r
    for k in ("case_id", "draft_id"):
        if isinstance(body, dict) and k in body: state[k.split("_")[0]] = body[k]
    print(json.dumps({"tool": c["tool"], "result": body}, ensure_ascii=False))
p.stdin.close(); p.wait()
