"""rbx_project save_scripts: the scripts the Fridae plugin hands over, written in export_scripts' layout."""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

import rbxproject

KIT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def plugin_answer():
    # the shape of the plugin's "sources" answer (the same as COLLECT's)
    return {"place": {"name": "Gäme", "id": 1}, "scripts": [
        {"p": [["ServerScriptService", 1], ["Shop", 1]], "c": "Script", "k": False, "s": "print('shop')\n"},
        {"p": [["ReplicatedStorage", 1], ["Round", 1]], "c": "ModuleScript", "k": True, "s": "return {}"},
        {"p": [["StarterPlayer", 1], ["StarterPlayerScripts", 1], ["Hud", 1]], "c": "LocalScript", "k": False, "s": "-- hud é"},
    ]}


class SaveScriptsTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="save é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.data = os.path.join(self.tmp, "scripts.json")
        with open(self.data, "w", encoding="utf-8") as f:
            json.dump(plugin_answer(), f)
        self.out = os.path.join(self.tmp, ".fridae", "before-undo", "20261121-101500")

    def read(self, rel):
        with open(os.path.join(self.out, *rel.split("/")), encoding="utf-8") as f:
            return f.read()

    def test_written_in_export_layout_with_manifest(self):
        r = rbxproject.save_scripts(self.data, self.out)
        self.assertEqual(r["scripts"], 3)
        self.assertEqual(self.read("ServerScriptService/Shop.server.luau"), "print('shop')\n")
        self.assertEqual(self.read("ReplicatedStorage/Round/init.luau"), "return {}")
        self.assertEqual(self.read("StarterPlayer/StarterPlayerScripts/Hud.client.luau"), "-- hud é")
        man = json.loads(self.read(rbxproject.MANIFEST))
        self.assertEqual(man["files"]["ServerScriptService/Shop.server.luau"]["instance"], "ServerScriptService.Shop")

    def test_refuses_a_folder_with_files_and_unreadable_input(self):
        os.makedirs(self.out)
        with open(os.path.join(self.out, "keep.txt"), "w") as f:
            f.write("mine")
        with self.assertRaises(rbxproject.ProjectError):
            rbxproject.save_scripts(self.data, self.out)
        with self.assertRaises(rbxproject.ProjectError):
            rbxproject.save_scripts(os.path.join(self.tmp, "missing.json"), os.path.join(self.tmp, "x"))

    def test_command_line_exit_status(self):
        env = dict(os.environ, RBXKIT_WORK=os.path.join(self.tmp, "work"))
        ok = subprocess.run([sys.executable, os.path.join(KIT, "rbx.py"), "project", "save_scripts", "--from", self.data, "--out", self.out],
                            capture_output=True, text=True, env=env)
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.assertTrue(os.path.isfile(os.path.join(self.out, "ServerScriptService", "Shop.server.luau")))
        bad = subprocess.run([sys.executable, os.path.join(KIT, "rbx.py"), "project", "save_scripts", "--from", self.data, "--out", self.out],
                             capture_output=True, text=True, env=env)
        self.assertNotEqual(bad.returncode, 0)


if __name__ == "__main__":
    unittest.main()
