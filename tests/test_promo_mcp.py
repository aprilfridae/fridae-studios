"""rbx_promo: the MCP tool, background render jobs, the CLI, and the rule that children never touch the protocol stdin."""
import ast, glob, io, json, os, subprocess, sys, time, unittest
from contextlib import redirect_stdout, redirect_stderr
from unittest import mock

import rbxkit_mcp
from rbxpromo import engine, reference, tool
from rbxpromo.recipe import RecipeError

KIT_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
READY = {"node": "n", "renderer": True, "browser": True, "whisper": True, "voice": True, "ready": True,
         "missing": [], "download_mb": 0}


def call(args):
    return rbxkit_mcp.handle({"method": "tools/call", "params": {"name": "rbx_promo", "arguments": args}})


class Capture(io.TextIOWrapper):
    """A stdout stand-in that rbx.main can reconfigure, like the real one."""
    def __init__(self):
        super().__init__(io.BytesIO(), encoding="utf-8", write_through=True)

    def getvalue(self):
        return self.buffer.getvalue().decode("utf-8")


def fake_clip(out_dir):
    """Stands in for reference.sample_clip where render_still is faked: no engine and no file needed."""
    return os.path.join(out_dir, "sample.mp4")


def text(res):
    return res["content"][0]["text"]


class ToolTests(unittest.TestCase):
    def setUp(self):
        rbxkit_mcp._JOBS.clear()
        self.addCleanup(rbxkit_mcp._JOBS.clear)

    def test_tools_list_has_promo(self):
        tools = {t["name"]: t for t in rbxkit_mcp.handle({"method": "tools/list"})["tools"]}
        t = tools["rbx_promo"]
        props = t["inputSchema"]["properties"]
        for a in ("status", "probe", "reference", "voice", "transcribe", "render", "check", "styles", "job"):
            self.assertIn(a, props["action"]["enum"])
        self.assertNotIn("install", props["action"]["enum"])
        for f in ("paths", "path", "recipe", "text", "voice", "out"):
            self.assertIn(f, props)
        self.assertIn("rbx_guide", t["description"])
        self.assertLess(len(t["description"]), 300)

    def test_promo_guide_loads(self):
        res = rbxkit_mcp.handle({"method": "tools/call", "params": {"name": "rbx_guide", "arguments": {"topics": ["promo"]}}})
        self.assertFalse(res["isError"])
        self.assertIn("Hook in the first second", text(res))

    def test_promo_needs_action(self):
        res = call({})
        self.assertTrue(res["isError"])
        self.assertIn("action", text(res))

    def test_unknown_action(self):
        self.assertTrue(call({"action": "bogus"})["isError"])

    def test_engine_missing_message(self):
        st = dict(READY, ready=False, missing=["renderer", "voice"], download_mb=492)
        with mock.patch.object(engine, "status", return_value=st):
            res = call({"action": "probe", "paths": ["a.mp4"]})
            self.assertTrue(res["isError"])
            self.assertIn("Install the promo engine in Setup", text(res))
            self.assertIn("renderer, voice", text(res))
            self.assertFalse(call({"action": "status"})["isError"])   # status works without the engine

    def test_install_points_to_setup(self):
        res = call({"action": "install"})
        self.assertTrue(res["isError"])
        self.assertIn("Setup", text(res))

    def test_probe_missing_clip_is_a_tool_error(self):
        with mock.patch.object(engine, "status", return_value=READY):
            res = call({"action": "probe", "paths": ["/no/such/clip.mp4"]})
        self.assertTrue(res["isError"])
        self.assertIn("not found", text(res))

    def test_fast_render_returns_the_result(self):
        fake = lambda args, progress: {"video": "x.mp4", "duration": 3}
        with mock.patch.object(engine, "status", return_value=READY), mock.patch.dict(tool.ACTIONS, {"render": fake}):
            res = call({"action": "render", "recipe": "r.json"})
        self.assertFalse(res["isError"])
        self.assertEqual(json.loads(text(res))["video"], "x.mp4")

    def test_render_returns_job_when_slow(self):
        def slow(args, progress):
            progress(0.5)
            time.sleep(2)
            return {"video": "x.mp4", "duration": 3}
        with mock.patch.object(engine, "status", return_value=READY), mock.patch.dict(tool.ACTIONS, {"render": slow}), \
                mock.patch.object(rbxkit_mcp, "PROMO_SYNC_SECONDS", 1), mock.patch.object(rbxkit_mcp, "PROMO_JOB_WAIT_SECONDS", 0.1):
            first = json.loads(text(call({"action": "render", "recipe": "r.json"})))
            self.assertEqual(first["state"], "running")
            self.assertEqual(len(first["job"]), 32)
            mid = json.loads(text(call({"action": "job", "id": first["job"]})))
            self.assertEqual(mid["state"], "running")
            self.assertEqual(mid["progress"], 0.5)
            time.sleep(1.5)
            done = call({"action": "job", "id": first["job"]})
        self.assertFalse(done["isError"])
        self.assertEqual(json.loads(text(done))["video"], "x.mp4")

    def test_job_call_waits_for_the_render_to_finish(self):
        def slow(args, progress):
            time.sleep(0.8)
            return {"video": "x.mp4", "duration": 3}
        with mock.patch.object(engine, "status", return_value=READY), mock.patch.dict(tool.ACTIONS, {"render": slow}), \
                mock.patch.object(rbxkit_mcp, "PROMO_SYNC_SECONDS", 0.05), mock.patch.object(rbxkit_mcp, "PROMO_JOB_WAIT_SECONDS", 10):
            first = json.loads(text(call({"action": "render", "recipe": "r.json"})))
            self.assertEqual(first["state"], "running")
            t0 = time.time()
            done = call({"action": "job", "id": first["job"]})      # no sleep: the call itself waits
        self.assertLess(time.time() - t0, 5)
        self.assertFalse(done["isError"])
        self.assertEqual(json.loads(text(done))["video"], "x.mp4")

    def test_job_wait_stays_under_the_mcp_timeout(self):
        self.assertTrue(30 <= rbxkit_mcp.PROMO_JOB_WAIT_SECONDS <= 60, rbxkit_mcp.PROMO_JOB_WAIT_SECONDS)

    def test_job_unknown_id(self):
        res = call({"action": "job", "id": "nope"})
        self.assertTrue(res["isError"])
        self.assertIn("No promo job", text(res))

    def test_job_error_lists_recipe_problems(self):
        def bad(args, progress):
            time.sleep(1.5)
            raise RecipeError(["scene 0: clip file not found: a.mp4", "'fps' must be 30 or 60."])
        with mock.patch.object(engine, "status", return_value=READY), mock.patch.dict(tool.ACTIONS, {"render": bad}), \
                mock.patch.object(rbxkit_mcp, "PROMO_SYNC_SECONDS", 1), mock.patch.object(rbxkit_mcp, "PROMO_JOB_WAIT_SECONDS", 0.1):
            jid = json.loads(text(call({"action": "render", "recipe": "r.json"})))["job"]
            time.sleep(1)
            res = call({"action": "job", "id": jid})
        self.assertTrue(res["isError"])
        self.assertIn("scene 0: clip file not found", text(res))
        self.assertIn("'fps' must be 30 or 60.", text(res))

    def test_job_crash_is_the_jobs_error_not_a_server_crash(self):
        def boom(args, progress):
            raise KeyError("oops")
        with mock.patch.object(engine, "status", return_value=READY), mock.patch.dict(tool.ACTIONS, {"render": boom}):
            res = call({"action": "render", "recipe": "r.json"})
        self.assertTrue(res["isError"])
        self.assertIn("oops", text(res))

    def test_read_jobs_are_forgotten_after_ttl_and_unread_ones_kept(self):
        read = rbxkit_mcp._start_promo_job(lambda a, p: {"ok": 1}, {})
        unread = rbxkit_mcp._start_promo_job(lambda a, p: {"ok": 1}, {})
        read["thread"].join(), unread["thread"].join()
        rbxkit_mcp._job_reply(read)
        with mock.patch.object(rbxkit_mcp, "PROMO_JOB_READ_TTL", -1):
            rbxkit_mcp._forget_old_jobs()
        self.assertNotIn(read["id"], rbxkit_mcp._JOBS)
        self.assertIn(unread["id"], rbxkit_mcp._JOBS)

    def test_corrupt_cached_still_is_rendered_again(self):
        import tempfile
        from PIL import Image
        from rbxpromo import render
        renders = []

        def fake_still(recipe_path, out_jpg, seconds):
            renders.append(out_jpg)
            Image.new("RGB", (108, 192), (90, 40, 160)).save(out_jpg)
            return out_jpg
        with tempfile.TemporaryDirectory() as out, mock.patch.object(engine, "promo_hash", return_value="b" * 40), \
                mock.patch.object(render, "render_still", fake_still), mock.patch.object(reference, "sample_clip", fake_clip):
            items = reference.styles(out)
            with open(items[0]["still"], "wb") as f:
                f.write(b"half a jpeg")
            n = len(renders)
            again = reference.styles(out)
            self.assertEqual(len(renders), n + 2)        # a still is two frames: the title card and the footage
            with Image.open(again[0]["still"]) as im:
                im.load()
            tool.styles_sheet(again, os.path.join(out, "sheet_styles.jpg"))
            self.assertEqual([f for f in os.listdir(out) if "part" in f or "tmp" in f], [])

    def test_failed_still_render_leaves_no_file(self):
        import tempfile
        from rbxpromo import render

        def broken(recipe_path, out_jpg, seconds):
            with open(out_jpg, "wb") as f:
                f.write(b"partial")
            raise engine.EngineError("renderer died")
        with tempfile.TemporaryDirectory() as out, mock.patch.object(engine, "promo_hash", return_value="c" * 40), \
                mock.patch.object(render, "render_still", broken), mock.patch.object(reference, "sample_clip", fake_clip):
            with self.assertRaises(engine.EngineError):
                reference.styles(out)
            self.assertEqual(os.listdir(out), [])

    def test_styles_reuses_stills_and_returns_one_sheet_image(self):
        import tempfile
        from PIL import Image
        from rbxpromo import render
        renders = []

        def fake_still(recipe_path, out_jpg, seconds):
            renders.append(out_jpg)
            Image.new("RGB", (1080, 1920), (90, 40, 160)).save(out_jpg)
            return out_jpg
        with tempfile.TemporaryDirectory() as work, mock.patch.object(engine, "status", return_value=READY), \
                mock.patch.object(engine, "promo_hash", return_value="a" * 40), \
                mock.patch("rbxpaths.WORK", work), mock.patch.object(render, "render_still", fake_still), \
                mock.patch.object(reference, "sample_clip", fake_clip):
            first = call({"action": "styles"})
            n = len(renders)
            second = call({"action": "styles"})
        self.assertGreaterEqual(n, 16)                  # two frames per style: the title card and the footage
        self.assertEqual(len(renders), n, "the second call must not render again")
        for res in (first, second):
            self.assertFalse(res["isError"])
            self.assertEqual([c["type"] for c in res["content"]], ["text", "image"])
            data = json.loads(text(res))
            self.assertEqual(len(data["styles"]) * 2, n)
            self.assertTrue(data["sheet"].endswith("sheet_styles.jpg"))


class ProtocolStdoutTests(unittest.TestCase):
    """The server's stdout carries only JSON-RPC, even while a job thread prints and runs a child that writes to fd 1."""

    SCRIPT = r"""
import os, subprocess, sys, time
sys.path.insert(0, %(kit)r)
import rbxkit_mcp
from rbxpromo import engine, tool
engine.status = lambda: {"ready": True, "missing": [], "download_mb": 0}
def noisy(args, progress):
    for i in range(20):
        print("python print", i)
        os.write(1, b"raw fd write\n")
        subprocess.run([sys.executable, "-c", "print('child stdout')"], stdin=subprocess.DEVNULL)
        time.sleep(0.05)
    return {"ok": 1}
tool.ACTIONS["render"] = noisy
rbxkit_mcp.PROMO_SYNC_SECONDS = 0.1
rbxkit_mcp.main()
"""

    def test_protocol_stream_has_only_jsonrpc_lines(self):
        reqs = [{"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                 "params": {"name": "rbx_promo", "arguments": {"action": "render", "recipe": "r.json"}}}]
        reqs += [{"jsonrpc": "2.0", "id": 2 + i, "method": "ping"} for i in range(5)]
        proc = subprocess.Popen([sys.executable, "-c", self.SCRIPT % {"kit": KIT_DIR}], stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for r in reqs:
            proc.stdin.write((json.dumps(r) + "\n").encode())
            proc.stdin.flush()
            time.sleep(0.15)
        time.sleep(1.5)
        proc.stdin.close()
        out, err = proc.communicate(timeout=30)
        lines = out.decode().splitlines()
        self.assertEqual(len(lines), len(reqs))
        for line in lines:
            self.assertEqual(json.loads(line)["jsonrpc"], "2.0")
        self.assertIn(b"python print", err)       # the noise went to stderr instead
        self.assertIn(b"child stdout", err)
        self.assertIn(b"raw fd write", err)


class ChildStdinTests(unittest.TestCase):
    """Ruling: the MCP server's stdin carries the protocol, so no child process rbxpromo starts may inherit it.
    Source check over every rbxpromo module (not a mock of one code path), so present and future call sites are covered."""

    def _calls(self):
        for path in sorted(glob.glob(os.path.join(KIT_DIR, "rbxpromo", "*.py"))):
            with open(path, encoding="utf-8") as f:
                tree = ast.parse(f.read())
            for node in ast.walk(tree):
                if (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                        and isinstance(node.func.value, ast.Name) and node.func.value.id == "subprocess"
                        and node.func.attr in ("run", "Popen", "call", "check_call", "check_output")):
                    yield os.path.basename(path), node

    def test_every_child_gets_stdin_devnull(self):
        seen = 0
        for name, node in self._calls():
            seen += 1
            kw = {k.arg: k.value for k in node.keywords}
            self.assertIn("stdin", kw, "%s:%d starts a child without stdin=" % (name, node.lineno))
            self.assertEqual(ast.unparse(kw["stdin"]), "subprocess.DEVNULL", "%s:%d" % (name, node.lineno))
        self.assertGreaterEqual(seen, 9)

    def test_every_ffmpeg_call_has_nostdin(self):
        seen = 0
        for name, node in self._calls():
            first = node.args[0] if node.args else None
            if isinstance(first, ast.List) and first.elts and ast.unparse(first.elts[0]) == "engine.ffmpeg()":
                seen += 1
                self.assertEqual(ast.unparse(first.elts[1]), "'-nostdin'", "%s:%d" % (name, node.lineno))
        self.assertGreaterEqual(seen, 5)

    def test_children_never_inherit_our_stdout(self):
        for name, node in self._calls():
            kw = {k.arg for k in node.keywords}
            self.assertTrue(kw & {"stdout", "capture_output"}, "%s:%d lets a child write to our stdout" % (name, node.lineno))

    def test_protocol_bytes_stay_unread_by_a_child_started_that_way(self):
        r, w = os.pipe()
        os.write(w, b'{"jsonrpc":"2.0"}\n')
        child = subprocess.run([sys.executable, "-c", "import sys; print(len(sys.stdin.read()))"],
                               stdin=subprocess.DEVNULL, capture_output=True, text=True)
        self.assertEqual(child.stdout.strip(), "0")
        self.assertEqual(os.read(r, 100), b'{"jsonrpc":"2.0"}\n')
        os.close(r)
        os.close(w)


class CliTests(unittest.TestCase):
    def run_cli(self, argv):
        import rbx
        out, err = Capture(), Capture()
        code = 0
        with redirect_stdout(out), redirect_stderr(err):
            try:
                rbx.main(["promo"] + argv)
            except SystemExit as e:
                code = e.code if isinstance(e.code, int) else 1
                if isinstance(e.code, str):
                    err.write(e.code)
        return code, out.getvalue(), err.getvalue()

    def test_status_text_and_json(self):
        with mock.patch.object(engine, "status", return_value=READY):
            code, out, _ = self.run_cli(["status"])
            self.assertEqual(code, 0)
            self.assertIn("ready: true", out)
            code, out, _ = self.run_cli(["status", "--json"])
            self.assertTrue(json.loads(out)["ready"])

    def test_install_json_progress_is_one_json_line_per_callback(self):
        def fake_install(progress):
            for step, f in (("node", 0.0), ("node", 0.5), ("node", 1.0), ("voice", 0.0)):
                progress(step, f)
            return dict(READY)
        todo = dict(READY, ready=False, missing=["node"], download_mb=50)
        with mock.patch.object(engine, "status", return_value=todo), mock.patch.object(engine, "install", side_effect=fake_install):
            code, out, _ = self.run_cli(["install", "--json-progress"])
            lines = [json.loads(l) for l in out.splitlines()]
            self.assertEqual(code, 0)
            self.assertEqual(lines[:4], [{"step": "node", "fraction": 0.0}, {"step": "node", "fraction": 0.5},
                                         {"step": "node", "fraction": 1.0}, {"step": "voice", "fraction": 0.0}])
            self.assertEqual(lines[4], {"ready": True, "missing": []})
            code, out, _ = self.run_cli(["install"])
            self.assertIn("Downloading about 50 MB", out)
            self.assertIn("node: 50%", out)

    def test_recipe_error_prints_every_problem_on_stderr(self):
        problems = ["scene 0: clip file not found: a.mp4", "'fps' must be 30 or 60."]
        with mock.patch.dict(tool.ACTIONS, {"render": mock.Mock(side_effect=RecipeError(problems))}):
            code, out, err = self.run_cli(["render", "r.json"])
        self.assertNotEqual(code, 0)
        self.assertEqual(out, "")
        for p in problems:
            self.assertIn(p, err)

    def test_machine_action_prints_json(self):
        with mock.patch.dict(tool.ACTIONS, {"styles": lambda a, p: {"styles": [{"name": "hype"}]}}):
            code, out, _ = self.run_cli(["styles"])
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(out)["styles"][0]["name"], "hype")

    def test_missing_clip_is_nonzero_with_message(self):
        with mock.patch.object(engine, "status", return_value=READY):
            code, out, err = self.run_cli(["probe", "/no/such/clip.mp4"])
        self.assertNotEqual(code, 0)
        self.assertIn("Clip not found", err)


class DoctorAndRoutingTests(unittest.TestCase):
    def test_doctor_row(self):
        import rbx
        for st, word in ((READY, "ready"),
                         (dict(READY, ready=False, missing=["voice"], download_mb=97), "not installed (missing: voice")):
            buf = Capture()
            with mock.patch.object(engine, "status", return_value=st), redirect_stdout(buf):
                with self.assertRaises(SystemExit):
                    rbx.main(["doctor"])
            row = [l for l in buf.getvalue().splitlines() if "Promo engine" in l]
            self.assertEqual(len(row), 1)
            self.assertIn(word, row[0])

    def test_routing_points_promo_requests_to_the_guide(self):
        with open(os.path.join(KIT_DIR, "routing.txt"), encoding="utf-8") as f:
            routing = f.read()
        self.assertIn("`promo`", routing)
        self.assertIn("TikTok", routing)


if __name__ == "__main__":
    unittest.main()
