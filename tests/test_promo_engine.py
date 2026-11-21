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
        with mock.patch.object(rbxpaths, "FROZEN", True), mock.patch.object(engine, "_migrated", True), \
                mock.patch.object(engine, "app_data_dir", return_value=os.path.join(self.tmp, "appdata")):
            self.assertEqual(_REAL_ENGINE_DIR(), os.path.join(self.tmp, "appdata", "engine"))

    def test_app_data_dir_per_platform(self):
        home = os.path.join(os.sep, "home", "jos\u00e9")
        self.assertEqual(engine.app_data_dir("darwin", {}, home),
                         os.path.join(home, "Library", "Application Support", "Fridae Studios"))
        self.assertEqual(engine.app_data_dir("win32", {"LOCALAPPDATA": r"C:\Users\Jos\u00e9\AppData\Local"}, home),
                         os.path.join(r"C:\Users\Jos\u00e9\AppData\Local", "Fridae Studios"))
        self.assertEqual(engine.app_data_dir("win32", {}, home), os.path.join(home, "AppData", "Local", "Fridae Studios"))
        self.assertEqual(engine.app_data_dir("linux", {"XDG_DATA_HOME": "/x/data"}, home), os.path.join("/x/data", "fridae-studios"))
        self.assertEqual(engine.app_data_dir("linux", {}, home), os.path.join(home, ".local", "share", "fridae-studios"))
        for plat in ("darwin", "win32", "linux"):
            self.assertNotIn("Documents", engine.app_data_dir(plat, {}, home))

    def _old_engine(self):
        old = os.path.join(self.tmp, "Documents", "Fridae Studios", "engine")
        os.makedirs(os.path.join(old, "node", "bin"))
        with open(os.path.join(old, "node", "bin", "node"), "w") as f:
            f.write("node")
        with open(os.path.join(old, ".state.json"), "w") as f:
            json.dump({"done": ["node"]}, f)
        return old

    def test_migration_moves_the_old_engine(self):
        old = self._old_engine()
        new = os.path.join(self.tmp, "Library", "Application Support", "Fridae Studios", "engine")
        self.assertTrue(engine.migrate_engine(old, new))
        self.assertFalse(os.path.exists(old))
        self.assertTrue(os.path.isdir(os.path.dirname(old)))  # only the engine folder leaves Documents
        with open(os.path.join(new, "node", "bin", "node")) as f:
            self.assertEqual(f.read(), "node")
        self.assertEqual(os.listdir(os.path.dirname(new)), ["engine"])
        self.assertFalse(engine.migrate_engine(old, new))  # nothing left to move

    def test_migration_across_volumes_copies_then_removes(self):
        old = self._old_engine()
        new = os.path.join(self.tmp, "appdata", "engine")
        real_rename = os.rename
        def rename(a, b):
            if a == old:
                raise OSError(18, "Invalid cross-device link")
            return real_rename(a, b)
        with mock.patch.object(engine.os, "rename", side_effect=rename):
            self.assertTrue(engine.migrate_engine(old, new))
        self.assertFalse(os.path.exists(old))
        self.assertTrue(os.path.isfile(os.path.join(new, ".state.json")))
        self.assertFalse(os.path.exists(new + ".moving"))

    def test_failed_copy_keeps_the_old_engine(self):
        old = self._old_engine()
        new = os.path.join(self.tmp, "appdata", "engine")
        with mock.patch.object(engine.os, "rename", side_effect=OSError(18, "cross-device")), \
                mock.patch.object(engine, "_tree_size", side_effect=[(1, 1), (2, 2)]):
            self.assertFalse(engine.migrate_engine(old, new))
        self.assertTrue(os.path.isfile(os.path.join(old, "node", "bin", "node")))
        self.assertFalse(os.path.exists(new) or os.path.exists(new + ".moving"))

    def test_no_migration_when_the_new_engine_exists(self):
        old = self._old_engine()
        new = os.path.join(self.tmp, "appdata", "engine")
        os.makedirs(new)
        self.assertFalse(engine.migrate_engine(old, new))
        self.assertTrue(os.path.isdir(old))
        self.assertEqual(os.listdir(new), [])

    def test_packaged_engine_dir_migrates_once(self):
        home = os.path.join(self.tmp, "Documents", "Fridae Studios")
        old = self._old_engine()
        with mock.patch.object(rbxpaths, "FROZEN", True), mock.patch.object(rbxpaths, "HOME_DIR", home), \
                mock.patch.object(engine, "_migrated", False), \
                mock.patch.object(engine, "app_data_dir", return_value=os.path.join(self.tmp, "appdata")):
            got = _REAL_ENGINE_DIR()
        self.assertEqual(got, os.path.join(self.tmp, "appdata", "engine"))
        self.assertTrue(os.path.isfile(os.path.join(got, "node", "bin", "node")))
        self.assertFalse(os.path.exists(old))

    def test_second_install_is_refused_while_one_runs(self):
        with open(os.path.join(self.tmp, ".install.lock"), "w") as f:
            json.dump({"pid": os.getppid(), "started": 0}, f)  # a live process that is not this one
        ran = []
        steps = {s: (lambda *a, **k: ran.append(1)) for s in engine.STEPS}
        with mock.patch.object(engine, "_STEP_FUNCS", steps):
            with self.assertRaises(EngineError) as cm:
                engine.install()
        self.assertIn("already being installed", str(cm.exception))
        self.assertEqual(ran, [])
        self.assertTrue(os.path.exists(os.path.join(self.tmp, ".install.lock")))  # still theirs

    def test_stale_install_lock_is_taken_over_and_released(self):
        import subprocess, sys
        dead = subprocess.Popen([sys.executable, "-c", "pass"])
        dead.wait()
        lock = os.path.join(self.tmp, ".install.lock")
        with open(lock, "w") as f:
            json.dump({"pid": dead.pid, "started": 0}, f)
        seen = []
        def step(*a, **k):
            seen.append(rbxpaths.read_json(lock))
        done = ["node", "renderer", "browser", "whisper"]
        with mock.patch.object(engine, "_STEP_FUNCS", {s: step for s in engine.STEPS}), \
                mock.patch.object(engine, "_load_state", side_effect=lambda: list(done)), \
                mock.patch.object(engine, "_mark_done", side_effect=done.append), \
                mock.patch.object(engine, "status", return_value={"ready": True}):
            engine.install()
        self.assertEqual(seen[0]["pid"], os.getpid())
        self.assertFalse(os.path.exists(lock))

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
