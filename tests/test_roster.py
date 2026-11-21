"""The Studio team: the kit roster, the expert files and per-project overrides."""
import json
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxkit_mcp
import rbxroster
import rbxstage
from rbxpaths import KitError

IDS = ["lead", "scout", "designer", "coder", "ui", "world", "effects", "playtester", "devices", "security", "economy",
       "art", "promo"]
KEYS = {"id", "name", "job", "stage", "guides", "tools", "kind", "writes"}


class RosterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="roster é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.project = os.path.join(self.tmp, "My Gäme")
        os.makedirs(self.project)
        env = mock.patch.dict(os.environ, {"RBXKIT_PROJECT": ""})
        env.start()
        self.addCleanup(env.stop)

    def team_dir(self):
        return os.path.join(self.project, ".fridae", "team")

    def write(self, rel, text):
        path = os.path.join(self.team_dir(), rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)

    def test_kit_roster_complete(self):
        roster = rbxroster.load()
        self.assertEqual([e["id"] for e in roster], IDS)
        for e in roster:
            self.assertEqual(set(e), KEYS, e["id"])
            self.assertTrue(e["name"] and e["job"], e["id"])
            self.assertTrue(e["stage"] in rbxstage.STAGES or e["stage"] == "all", e["id"])
            for g in e["guides"]:
                self.assertIn(g, rbxkit_mcp.GUIDES, "%s: guide %r" % (e["id"], g))
            for t in e["tools"]:
                self.assertTrue(t.startswith("rbx_"), "%s: tool %r" % (e["id"], t))
            ex = rbxroster.expert(e["id"])
            self.assertTrue(ex["text"].strip(), e["id"])
            self.assertEqual(set(ex), KEYS | {"text"})
            lines = ex["text"].strip().splitlines()
            self.assertLessEqual(len(lines), 35, e["id"])
            if e["kind"] != "lead":
                self.assertGreaterEqual(len(lines), 15, e["id"])
                for g in e["guides"]:
                    self.assertIn("`%s`" % g, ex["text"], "%s file should name guide %s" % (e["id"], g))
                for t in e["tools"]:
                    self.assertIn(t, ex["text"], "%s file should name tool %s" % (e["id"], t))

    def test_kinds(self):
        kinds = {e["id"]: e["kind"] for e in rbxroster.load()}
        self.assertEqual(kinds.pop("lead"), "lead")
        self.assertEqual(kinds.pop("security"), "critic")
        self.assertEqual(set(kinds.values()), {"worker"})
        lead = rbxroster.expert("lead")
        self.assertEqual((lead["stage"], lead["writes"]), ("all", False))

    def test_writers(self):
        self.assertEqual(sorted(e["id"] for e in rbxroster.load() if e["writes"]),
                         sorted(["coder", "ui", "world", "effects", "economy"]))

    def test_project_override(self):
        self.write(os.path.join("experts", "coder.md"), "Project coder rules.\n")
        self.write("roster.json", json.dumps([{"id": "scout", "name": "Idea finder", "guides": ["market-research"]}]))
        coder = rbxroster.expert("coder", self.project)
        self.assertEqual(coder["text"], "Project coder rules.\n")
        self.assertEqual(coder["name"], "Coder")
        roster = rbxroster.load(self.project)
        self.assertEqual([e["id"] for e in roster], IDS)
        scout = [e for e in roster if e["id"] == "scout"][0]
        self.assertEqual(scout["name"], "Idea finder")
        self.assertEqual(scout["guides"], ["market-research"])
        self.assertEqual(scout["stage"], "idea")  # fields the project did not give stay as in the kit
        kit = {e["id"]: e for e in rbxroster.load()}
        for e in roster:
            if e["id"] != "scout":
                self.assertEqual(e, kit[e["id"]])
        self.assertNotEqual(rbxroster.expert("coder")["text"], "Project coder rules.\n")

    def test_project_adds_expert(self):
        self.write("roster.json", json.dumps([{"id": "writer", "name": "Story writer", "job": "Writes the story",
                                               "stage": "design", "guides": [], "tools": [], "kind": "worker",
                                               "writes": False}]))
        self.write(os.path.join("experts", "writer.md"), "Write the story.\n")
        self.assertEqual(rbxroster.load(self.project)[-1]["id"], "writer")
        self.assertEqual(rbxroster.expert("writer", self.project)["text"], "Write the story.\n")

    def test_unknown_expert(self):
        with self.assertRaises(KitError) as cm:
            rbxroster.expert("wizard")
        for i in IDS:
            self.assertIn(i, str(cm.exception))
        with self.assertRaises(KitError):
            rbxroster.expert("../roster")

    def test_malformed_project_roster(self):
        for bad in ("{not json", json.dumps({"experts": []}), json.dumps([{"name": "no id"}]),
                    json.dumps([{"id": "../x", "name": "X"}]), json.dumps([{"id": "new"}])):
            self.write("roster.json", bad)
            with self.assertRaises(KitError) as cm:
                rbxroster.load(self.project)
            self.assertIn("roster.json", str(cm.exception))
            with self.assertRaises(KitError):
                rbxstage.run("status", {"project": self.project})

    def test_status_lists_team(self):
        out = rbxstage.run("status", {"project": self.project})
        self.assertIn("Team (", out)
        self.assertIn("Security reviewer [security, test]", out)
        self.assertIn("Coder [coder, build, changes the place]", out)
        self.write("roster.json", json.dumps([{"id": "coder", "name": "Script writer"}]))
        self.assertIn("Script writer", rbxstage.run("status", {"project": self.project}))


if __name__ == "__main__":
    unittest.main()
