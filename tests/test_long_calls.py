"""Long tool calls (docs/superpowers/specs/2026-11-22-long-tool-calls-design.md): with a call budget set
(RBXKIT_CALL_BUDGET, by the app, for an AI that cuts tool calls off), a call that has not finished in time answers
"not finished yet" with a job name, and rbx_job picks the result up. With no budget nothing changes.

Every test here talks to a real tool server process over its pipes, as an AI does. The slow tool is the stand-in the
server offers only under RBXKIT_TEST_TOOLS=1.

RBXKIT_TEST_SERVER="<command>" runs them all against another form of the server, such as the packaged one
("/path/to/fridae-cli --mcp"); by default it is this checkout's rbxkit_mcp.py under the Python running the tests."""
import importlib.util
import json
import os
import queue
import re
import shlex
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
SERVER = shlex.split(os.environ.get("RBXKIT_TEST_SERVER") or "") or [sys.executable, os.path.join(KIT, "rbxkit_mcp.py")]
FROM_SOURCE = not os.environ.get("RBXKIT_TEST_SERVER")
CAN_DRAW = FROM_SOURCE and all(importlib.util.find_spec(m) for m in ("numpy", "PIL"))


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
        self.p = subprocess.Popen(SERVER, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=e)
        self.replies, self.next = {}, 0
        self.lines = queue.Queue()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for raw in self.p.stdout:
            self.lines.put(json.loads(raw))
        self.lines.put(None)

    def write(self, msg):
        self.p.stdin.write((json.dumps(msg) + "\n").encode())
        self.p.stdin.flush()

    def send(self, method, params=None):
        self.next += 1
        self.write({"jsonrpc": "2.0", "id": self.next, "method": method, "params": params or {}})
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

    def collect(self, job=None, timeout=60):
        """rbx_job wait, again for as long as it answers 'not finished yet'."""
        until = time.time() + timeout
        while True:
            res = self.call("rbx_job", action="wait", **({"job": job} if job else {}))
            if not text(res).startswith("NOT FINISHED YET"):
                return res
            assert time.time() < until, text(res)

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


def until(what, within=10.0):
    """Wait for a condition instead of sleeping for a guessed time."""
    end = time.time() + within
    while time.time() < end:
        got = what()
        if got:
            return got
        time.sleep(0.05)
    return what()


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
        end = time.time() + within
        while time.time() < end:
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
        self.assertIn("PLUM", text(r))
        self.assertEqual([c["type"] for c in r["content"]], ["text", "image"])
        self.assertFalse(r["isError"])

    def test_with_a_budget_a_quick_call_is_exactly_what_it_is_without_one(self):
        a, b = self.server(), self.server(budget=20)
        self.assertEqual(a.call("rbx_test_slow", seconds=0, word="kiwi"), b.call("rbx_test_slow", seconds=0, word="kiwi"))
        self.assertEqual(a.call("rbx_guide"), b.call("rbx_guide"))
        self.assertEqual(a.call("rbx_studio_lock", action="status"), b.call("rbx_studio_lock", action="status"))
        self.assertEqual(a.call("rbx_board", action="list"), b.call("rbx_board", action="list"))
        # real tools as jobs: one that only computes, with its example and with a real simulation
        self.assertEqual(a.call("rbx_economy", example=True), b.call("rbx_economy", example=True))
        economy = {"income": {"per_second": 2}, "upgrades": [{"name": "Speed", "base_cost": 10, "growth": {"type": "exponential", "rate": 1.5},
                                                              "max_level": 12, "effect": {"type": "multiply", "factor": 1.3}}]}
        sim = a.call("rbx_economy", config=economy)
        self.assertFalse(sim["isError"], text(sim))
        self.assertEqual(sim, b.call("rbx_economy", config=economy))
        # an unknown tool is the same protocol error
        ea, eb = a.reply(a.start("rbx_nope")), b.reply(b.start("rbx_nope"))
        self.assertEqual(ea["error"], eb["error"])
        # a wait with a timeout that is no number is the same tool error, not a protocol error
        ta, tb = a.reply(a.start("rbx_team", action="wait", ids=["w1"], timeout="soon")), b.reply(b.start("rbx_team", action="wait", ids=["w1"], timeout="soon"))
        self.assertEqual(ta["result"], tb["result"])
        self.assertTrue(tb["result"]["isError"])
        self.assertIn("rbx_job", b.tools())
        self.assertEqual([t for t in b.tools() if t != "rbx_job"], a.tools())

    # -------------------------------------------------------------- a job
    def test_a_slow_call_answers_in_time_and_the_job_gives_the_result_the_tool_would_have(self):
        notes = os.path.join(self.tmp, "pictures.jsonl")
        direct = self.server().call("rbx_test_slow", seconds=3, word="pear")
        s = self.server(budget=1, FRIDAE_TOOL_PICTURES=notes)
        t0 = time.time()
        r = s.call("rbx_test_slow", seconds=3, word="pear")
        self.assertLess(time.time() - t0, 3.0, "answered before the tool was done: at the budget")
        self.assertFalse(r["isError"], "not a failure")
        self.assertEqual(len(r["content"]), 1)
        self.assertTrue(text(r).startswith("NOT FINISHED YET"), text(r))
        self.assertNotIn("PEAR", text(r))
        self.assertIn("rbx_job", text(r))
        self.assertFalse(os.path.exists(notes), "no picture is noted before the result is given")
        job = job_of(r)
        # two waits in a row: the first runs out of time again, the second has the result
        again = s.call("rbx_job", action="wait", job=job)
        self.assertTrue(text(again).startswith("NOT FINISHED YET"), text(again))
        self.assertEqual(job_of(again), job)
        done = s.collect(job)
        self.assertEqual(done, direct, "text and picture exactly as the tool returns them")
        with open(notes, encoding="utf-8") as f:
            noted = [json.loads(line) for line in f]
        self.assertEqual(len(noted), 1)
        self.assertTrue(noted[0]["images"][0].endswith("pear_check.png"))
        # given once; a job that is gone is said to be gone, never "not finished yet"
        late = s.call("rbx_job", action="wait", job=job)
        self.assertTrue(late["isError"])
        self.assertIn("No job", text(late))
        self.assertNotIn("NOT FINISHED YET", text(late))
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))

    def test_wait_without_a_name_takes_the_only_job_and_list_shows_it(self):
        s = self.server(budget=0.5)
        r = s.call("rbx_test_slow", seconds=1.5, word="fig")
        listed = text(s.call("rbx_job", action="list"))
        self.assertIn(job_of(r), listed)
        self.assertIn("rbx_test_slow", listed)
        self.assertIn("FIG", text(s.collect()))

    def test_a_failing_job_reports_its_error_once(self):
        direct = self.server().call("rbx_test_slow", seconds=1.2, fail=True)
        self.assertTrue(direct["isError"])
        s = self.server(budget=0.5)
        job = job_of(s.call("rbx_test_slow", seconds=1.2, fail=True))
        self.assertEqual(s.collect(job), direct)
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

    # ------------------------------------------------ a job's longest life
    @unittest.skipUnless(POSIX, "process groups")
    def test_a_job_that_hangs_is_ended_at_its_longest_life_and_says_so_once(self):
        # unattended: nobody cancels; the caller only ever does what it was told, wait again
        s = self.server(budget=0.5)
        pidfile = os.path.join(self.tmp, "child.pid")
        r = s.call("rbx_test_slow", seconds=3600, life=2, child_pidfile=pidfile)
        child = self.pid_from(pidfile)
        t0 = time.time()
        res = s.collect(job_of(r), timeout=20)
        self.assertLess(time.time() - t0, 10, "the waiting ends by itself")
        self.assertTrue(res["isError"], "plainly a failure")
        self.assertIn("was ended", text(res))
        self.assertIn("Do not wait for it again", text(res))
        self.assertTrue(gone(child), "with everything it started, also what ignores SIGTERM")
        again = s.call("rbx_job", action="wait", job=job_of(r))
        self.assertTrue(again["isError"])
        self.assertIn("No job", text(again))
        self.assertNotIn("NOT FINISHED YET", text(again))
        # the next job in line then runs
        self.assertIn("NEXT", text(s.call("rbx_test_slow", seconds=0, word="next")))

    @unittest.skipUnless(FROM_SOURCE, "reads the module")
    def test_a_jobs_longest_life_is_its_tools_own_limit_and_never_over_fifteen_minutes(self):
        import rbxkit_mcp as m
        self.assertEqual(m.JOB_LIFE, 900)
        self.assertEqual(m._job_life("rbx_project", {}), 900, "no limit of its own: the 15 minutes a process per message gives")
        self.assertEqual(m._job_life("rbx_economy", {}), 900)
        self.assertEqual(m._job_life("rbx_blender", {}), 620, "its own 600 seconds, and time to say so itself")
        self.assertEqual(m._job_life("rbx_blender", {"timeout": 60}), 80)
        self.assertEqual(m._job_life("rbx_blender", {"timeout": 999999}), 900, "whatever the model asks for")
        self.assertEqual(m._job_life("rbx_blender", {"timeout": "long"}), 620)
        self.assertEqual(m._job_life("rbx_generate_texture", {}), 740)
        self.assertEqual(m._job_life("rbx_doctor", {}), 140)
        self.assertTrue(all(m._job_life(name, {}) <= 900 for name, _, _, _ in m.TOOLS))

    # ------------------------------------- what a job leaves behind goes too
    @unittest.skipUnless(POSIX, "process groups")
    def test_what_a_job_started_and_left_running_is_ended_when_the_job_ends_by_itself(self):
        # the tool returns (as a tool does whose own time limit ran out on a program it started) and its child lives on
        s = self.server(budget=10)
        pidfile = os.path.join(self.tmp, "child.pid")
        r = s.call("rbx_test_slow", seconds=0.3, word="left", child_pidfile=pidfile)
        self.assertIn("LEFT", text(r))
        self.assertFalse(r["isError"], "the tool's own result is given as it is")
        child = self.pid_from(pidfile)
        self.assertFalse(alive(child), "gone by the time the result is given")
        # and the same when the result is picked up later
        os.remove(pidfile)
        s2 = self.server(budget=0.3)
        job = job_of(s2.call("rbx_test_slow", seconds=1.2, word="later", child_pidfile=pidfile))
        child = self.pid_from(pidfile)
        self.assertIn("LATER", text(s2.collect(job)))
        self.assertFalse(alive(child))

    # --------------------------------------------------- one at a time
    def test_jobs_run_one_at_a_time_in_the_order_asked_and_the_second_names_the_job_it_waits_behind(self):
        s = self.server(budget=0.6)
        log = os.path.join(self.tmp, "lane.log")
        # sent together, as an AI sends two calls at once
        one = s.start("rbx_test_slow", seconds=2, word="one", log=log)
        two = s.start("rbx_test_slow", seconds=0.2, word="two", log=log)
        first, second = s.reply(one)["result"], s.reply(two)["result"]
        self.assertTrue(text(first).startswith("NOT FINISHED YET"), text(first))
        self.assertTrue(text(second).startswith("NOT FINISHED YET"), text(second))
        self.assertIn("queued behind", text(second))
        self.assertIn(job_of(first), text(second), "names the job it waits behind")
        self.assertNotEqual(job_of(second), job_of(first))
        self.assertIn("queued behind", text(s.call("rbx_job", action="list")))
        # the few tools that touch no shared file are not held up: the stand-in told to be one, and a real one
        self.assertIn("FREE", text(s.call("rbx_test_slow", seconds=0, word="free", beside=True)))
        self.assertIn("Example inputs", text(s.call("rbx_economy", example=True)))
        with open(log) as f:
            self.assertEqual(f.read().split(), ["start:one"], "the second has not started while the first runs")
        self.assertIn("TWO", text(s.collect(job_of(second))))
        with open(log) as f:
            self.assertEqual(f.read().split(), ["start:one", "end:one", "start:two", "end:two"], "one at a time, in order")
        self.assertIn("ONE", text(s.collect(job_of(first))))

    @unittest.skipUnless(FROM_SOURCE, "reads the module")
    def test_every_tool_takes_its_turn_but_the_listed_few(self):
        import rbxkit_mcp as m
        self.assertEqual(sorted(m.BESIDE), ["rbx_doctor", "rbx_economy", "rbx_market", "rbx_research"])
        self.assertTrue(all(len(why) > 10 for why in m.BESIDE.values()), "each with a line of why")
        for name, _, _, _ in m.TOOLS:
            if name not in m.BESIDE:
                self.assertFalse(m._beside(name, {}), name)
        for name in ("rbx_texture", "rbx_check_images", "rbx_generate_texture", "rbx_blender", "rbx_project", "rbx_icon", "rbx_vfx_package", "rbx_stage", "rbx_vault"):
            self.assertTrue(m._Job(name, {}).queued, name)

    @unittest.skipUnless(CAN_DRAW, "needs numpy and Pillow, and the server from source")
    def test_two_real_texture_calls_on_one_folder_sent_together_run_one_after_the_other(self):
        # rbx_texture needs neither Blender, Roblox Studio nor an AI. Both calls write <folder>/_check.png and attach it
        code = 'rbx.draw_texture("%s", lambda size: rbx.glow(*rbx.tex_coords(size), %s, 2.0), size=64)'
        # (a work folder with plain letters: a tool's result writes other letters as escapes, and the server then
        # does not find the picture it names, with or without a budget)
        plain = os.path.realpath(tempfile.mkdtemp(prefix="longcalls"))
        self.addCleanup(shutil.rmtree, plain, True)
        self.work = os.path.join(plain, "work")
        alone = Server(os.path.join(plain, "alone"))
        self.servers.append(alone)
        expected = {n: alone.call("rbx_texture", code=code % (n, r), folder=n, no_preview=True)["content"][1]["data"] for n, r in (("small", "0.15"), ("big", "0.45"))}
        self.assertNotEqual(expected["small"], expected["big"])
        s = self.server(budget=0.05)
        a = s.start("rbx_texture", code=code % ("small", "0.15"), folder="same", no_preview=True)
        b = s.start("rbx_texture", code=code % ("big", "0.45"), folder="same", no_preview=True)
        ra, rb = s.reply(a)["result"], s.reply(b)["result"]
        self.assertIn("is still working", text(ra))
        self.assertIn("queued behind " + job_of(ra), text(rb), "the second waits for the first")
        got_a, got_b = s.collect(job_of(ra)), s.collect(job_of(rb))
        for got in (got_a, got_b):
            self.assertFalse(got["isError"], text(got))
            self.assertEqual([c["type"] for c in got["content"]], ["text", "image", "image"], "the sheet to look at, then the texture")
        # each call's sheet is its own, not the other's or a half-written one
        self.assertEqual(got_a["content"][1]["data"], expected["small"])
        self.assertEqual(got_b["content"][1]["data"], expected["big"])

    # ------------------------------------------------- waits need no job
    def test_a_team_wait_returns_inside_the_budget_and_says_to_wait_again(self):
        s = self.server(budget=4)
        t0 = time.time()
        r = s.call("rbx_team", action="wait", ids=["w1"], timeout=600)
        self.assertLess(time.time() - t0, 4.0)
        self.assertIn("Call wait again", text(r))
        self.assertNotIn("NOT FINISHED YET", text(r))
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))

    # ------------------------------------------ a call the client gave up
    def test_a_call_the_client_cancelled_answers_nothing_and_spends_no_result(self):
        s = self.server(budget=20)
        call = s.start("rbx_test_slow", seconds=1.5, word="kept")
        self.assertTrue(until(lambda: "rbx_test_slow" in text(s.call("rbx_job", action="list"))))
        s.write({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": call, "reason": "timed out"}})
        res = s.collect()
        self.assertIn("KEPT", text(res), "the result went to the call that asked for it, not to the one given up")
        with self.assertRaises(queue.Empty):
            s.reply(call, timeout=1.0)

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
            self.assertNotIn("MINE", text(r))
        self.assertNotIn(ja, text(b.call("rbx_job", action="list")))
        self.assertIn(ja, text(a.call("rbx_job", action="list")), "still there for its own chat")

    # --------------------------------------------- the notes for the app
    def test_jobs_are_noted_for_the_app_and_the_turns_end_drops_them(self):
        notes = os.path.join(self.work, "tool-jobs", "c1.jsonl")
        os.makedirs(os.path.dirname(notes))
        # a line the app left for an earlier server that never read it: not this server's business
        with open(notes, "w", encoding="utf-8") as f:
            f.write(json.dumps({"turn": "ended", "at": time.time() + 3600}) + "\n")
        s = self.server(budget=0.5, FRIDAE_TOOL_JOBS=notes)
        pidfile = os.path.join(self.tmp, "child.pid")
        quick = job_of(s.call("rbx_test_slow", seconds=1, word="quick"))
        s.collect(quick)
        slow = job_of(s.call("rbx_test_slow", seconds=60, child_pidfile=pidfile))

        def lines():
            with open(notes, encoding="utf-8") as f:
                return [json.loads(line) for line in f]

        self.assertEqual(lines()[1:], [{"job": quick, "tool": "rbx_test_slow", "at": "started"}, {"job": quick, "at": "collected"},
                                       {"job": slow, "tool": "rbx_test_slow", "at": "started"}])
        child = self.pid_from(pidfile) if POSIX else None
        # the app says the turn is over: the job goes with all it started, and the server says so
        at = time.time()
        with open(notes, "a", encoding="utf-8") as f:
            f.write(json.dumps({"turn": "ended", "at": at}) + "\n")
        self.assertTrue(until(lambda: lines()[-1] == {"dropped": [slow], "at": at}), lines())
        if child:
            self.assertTrue(gone(child))
        self.assertIn("No jobs", text(s.call("rbx_job", action="list")))
        # the app empties the file; the next turn's jobs are noted and live. A line of an earlier turn that the
        # server reads only now (the app left it for a server that was slow) ends nothing of this turn
        open(notes, "w").close()
        later = job_of(s.call("rbx_test_slow", seconds=1.5, word="later"))
        with open(notes, "a", encoding="utf-8") as f:
            f.write(json.dumps({"turn": "ended", "at": at + 0.001}) + "\n")
        self.assertTrue(until(lambda: {"dropped": [], "at": at + 0.001} in lines()), lines())
        self.assertIn("LATER", text(s.collect(later)))

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
