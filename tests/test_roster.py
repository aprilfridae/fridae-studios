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

    def test_expert_text_capped(self):
        self.write(os.path.join("experts", "coder.md"), "x" * rbxroster.MAX_TEXT)
        self.assertEqual(len(rbxroster.expert("coder", self.project)["text"]), rbxroster.MAX_TEXT)
        self.write(os.path.join("experts", "coder.md"), "é" * (rbxroster.MAX_TEXT + 1))
        with self.assertRaises(KitError) as cm:
            rbxroster.expert("coder", self.project)
        self.assertIn(os.path.join(self.team_dir(), "experts", "coder.md"), str(cm.exception))
        self.assertIn("16,000", str(cm.exception))
        self.assertEqual(rbxroster.MAX_TEXT, 16000)

    def test_malformed_project_roster(self):
        for bad in ("{not json", json.dumps({"experts": []}), json.dumps([{"name": "no id"}]),
                    json.dumps([{"id": "../x", "name": "X"}]), json.dumps([{"id": "new"}])):
            self.write("roster.json", bad)
            with self.assertRaises(KitError) as cm:
                rbxroster.load(self.project)
            self.assertIn("roster.json", str(cm.exception))
            with self.assertRaises(KitError):
                rbxstage.run("status", {"project": self.project})

    def test_bad_field_values(self):
        full = {"id": "writer", "name": "Story writer", "job": "Writes the story", "stage": "design", "guides": [],
                "tools": [], "kind": "worker", "writes": False}
        cases = [
            ({"id": "coder", "writes": "false"}, "writes"),
            ({"id": "coder", "guides": "promo"}, "guides"),
            ({"id": "coder", "guides": ["no-such-guide"]}, "guides"),
            ({"id": "coder", "tools": "rbx_project"}, "tools"),
            ({"id": "coder", "stage": "polish"}, "stage"),
            ({"id": "coder", "name": ""}, "name"),
            ({"id": "coder", "job": 3}, "job"),
            ({"id": "coder", "kind": "lead"}, "kind"),
            ({"id": "coder", "kind": "boss"}, "kind"),
            (dict(full, kind="lead"), "kind"),
        ]
        for entry, field in cases:
            self.write("roster.json", json.dumps([entry]))
            with self.assertRaises(KitError) as cm:
                rbxroster.load(self.project)
            msg = str(cm.exception)
            self.assertIn("roster.json", msg, entry)
            self.assertIn(repr(entry["id"]), msg, entry)
            self.assertIn(repr(field), msg, entry)
        self.write("roster.json", json.dumps([{"id": "scout", "name": "A"}, {"id": "scout", "name": "B"}]))
        with self.assertRaises(KitError) as cm:
            rbxroster.load(self.project)
        self.assertIn("twice", str(cm.exception))
        self.write("roster.json", json.dumps([{"id": "lead", "kind": "lead"}, dict(full)]))
        self.assertEqual(rbxroster.load(self.project)[-1]["id"], "writer")

    def test_bad_kit_roster(self):
        kit = os.path.join(self.tmp, "kit")
        os.makedirs(kit)
        good = {"id": "coder", "name": "Coder", "job": "Scripts", "stage": "build", "guides": ["roblox-luau"],
                "tools": [], "kind": "worker", "writes": True}
        partial = dict(good)
        del partial["writes"]
        with mock.patch.object(rbxroster, "kit_dir", return_value=kit):
            for data, words in (([good, good], "twice"), ([partial], "missing writes"), ([dict(good, writes=1)], "'writes'")):
                with open(os.path.join(kit, "roster.json"), "w", encoding="utf-8") as f:
                    json.dump(data, f)
                with self.assertRaises(KitError) as cm:
                    rbxroster.load()
                self.assertIn(words, str(cm.exception))
                self.assertIn(os.path.join(kit, "roster.json"), str(cm.exception))

    def test_status_lists_team(self):
        out = rbxstage.run("status", {"project": self.project})
        self.assertIn("Team (", out)
        self.assertIn("Security reviewer [security, test]", out)
        self.assertIn("Coder [coder, build, changes the place]", out)
        self.write("roster.json", json.dumps([{"id": "coder", "name": "Script writer"}]))
        self.assertIn("Script writer", rbxstage.run("status", {"project": self.project}))


class GuideLoadTests(unittest.TestCase):
    """Live run L7: a 6-topic rbx_guide call returned 62 KB and the expert tried to parse it with a shell command."""

    def text(self, topics, role="worker"):
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": role}):
            res = rbxkit_mcp.t_guide({"topics": topics})
        return res["content"][0]["text"]

    def test_an_ordinary_chat_gets_up_to_120_kb(self):
        topics = ["workflow", "unslop", "emil-design-eng", "motion-design", "studio"]
        text = self.text(topics, role="")
        for t in topics:
            self.assertIn("GUIDE: " + t, text, "one call still loads a chat's guides (routing.txt)")
        self.assertNotIn("Left out", text)
        big = self.text(list(rbxkit_mcp.GUIDES), role="")
        self.assertLessEqual(len(big.encode("utf-8")), rbxkit_mcp.GUIDE_CAP_CHAT + 40000)
        self.assertIn("Left out to keep this answer under 120 KB", big)

    def test_one_answer_stays_under_the_cap_and_names_what_was_left_out(self):
        topics = ["workflow", "unslop", "emil-design-eng", "motion-design", "studio"]
        text = self.text(topics)
        self.assertIn("GUIDE: workflow", text, "the first guide always comes")
        self.assertLessEqual(len(text.encode("utf-8")), rbxkit_mcp.GUIDE_CAP + 2000)
        self.assertIn("Left out to keep this answer under 30 KB", text)
        left = text.rsplit("Left out", 1)[1]
        for t in topics[1:]:
            if "GUIDE: " + t not in text:
                self.assertIn(t, left)

    def test_a_small_request_is_whole(self):
        text = self.text(["studio", "producer"])
        self.assertIn("GUIDE: studio", text)
        self.assertIn("GUIDE: producer", text)
        self.assertNotIn("Left out", text)

    def test_expert_files_load_two_topics_at_a_time_and_use_no_shell_on_output(self):
        folder = rbxroster.kit_dir()
        for name in IDS:
            if name == "lead":
                continue
            with open(os.path.join(folder, name + ".md"), encoding="utf-8") as f:
                text = " ".join(f.read().split())
            self.assertIn("at most two topics per rbx_guide call", text, name)
            self.assertIn("Never use shell commands to post-process a tool's output", text, name)


if __name__ == "__main__":
    unittest.main()
