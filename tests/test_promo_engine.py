import json, os, shutil, tempfile, unittest
from unittest import mock

import rbxpaths
from rbxpromo import engine
from rbxpromo.engine import EngineError

_REAL_ENGINE_DIR = engine.engine_dir


class EngineTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)
        p = mock.patch.object(engine, "engine_dir", return_value=self.tmp)
        p.start()
        self.addCleanup(p.stop)

    def test_engine_dir_source_vs_packaged(self):
        with mock.patch.object(rbxpaths, "FROZEN", False):
            self.assertEqual(_REAL_ENGINE_DIR(), os.path.join(rbxpaths.KIT, "engine"))
        with mock.patch.object(rbxpaths, "FROZEN", True):
            self.assertEqual(_REAL_ENGINE_DIR(),
                             os.path.join(rbxpaths.HOME_DIR, "engine"))

    def test_node_download_url(self):
        self.assertTrue(engine.node_url("win32", "AMD64").endswith("win-x64.zip"))
        self.assertTrue(engine.node_url("darwin", "arm64").endswith("darwin-arm64.tar.gz"))
        self.assertTrue(engine.node_url("darwin", "x86_64").endswith("darwin-x64.tar.gz"))
        self.assertIn("v22.1.0", engine.node_url("darwin", "arm64", "v22.1.0"))
        with self.assertRaises(EngineError):
            engine.node_url("linux", "x86_64")

    def test_checksum_mismatch_rejected(self):
        f = os.path.join(self.tmp, "a.bin")
        with open(f, "wb") as fh:
            fh.write(b"hello")
        with self.assertRaises(EngineError):
            engine.verify_sha256(f, "0" * 64)
        self.assertFalse(os.path.exists(f))

    def test_checksum_match_accepted(self):
        f = os.path.join(self.tmp, "a.bin")
        with open(f, "wb") as fh:
            fh.write(b"hello")
        good = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        engine.verify_sha256(f, good)
        self.assertTrue(os.path.exists(f))

    def test_status_reports_missing_parts(self):
        st = engine.status()
        self.assertFalse(st["ready"])
        self.assertEqual(st["missing"], ["node", "renderer", "browser", "whisper", "voice"])
        self.assertIsNone(st["node"])
        self.assertIsInstance(st["download_mb"], int)

    def test_resume_skips_done_steps(self):
        with open(os.path.join(self.tmp, ".state.json"), "w") as f:
            json.dump({"done": ["node"]}, f)
        plan = engine.install(dry_run=True)
        self.assertEqual(plan[0], "renderer")
        self.assertNotIn("node", plan)

    def test_install_resumes_after_failure(self):
        calls = []
        def ok(name):
            def run(*a, **k):
                calls.append(name)
                made = {"browser": [("renderer", "node_modules", ".remotion")],
                        "whisper": [("hf-cache", *engine.ASR_MODEL.split("/"))],
                        "voice": [("hf-cache", *engine.VOICE_MODEL.split("/"))]}.get(name, [])
                for parts in made:  # what the real step leaves on disk
                    os.makedirs(os.path.join(self.tmp, *parts), exist_ok=True)
            return run
        def boom(*a, **k):
            raise EngineError("npm failed")
        steps = {"node": ok("node"), "renderer": boom, "browser": ok("browser"),
                 "whisper": ok("whisper"), "voice": ok("voice")}
        with mock.patch.object(engine, "_STEP_FUNCS", steps):
            with self.assertRaises(EngineError):
                engine.install()
            self.assertEqual(calls, ["node"])
            steps["renderer"] = ok("renderer")
            engine.install()
        self.assertEqual(calls, ["node", "renderer", "browser", "whisper", "voice"])

    def _fake_ready_engine(self):
        src = os.path.join(self.tmp, "src-promo")
        os.makedirs(os.path.join(src, "scripts"))
        for name, text in (("package.json", "{}"), ("package-lock.json", "L1"), ("scripts/a.mjs", "a")):
            with open(os.path.join(src, name), "w") as f:
                f.write(text)
        p = mock.patch.object(engine, "_source_dir", return_value=src)
        p.start()
        self.addCleanup(p.stop)
        os.makedirs(os.path.join(self.tmp, "node", "bin"))
        open(os.path.join(self.tmp, "node", "bin", "node"), "w").close()
        os.makedirs(os.path.join(self.tmp, "renderer", "node_modules", "remotion"))
        os.makedirs(os.path.join(self.tmp, "renderer", "node_modules", ".remotion"))
        for m in (engine.ASR_MODEL, engine.VOICE_MODEL):
            os.makedirs(os.path.join(self.tmp, "hf-cache", *m.split("/")))
        engine._mark_done("node")
        engine._mark_done("renderer")
        for s in ("browser", "whisper", "voice"):
            engine._mark_done(s)
        return src

    def test_status_renderer_stale_on_hash_change(self):
        src = self._fake_ready_engine()
        with mock.patch.object(engine, "_node_path", return_value="/x/node"):
            self.assertTrue(engine.status()["renderer"])
            with open(os.path.join(src, "scripts", "a.mjs"), "w") as f:
                f.write("changed")
            st = engine.status()
            self.assertFalse(st["renderer"])
            self.assertEqual(st["missing"], ["renderer"])
            self.assertEqual(engine.install(dry_run=True), ["renderer"])

    def test_node_modules_in_source_ignored_by_hash(self):
        src = self._fake_ready_engine()
        before = engine.promo_hash()
        os.makedirs(os.path.join(src, "node_modules"))
        with open(os.path.join(src, "node_modules", "x"), "w") as f:
            f.write("x")
        self.assertEqual(engine.promo_hash(), before)

    def test_renderer_refresh_skips_npm_when_lock_unchanged(self):
        src = self._fake_ready_engine()
        with open(os.path.join(src, "scripts", "a.mjs"), "w") as f:
            f.write("changed")
        with mock.patch.object(engine, "_run") as run, mock.patch.object(engine, "_node_path", return_value="/x/node"):
            engine._step_renderer(lambda *a: None)
            run.assert_not_called()
            with open(os.path.join(src, "package-lock.json"), "w") as f:
                f.write("L2")
            engine._step_renderer(lambda *a: None)
            run.assert_called_once()

    def test_download_mb_is_sum_of_missing_steps(self):
        st = engine.status()
        self.assertEqual(st["download_mb"], sum(engine._SIZE_MB[s] for s in st["missing"]))

    def test_install_ends_ready_when_renderer_unmarks_browser(self):
        self._fake_ready_engine()
        calls = []
        def renderer(progress):
            calls.append("renderer")
            engine._unmark("browser")  # what npm ci does after a lock change
        def browser(progress):
            calls.append("browser")
            engine._mark_done("browser")
        with open(os.path.join(engine._source_dir(), "scripts", "a.mjs"), "w") as f:
            f.write("changed")
        steps = dict(engine._STEP_FUNCS, renderer=renderer, browser=browser)
        with mock.patch.object(engine, "_STEP_FUNCS", steps), mock.patch.object(engine, "_node_path", return_value="/x/node"):
            st = engine.install()
        self.assertEqual(calls, ["renderer", "browser"])
        self.assertTrue(st["ready"])
        self.assertEqual(st["missing"], [])

    def test_install_does_not_loop_on_a_step_that_never_sticks(self):
        self._fake_ready_engine()
        with mock.patch.object(engine, "_STEP_FUNCS", dict(engine._STEP_FUNCS, browser=lambda p: None)):
            engine._unmark("browser")
            shutil.rmtree(os.path.join(self.tmp, "renderer", "node_modules", ".remotion"))
            with mock.patch.object(engine, "_node_path", return_value="/x/node"):
                with self.assertRaises(EngineError):
                    engine.install()

    def test_node_env_keeps_hf_cache_inside_engine(self):
        env = engine.node_env()
        self.assertEqual(env["HF_HOME"], os.path.join(self.tmp, "hf-cache"))
        self.assertEqual(env["npm_config_cache"], os.path.join(self.tmp, "npm-cache"))

    def test_status_needs_model_folders(self):
        self._fake_ready_engine()
        with mock.patch.object(engine, "_node_path", return_value="/x/node"):
            self.assertTrue(engine.status()["ready"])
            shutil.rmtree(os.path.join(self.tmp, "hf-cache", *engine.ASR_MODEL.split("/")))
            st = engine.status()
            self.assertFalse(st["ready"])
            self.assertEqual(st["missing"], ["whisper"])
            self.assertEqual(engine.install(dry_run=True), ["whisper"])
            shutil.rmtree(os.path.join(self.tmp, "hf-cache", *engine.VOICE_MODEL.split("/")))
            self.assertEqual(engine.status()["missing"], ["whisper", "voice"])

    def test_tools_raise_when_not_ready(self):
        with self.assertRaises(EngineError) as cm:
            engine.ffmpeg()
        self.assertIn("install", str(cm.exception))
        with self.assertRaises(EngineError):
            engine.node_cmd("x.mjs")


if __name__ == "__main__":
    unittest.main()
