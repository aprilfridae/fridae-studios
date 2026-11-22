"""Long tool calls (docs/superpowers/specs/2026-11-22-long-tool-calls-design.md): with a call budget set
(RBXKIT_CALL_BUDGET, by the app, for an AI that cuts tool calls off), a call that has not finished in time answers
"not finished yet" with a job name, and rbx_job picks the result up. With no budget nothing changes.

Every test here talks to a real tool server process over its pipes, as an AI does. The slow tool is the stand-in the
server offers only under RBXKIT_TEST_TOOLS=1."""
import json
import os
import queue
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest

KIT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POSIX = os.name == "posix"


class Server:
    """One tool server process."""

    def __init__(self, work, budget=None, **env):
        e = {k: v for k, v in os.environ.items() if not k.startswith(("RBXKIT_", "FRIDAE_"))}
        e.update({"RBXKIT_WORK": work, "RBXKIT_AGENT_ID": "c1", "RBXKIT_TEST_TOOLS": "1"})
        if budget is not None:
            e["RBXKIT_CALL_BUDGET"] = str(budget)
        e.update({k: v for k, v in env.items() if v is not None})
        for k in [k for k, v in env.items() if v is None]:
            e.pop(k, None)
        self.p = subprocess.Popen([sys.executable, os.path.join(KIT, "rbxkit_mcp.py")], stdin=subprocess.PIPE,
                                  stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=e)
        self.replies, self.next = {}, 0
        self.lines = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.p.stdout:
            self.lines.put(json.loads(raw))
        self.lines.put(None)

    def send(self, method, params=None):
        self.next += 1
        self.p.stdin.write((json.dumps({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params or {}}) + "\n").encode())
        self.p.stdin.flush()
        return self.next

    def reply(self, mid, timeout=30):
        until = time.time() + timeout
        while mid not in self.replies:
            line = self.lines.get(timeout=max(0.01, until - time.time()))
            if line is None:
                raise AssertionError("the server ended")
            self.replies[line["id"]] = line
        return self.replies.pop(mid)

    def start(self, tool, **args):
        return self.send("tools/call", {"name": tool, "arguments": args})

    def call(self, tool, timeout=30, **args):
        r = self.reply(self.start(tool, **args), timeout)
        if "error" in r:
            raise AssertionError(r["error"])
        return r["result"]

    def tools(self):
        return [t["name"] for t in self.reply(self.send("tools/list"))["result"]["tools"]]

    def close(self):
        try:
            self.p.stdin.close()
        except OSError:
            pass
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            self.p.kill()
        self.p.stdout.close()


def text(res):
    return res["content"][0]["text"]


def job_of(res):
    """The job name a 'not finished yet' answer gives."""
    import re
    m = re.search(r'job "(job-[0-9a-f]+)"', text(res))
    assert m, text(res)
    return m.group(1)


def alive(pid):
    try:
        os.kill(pid, 0)
    except OSError:
        return False
    # a zombie is not running
    out = subprocess.run(["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return bool(out) and not out.startswith("Z")


def gone(pid, within=8.0):
    until = time.time() + within
    while time.time() < until:
        if not alive(pid):
            return True
        time.sleep(0.05)
    return False


class LongCalls(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="long calls é "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.work = os.path.join(self.tmp, "work")
        os.makedirs(self.work)
        self.servers = []
        self.addCleanup(lambda: [s.close() for s in self.servers])

    def server(self, budget=None, **env):
        s = Server(self.work, budget, **env)
        self.servers.append(s)
        return s

    def pid_from(self, path, within=10.0):
        until = time.time() + within
        while time.time() < until:
            try:
                with open(path) as f:
                    return int(f.read())
            except (OSError, ValueError):
                time.sleep(0.05)
        raise AssertionError("no pid in %s" % path)

    # ------------------------------------------------------------ no budget
    def test_without_a_budget_nothing_is_added_and_a_slow_call_just_takes_its_time(self):
        plain = Server(self.work, RBXKIT_TEST_TOOLS=None)
        self.servers.append(plain)
        self.assertNotIn("rbx_job", plain.tools())
        self.assertNotIn("rbx_test_slow", plain.tools())
        s = self.server()
        self.assertNotIn("rbx_job", s.tools())
        t0 = time.time()
        r = s.call("rbx_test_slow", seconds=1.2, word="plum")
        self.assertGreaterEqual(time.time() - t0, 1.2)
        self.assertIn("plum", text(r))
        self.assertEqual([c["type"] for c in r["content"]], ["text", "image"])
        self.assertFalse(r["isError"])

    def test_with_a_budget_a_quick_call_is_exactly_what_it_is_without_one(self):
        a, b = self.server(), self.server(budget=20)
        self.assertEqual(a.call("rbx_test_slow", seconds=0, word="kiwi"), b.call("rbx_test_slow", seconds=0, word="kiwi"))
        self.assertEqual(a.call("rbx_guide"), b.call("rbx_guide"))
        self.assertEqual(a.call("rbx_studio_lock", action="status"), b.call("rbx_studio_lock", action="status"))
        self.assertEqual(a.call("rbx_economy", example=True), b.call("rbx_economy", example=True))
        # an unknown tool is the same protocol error
        ea, eb = a.reply(a.start("rbx_nope")), b.reply(b.start("rbx_nope"))
        self.assertEqual(ea["error"], eb["error"])
        self.assertIn("rbx_job", b.tools())
        self.assertEqual([t for t in b.tools() if t != "rbx_job"], a.tools())

    # -------------------------------------------------------------- a job
    def test_a_slow_call_answers_in_time_and_the_job_gives_the_result_the_tool_would_have(self):
        notes = os.path.join(self.tmp, "pictures.jsonl")
        direct = self.server().call("rbx_test_slow", seconds=3, word="pear")
        s = self.server(budget=1, FRIDAE_TOOL_PICTURES=notes)
        t0 = time.time()
        r = s.call("rbx_test_slow", seconds=3, word="pear")
        self.assertLess(time.time() - t0, 2.5, "answered within the budget")
        self.assertFalse(r["isError"], "not a failure")
        self.assertEqual(len(r["content"]), 1)
        self.assertTrue(text(r).startswith("NOT FINISHED YET"), text(r))
        self.assertNotIn("pear", text(r))
        self.assertIn("rbx_job", text(r))
        self.assertFalse(os.path.exists(notes), "no picture is noted before the result is given")
        job = job_of(r)
        # two waits in a row: the first runs out of time again, the second has the result
        again = s.call("rbx_job", action="wait", job=job)
        self.assertTrue(text(again).startswith("NOT FINISHED YET"), text(again))
        self.assertEqual(job_of(again), job)
        done = s.call("rbx_job", action="wait", job=job)
        if text(done).startswith("NOT FINISHED YET"):
            done = s.call("rbx_job", action="wait", job=job)
        self.assertEqual(done, direct, "text and picture exactly as the tool returns them")
        with open(notes, encoding="utf-8") as f:
            noted = [json.loads(line) for line in f]
        self.assertEqual(len(noted), 1)
        self.assertTrue(noted[0]["images"][0].endswith("pear_check.png"))
        # given once
        late = s.call("rbx_job", action="wait", job=job)
        self.assertTrue(late["isError"])
        self.assertIn("No job", text(late))
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))

    def test_wait_without_a_name_takes_the_only_job_and_list_shows_it(self):
        s = self.server(budget=0.5)
        r = s.call("rbx_test_slow", seconds=1.5, word="fig")
        listed = text(s.call("rbx_job", action="list"))
        self.assertIn(job_of(r), listed)
        self.assertIn("rbx_test_slow", listed)
        res = s.call("rbx_job", action="wait")
        while text(res).startswith("NOT FINISHED YET"):
            res = s.call("rbx_job", action="wait")
        self.assertIn("fig", text(res))

    def test_a_failing_job_reports_its_error_once(self):
        direct = self.server().call("rbx_test_slow", seconds=1.2, fail=True)
        self.assertTrue(direct["isError"])
        s = self.server(budget=0.5)
        r = s.call("rbx_test_slow", seconds=1.2, fail=True)
        job = job_of(r)
        res = s.call("rbx_job", action="wait", job=job)
        while text(res).startswith("NOT FINISHED YET"):
            res = s.call("rbx_job", action="wait", job=job)
        self.assertEqual(res, direct)
        self.assertIn("No job", text(s.call("rbx_job", action="wait", job=job)))

    @unittest.skipUnless(POSIX, "process groups")
    def test_cancel_ends_the_job_and_a_child_that_ignores_sigterm(self):
        s = self.server(budget=0.5)
        pidfile = os.path.join(self.tmp, "child.pid")
        r = s.call("rbx_test_slow", seconds=60, child_pidfile=pidfile)
        child = self.pid_from(pidfile)
        self.assertTrue(alive(child))
        res = s.call("rbx_job", action="cancel", job=job_of(r))
        self.assertFalse(res["isError"])
        self.assertIn("Cancelled", text(res))
        self.assertTrue(gone(child), "the child that ignores SIGTERM is gone")
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))
        self.assertTrue(s.call("rbx_job", action="wait", job=job_of(r))["isError"])

    def test_work_that_must_not_overlap_is_queued_behind_the_named_job(self):
        s = self.server(budget=0.6)
        log = os.path.join(self.tmp, "lane.log")
        first = s.call("rbx_test_slow", seconds=2, word="one", lane="blender", log=log)
        t0 = time.time()
        second = s.call("rbx_test_slow", seconds=0.2, word="two", lane="blender", log=log)
        self.assertLess(time.time() - t0, 2.0)
        self.assertTrue(text(second).startswith("NOT FINISHED YET"), text(second))
        self.assertIn("queued behind", text(second))
        self.assertIn(job_of(first), text(second), "names the job it waits behind")
        own = [j for j in __import__("re").findall(r"job-[0-9a-f]+", text(second)) if j != job_of(first)]
        self.assertTrue(own, text(second))
        self.assertIn("queued behind", text(s.call("rbx_job", action="list")))
        # work with no lane is not held up by it
        t0 = time.time()
        self.assertIn("free", text(s.call("rbx_test_slow", seconds=0, word="free")))
        self.assertLess(time.time() - t0, 1.0)
        res = s.call("rbx_job", action="wait", job=own[0])
        while text(res).startswith("NOT FINISHED YET"):
            res = s.call("rbx_job", action="wait", job=own[0])
        self.assertIn("two", text(res))
        with open(log) as f:
            order = f.read().split()
        self.assertEqual(order, ["start:one", "end:one", "start:two", "end:two"], "one at a time, in order")

    # ------------------------------------------------- waits need no job
    def test_a_team_wait_returns_inside_the_budget_and_says_to_wait_again(self):
        s = self.server(budget=4)
        t0 = time.time()
        r = s.call("rbx_team", action="wait", ids=["w1"], timeout=600)
        self.assertLess(time.time() - t0, 4.0)
        self.assertIn("Call wait again", text(r))
        self.assertNotIn("NOT FINISHED YET", text(r))
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))

    # ----------------------------------------------------- chats are apart
    def test_one_chats_server_knows_nothing_of_anothers_jobs(self):
        a, b = self.server(budget=0.5), self.server(budget=0.5)
        ja = job_of(a.call("rbx_test_slow", seconds=5, word="mine"))
        jb = job_of(b.call("rbx_test_slow", seconds=5, word="theirs"))
        self.assertNotEqual(ja, jb)
        self.assertGreaterEqual(len(ja), len("job-") + 12, "long enough not to be guessed")
        for action in ("wait", "cancel"):
            r = b.call("rbx_job", action=action, job=ja)
            self.assertTrue(r["isError"], action)
            self.assertNotIn("mine", text(r))
        self.assertNotIn(ja, text(b.call("rbx_job", action="list")))
        self.assertIn(ja, text(a.call("rbx_job", action="list")), "still there for its own chat")

    # --------------------------------------------- the notes for the app
    def test_jobs_are_noted_for_the_app_and_the_turns_end_drops_them(self):
        notes = os.path.join(self.work, "tool-jobs", "c1.jsonl")
        s = self.server(budget=0.5, FRIDAE_TOOL_JOBS=notes)
        pidfile = os.path.join(self.tmp, "child.pid")
        quick = job_of(s.call("rbx_test_slow", seconds=1, word="quick"))
        res = s.call("rbx_job", action="wait", job=quick)
        while text(res).startswith("NOT FINISHED YET"):
            res = s.call("rbx_job", action="wait", job=quick)
        slow = job_of(s.call("rbx_test_slow", seconds=60, child_pidfile=pidfile))

        def lines():
            with open(notes, encoding="utf-8") as f:
                return [json.loads(line) for line in f]

        self.assertEqual(lines(), [{"job": quick, "tool": "rbx_test_slow", "at": "started"}, {"job": quick, "at": "collected"},
                                   {"job": slow, "tool": "rbx_test_slow", "at": "started"}])
        child = self.pid_from(pidfile) if POSIX else None
        # the app says the turn is over: the job goes with all it started, and the server says so
        with open(notes, "a", encoding="utf-8") as f:
            f.write(json.dumps({"turn": "ended"}) + "\n")
        until = time.time() + 8
        while time.time() < until and not any("dropped" in line for line in lines()):
            time.sleep(0.05)
        self.assertEqual(lines()[-1], {"dropped": [slow]})
        if child:
            self.assertTrue(gone(child))
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))
        # the app empties the file; the next turn's jobs are noted and live
        open(notes, "w").close()
        time.sleep(0.6)
        later = job_of(s.call("rbx_test_slow", seconds=1.5, word="later"))
        res = s.call("rbx_job", action="wait", job=later)
        while text(res).startswith("NOT FINISHED YET"):
            res = s.call("rbx_job", action="wait", job=later)
        self.assertIn("later", text(res))

    # ------------------------------------------- nothing outlives the server
    def ending(self, end):
        s = self.server(budget=0.5)
        pidfile = os.path.join(self.tmp, "child.pid")
        s.call("rbx_test_slow", seconds=120, child_pidfile=pidfile)
        child = self.pid_from(pidfile)
        self.assertTrue(alive(child))
        end(s)
        self.assertTrue(gone(child), "the job's child, which ignores SIGTERM, went with the server")
        s.p.wait(10)

    @unittest.skipUnless(POSIX, "process groups")
    def test_the_server_closing_takes_its_jobs_with_it(self):
        self.ending(lambda s: s.p.stdin.close())

    @unittest.skipUnless(POSIX, "process groups")
    def test_the_server_being_told_to_end_takes_its_jobs_with_it(self):
        self.ending(lambda s: s.p.send_signal(signal.SIGTERM))

    @unittest.skipUnless(POSIX, "process groups")
    def test_the_server_being_killed_outright_takes_its_jobs_with_it(self):
        self.ending(lambda s: s.p.kill())

    @unittest.skipUnless(POSIX, "process groups")
    def test_no_process_is_left_once_the_server_is_gone(self):
        s = self.server(budget=0.5)
        s.call("rbx_test_slow", seconds=120)
        mine = subprocess.run(["pgrep", "-P", str(s.p.pid)], capture_output=True, text=True).stdout.split()
        self.assertTrue(mine, "the job and the watcher are its children")
        s.p.stdin.close()
        s.p.wait(10)
        for pid in mine:
            self.assertTrue(gone(int(pid)), pid)


if __name__ == "__main__":
    unittest.main()
