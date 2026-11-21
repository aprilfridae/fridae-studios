import json, os, tempfile, unittest
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
            return lambda *a, **k: calls.append(name)
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

    def test_tools_raise_when_not_ready(self):
        with self.assertRaises(EngineError) as cm:
            engine.ffmpeg()
        self.assertIn("install", str(cm.exception))
        with self.assertRaises(EngineError):
            engine.node_cmd("x.mjs")


if __name__ == "__main__":
    unittest.main()
