#!/usr/bin/env python3
import json
import sys
import time
import urllib.error
import urllib.request

URL, TOKEN, TMP = sys.argv[1:4]
rid = 0

def rpc(method, params=None, token=TOKEN, ok=True):
    global rid
    rid += 1
    body = json.dumps({"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}}).encode()
    req = urllib.request.Request(URL, data=body, method="POST", headers={
        "Content-Type": "application/json",
        "Accept": "application/json, text/event-stream",
        "Authorization": f"Bearer {token}",
        "MCP-Protocol-Version": "2025-06-18",
    })
    try:
        data = urllib.request.urlopen(req, timeout=10).read()
    except urllib.error.HTTPError as e:
        if ok:
            raise
        return {"http_error": e.code, "body": e.read().decode()}
    resp = json.loads(data)
    if "error" in resp:
        if ok:
            raise AssertionError(resp["error"])
        return resp
    return resp["result"]

def call(name, args=None, ok=True):
    return rpc("tools/call", {"name": name, "arguments": args or {}}, ok=ok)

def structured(result):
    return result["structuredContent"]

assert rpc("initialize")["protocolVersion"] == "2025-06-18"
tools = rpc("tools/list")["tools"]
names = {t["name"] for t in tools}
for name in ["agentbox_exec_command", "agentbox_write_stdin", "agentbox_apply_patch", "agentbox_bootstrap", "agentbox_list_skills", "agentbox_load_skill"]:
    assert name in names, name
for t in tools:
    assert t["annotations"] == {"readOnlyHint": True, "destructiveHint": False, "openWorldHint": False}
    assert "outputSchema" in t, t["name"]

bad = rpc("initialize", token="wrong", ok=False)
assert bad["http_error"] == 401

assert "default_workdir" in structured(call("agentbox_bootstrap"))
skills = structured(call("agentbox_list_skills"))
body = json.dumps(skills)
assert "Rust maintainer" in body and "FULL RUST BODY" not in body
loaded = structured(call("agentbox_load_skill", {"skill": "rust-maintainer"}))
assert "FULL RUST BODY" in loaded["content"]

def run(cmd, **kw):
    return structured(call("agentbox_exec_command", {"cmd": cmd, **kw}))

def run_to_exit(cmd, **kw):
    out = run(cmd, **kw)
    while "session_id" in out and out.get("exit_code") is None:
        out = structured(call("agentbox_write_stdin", {"session_id": out["session_id"], "chars": "", "yield_time_ms": 1000}))
    return out

assert run("printf 'hello\\n'")["output"] == "hello\n"
assert run("echo before; exit 7")["exit_code"] == 7
assert "out" in run("echo out; echo err >&2")["output"]
pwd_out = run("pwd", workdir=f"{TMP}/work fixture")
assert pwd_out["output"].strip().endswith("work fixture"), pwd_out
assert "a'b\"c $HOME $(uname)" in run("cat <<'EOF'\na'b\"c $HOME $(uname)\nEOF")["output"]
assert "π" in run("printf 'π'")["output"]
assert run("printf nonewline")["output"] == "nonewline"
bad_utf = run("python3 -c 'import os; os.write(1, bytes([255]))'")
if "session_id" in bad_utf:
    bad_utf = structured(call("agentbox_write_stdin", {"session_id": bad_utf["session_id"], "chars": "", "yield_time_ms": 500}))
assert "\ufffd" in bad_utf["output"], bad_utf

weird = [
    "printf \"%s\" \"single ' inside\"",
    "printf '%s' 'double \" inside'",
    "printf '%s' '$HOME $(uname)'",
    "printf '%s' \"$(printf actual)\"",
    "printf a | tr a b",
    "printf err >&2",
    "mkdir -p glob && touch glob/a.txt && printf '%s' glob/*.txt",
    "printf '%s' 'glob/*.txt'",
    "cat <<EOF\n$HOME\nEOF",
    "python3 - <<'PY'\nprint('multi')\nPY",
    "mkdir -p 'path space' && touch 'path space/file' && test -f 'path space/file' && echo ok",
    "mkdir -p 'unicodé' && echo ok",
    "(echo sub)",
    "cat <(printf ps)",
    "FOO=bar sh -c 'echo $FOO'",
]
for cmd in weird:
    assert run(cmd)["exit_code"] == 0, cmd

long = run("sleep 0.5; echo done", yield_time_ms=50)
sid = long["session_id"]
while True:
    polled = structured(call("agentbox_write_stdin", {"session_id": sid, "chars": "", "yield_time_ms": 200}))
    if polled.get("exit_code") is not None:
        assert "done" in polled["output"]
        break

tty = run("read -p 'Name: ' name; echo \"Hello $name\"", tty=True, yield_time_ms=100)
sid = tty["session_id"]
reply = structured(call("agentbox_write_stdin", {"session_id": sid, "chars": "Ada\n", "yield_time_ms": 500}))
assert "Hello Ada" in reply["output"]

ctrl = run("python3 -c 'import signal,time,sys; signal.signal(signal.SIGINT, lambda s,f: (print(\"trapped\", flush=True), sys.exit(0))); [time.sleep(1) for _ in iter(int,1)]'", tty=True, yield_time_ms=50)
reply = structured(call("agentbox_write_stdin", {"session_id": ctrl["session_id"], "chars": "\u0003", "yield_time_ms": 1000}))
assert reply.get("exit_code") is not None and "^C" in reply["output"], reply

nt = run("sleep 1", tty=False, yield_time_ms=50)
assert "error" in call("agentbox_write_stdin", {"session_id": nt["session_id"], "chars": "x"}, ok=False)
assert "error" in call("agentbox_write_stdin", {"session_id": 999999, "chars": ""}, ok=False)

a = run("sleep 0.2; echo A", yield_time_ms=50)["session_id"]
b = run("sleep 0.2; echo B", yield_time_ms=50)["session_id"]
outs = []
for sid in [a, b]:
    while True:
        p = structured(call("agentbox_write_stdin", {"session_id": sid, "chars": "", "yield_time_ms": 300}))
        if p.get("exit_code") is not None:
            outs.append(p["output"])
            break
assert any("A" in o for o in outs) and any("B" in o for o in outs)

tr = run("python3 - <<'PY'\nprint('x'*2000)\nPY", max_output_tokens=20)
assert "agentbox output truncated" in tr["output"] and tr["original_token_count"] > 20

fail = run_to_exit("cargo test", workdir=f"{TMP}/fixture")
assert fail["exit_code"] != 0
patch = """*** Begin Patch
*** Update File: src/lib.rs
@@
-pub fn answer() -> i32 { 41 }
+pub fn answer() -> i32 { 42 }
*** End Patch
"""
patched = structured(call("agentbox_apply_patch", {"patch": patch, "workdir": f"{TMP}/fixture"}))
assert patched["status"] == "completed", patched
success = run_to_exit("cargo test", workdir=f"{TMP}/fixture")
assert success["exit_code"] == 0, success["output"]

print("smoke ok")
