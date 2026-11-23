#!/usr/bin/env python
"""Proof that a form of the tool server runs long calls as jobs: six calls against a real server process, each
answer checked, then that nothing is left running. For the packaged build before it is installed:

    python tests/long_calls_proof.py "build-out/Fridae Studios.app/Contents/Resources/tools/fridae-cli" --mcp
    python tests/long_calls_proof.py python3 rbxkit_mcp.py          (the source form)

It uses a temporary work folder of its own and nothing of the user's. Exit status 0 only if every check holds;
each difference is printed. (All of tests/test_long_calls.py runs against a build with
RBXKIT_TEST_SERVER="<the same command>".)"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import queue


def main(command):
    work = tempfile.mkdtemp(prefix="longcallsproof")
    env = {k: v for k, v in os.environ.items() if not k.startswith(("RBXKIT_", "FRIDAE_"))}
    env.update({"RBXKIT_CALL_BUDGET": "3", "RBXKIT_TEST_TOOLS": "1", "RBXKIT_WORK": work, "RBXKIT_AGENT_ID": "proof",
                "FRIDAE_TOOL_JOBS": os.path.join(work, "tool-jobs", "proof.jsonl")})
    p = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env)
    lines = queue.Queue()

    def read():
        for raw in p.stdout:
            try:
                lines.put(json.loads(raw))
            except ValueError:
                lines.put({"id": None, "not json": raw[:200].decode("utf-8", "replace")})
        lines.put(None)
    threading.Thread(target=read, daemon=True).start()
    wrong = []

    def check(ok, what, got=""):
        print(("ok    " if ok else "WRONG ") + what + ("" if ok else ": " + str(got)[:300]))
        if not ok:
            wrong.append(what)

    def ask(mid, method, params=None, wait=30):
        t0 = time.time()
        p.stdin.write((json.dumps({"jsonrpc": "2.0", "id": mid, "method": method, "params": params or {}}) + "\n").encode())
        p.stdin.flush()
        try:
            line = lines.get(timeout=wait)
        except queue.Empty:
            line = None
        if not line or line.get("id") != mid or "result" not in line:
            check(False, "call %d answered" % mid, line)
            return {"content": [{"type": "text", "text": ""}], "tools": []}, time.time() - t0
        return line["result"], time.time() - t0

    def call(mid, tool, **args):
        return ask(mid, "tools/call", {"name": tool, "arguments": args})

    def text(res):
        return res["content"][0].get("text", "")

    try:
        listed, _ = ask(1, "tools/list")
        names = [t["name"] for t in listed.get("tools", [])]
        check("rbx_job" in names and "rbx_test_slow" in names, "1: the tool list has rbx_job (and the stand-in)", names[-4:])
        r, took = call(2, "rbx_economy", example=True)
        check(text(r).startswith("Example inputs") and not r.get("isError"), "2: a real tool run as a job child returns its result", text(r))
        print("      (that job child took %.2f s from call to answer)" % took)
        r, took = call(3, "rbx_test_slow", seconds=8, word="marble")
        check(text(r).startswith("NOT FINISHED YET") and took < 6, "3: a call longer than the budget answers NOT FINISHED YET in time", (took, text(r)))
        r, _ = call(4, "rbx_job", action="wait")
        check(text(r).startswith("NOT FINISHED YET"), "4: the wait runs out of time again", text(r))
        while text(r).startswith("NOT FINISHED YET") and not wrong:
            r, _ = call(5, "rbx_job", action="wait")
        check("MARBLE" in text(r) and [c["type"] for c in r["content"]] == ["text", "image"], "5: the wait returns the tool's own result, with its picture", text(r))
        r, _ = call(6, "rbx_test_slow", seconds=120)
        check(text(r).startswith("NOT FINISHED YET"), "6: a job is left running", text(r))
        children = subprocess.run(["pgrep", "-P", str(p.pid)], capture_output=True, text=True).stdout.split() if os.name == "posix" else []
        check(os.name != "posix" or len(children) >= 2, "   the job and the watcher are the server's children", children)
        p.stdin.close()
        try:
            p.wait(15)
        except subprocess.TimeoutExpired:
            pass
        check(p.poll() is not None, "   the server ends when its input closes")
        time.sleep(1.5)
        left = [c for c in children if subprocess.run(["ps", "-o", "stat=", "-p", c], capture_output=True, text=True).stdout.strip()[:1] not in ("", "Z")]
        check(not left, "   nothing of it is left running", left)
    finally:
        if p.poll() is None:
            p.kill()
        shutil.rmtree(work, True)
    print("PROOF HOLDS" if not wrong else "PROOF FAILED: %d difference(s)" % len(wrong))
    return 1 if wrong else 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    sys.exit(main(sys.argv[1:]))
