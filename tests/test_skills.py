"""rbxskills: one list of the user's skills for every AI, the catalog in an agent's instructions, and rbx_skill's
load and read. Built on a made-up home folder, never the real ~/.claude."""
import json
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxskills


def skill(folder, name, desc, body="Do the thing.", extra=None):
    os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, "SKILL.md"), "w", encoding="utf-8") as f:
        f.write("---\nname: %s\ndescription: %s\n---\n\n%s\n" % (name, desc, body))
    for rel, text in (extra or {}).items():
        path = os.path.join(folder, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)


class SkillsTests(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="skills home é ")
        self.addCleanup(shutil.rmtree, self.home, True)
        claude = os.path.join(self.home, ".claude")
        patches = [mock.patch.object(rbxskills, "HOME", self.home),
                   mock.patch.dict(os.environ, {"FRIDAE_USER_CLAUDE": "", "CLAUDE_CONFIG_DIR": ""})]
        for p in patches:
            p.start()
            self.addCleanup(p.stop)
        os.makedirs(claude)
        self.claude = claude
        # Claude Code's own skill, an enabled and a disabled plugin
        skill(os.path.join(claude, "skills", "ui-ux-pro-max"), "ui-ux-pro-max", '"UI/UX for games and apps. Use for \\"menus\\"."',
              "Run ${CLAUDE_SKILL_DIR}/scripts/search.py", {"scripts/search.py": "print(1)", "references/rules.md": "# Rules"})
        on = os.path.join(self.home, "plugins", "kit-on")
        off = os.path.join(self.home, "plugins", "kit-off")
        skill(os.path.join(on, "skills", "tdd"), "tdd", "Use before writing code.", "See ${CLAUDE_PLUGIN_ROOT}/x")
        skill(os.path.join(off, "skills", "hidden"), "hidden", "Never listed.")
        os.makedirs(os.path.join(claude, "plugins"))
        with open(os.path.join(claude, "plugins", "installed_plugins.json"), "w") as f:
            json.dump({"plugins": {"kit@m": [{"installPath": on}], "off@m": [{"installPath": off}]}}, f)
        with open(os.path.join(claude, "settings.json"), "w") as f:
            json.dump({"enabledPlugins": {"kit@m": True, "off@m": False}}, f)
        # synced from claude.ai for two accounts: this one (acct-a) and another (acct-b)
        mine = os.path.join(claude, "plugins", "synced", "org_acct-a", "p1")
        theirs = os.path.join(claude, "plugins", "synced", "org_acct-b", "p2")
        for root, plugin, sk in ((mine, "superpowers", "debugging"), (theirs, "runpod", "pods")):
            os.makedirs(os.path.join(root, ".claude-plugin"))
            with open(os.path.join(root, ".claude-plugin", "plugin.json"), "w") as f:
                json.dump({"name": plugin}, f)
            skill(os.path.join(root, "skills", sk), sk, "About %s." % sk)
        synced_skills = os.path.join(claude, "skills", "synced", "org_acct-a")
        skill(os.path.join(synced_skills, "pdf"), "pdf", "PDF files.")
        skill(os.path.join(synced_skills, "computer-use"), "computer-use", "Desktop app only.")
        with open(os.path.join(self.home, ".claude.json"), "w") as f:
            json.dump({"oauthAccount": {"accountUuid": "acct-a", "emailAddress": "never read"}}, f)
        # another tool's skill folder
        skill(os.path.join(self.home, ".agents", "skills", "codex-only"), "codex-only", "From Codex.")

    def ids(self, **kw):
        return [s["id"] for s in rbxskills.skills(**kw)]

    def test_every_source_is_found_with_plugin_names(self):
        ids = self.ids()
        for want in ("ui-ux-pro-max", "kit:tdd", "superpowers:debugging", "runpod:pods", "pdf", "codex-only"):
            self.assertIn(want, ids)
        self.assertNotIn("off:hidden", ids, "a disabled plugin's skills are not offered")
        self.assertNotIn("computer-use", ids, "a desktop-app-only skill is left out")

    def test_quoted_descriptions_are_unescaped(self):
        d = {s["id"]: s for s in rbxskills.skills()}["ui-ux-pro-max"]["description"]
        self.assertEqual(d, 'UI/UX for games and apps. Use for "menus".')

    def test_the_project_wins_for_a_name(self):
        game = os.path.join(self.home, "game")
        skill(os.path.join(game, ".claude", "skills", "ui-ux-pro-max"), "ui-ux-pro-max", "The game's own.")
        s = {s["id"]: s for s in rbxskills.skills(project=game)}["ui-ux-pro-max"]
        self.assertEqual((s["origin"], s["description"]), ("project", "The game's own."))

    def test_other_ais_get_every_skill_in_the_catalog(self):
        text = rbxskills.catalog("gemini")
        for want in ("## Skills", "rbx_skill", "- ui-ux-pro-max:", "- kit:tdd:", "- superpowers:debugging:",
                     "- runpod:pods:", "- pdf:", "- codex-only:"):
            self.assertIn(want, text)

    def test_claude_gets_only_what_claude_code_does_not_load(self):
        text = rbxskills.catalog("claude")
        for native in ("ui-ux-pro-max", "kit:tdd", "superpowers:debugging", "- pdf:"):
            self.assertNotIn(native, text, "Claude Code lists its own, and its own account's synced skills")
        self.assertIn("- runpod:pods:", text, "another account's synced skill")
        self.assertIn("- codex-only:", text, "another tool's skill")

    def test_a_second_claude_account_uses_its_own_synced_skills(self):
        second = os.path.join(self.home, "second")
        os.makedirs(second)
        with open(os.path.join(second, ".claude.json"), "w") as f:
            json.dump({"oauthAccount": {"accountUuid": "acct-b"}}, f)
        text = rbxskills.catalog("claude", config=second)
        self.assertIn("- superpowers:debugging:", text)
        self.assertNotIn("runpod:pods", text)

    def test_nothing_to_list_is_empty(self):
        shutil.rmtree(os.path.join(self.home, ".agents"))
        shutil.rmtree(os.path.join(self.claude, "plugins", "synced", "org_acct-b"))
        self.assertEqual(rbxskills.catalog("claude"), "")

    def test_load_fills_paths_and_lists_files(self):
        out = rbxskills.load("ui-ux-pro-max")
        base = os.path.join(self.claude, "skills", "ui-ux-pro-max")
        self.assertIn("Base directory: %s" % base, out)
        self.assertIn("Run %s/scripts/search.py" % base, out)
        self.assertIn("references/rules.md", out)
        self.assertIn("scripts/search.py", out)
        self.assertIn("See %s/x" % os.path.join(self.home, "plugins", "kit-on"), rbxskills.load("tdd"))

    def test_names_are_found_loosely_but_not_wrongly(self):
        self.assertEqual(rbxskills.find("/Superpowers:Debugging")["id"], "superpowers:debugging")
        self.assertEqual(rbxskills.find("debugging")["id"], "superpowers:debugging", "a bare name that only one has")
        with self.assertRaises(LookupError) as e:
            rbxskills.find("debug")
        self.assertIn("superpowers:debugging", str(e.exception), "a near name is suggested")

    def test_read_stays_inside_the_skill(self):
        self.assertIn("# Rules", rbxskills.read_file("ui-ux-pro-max", "references/rules.md"))
        for bad in ("../../settings.json", "/etc/passwd", "../tdd/SKILL.md"):
            with self.assertRaises(LookupError):
                rbxskills.read_file("ui-ux-pro-max", bad)

    def test_the_account_file_is_read_for_its_id_only(self):
        self.assertEqual(rbxskills.claude_account(), "acct-a")
        self.assertIsNone(rbxskills.claude_account(os.path.join(self.home, "nowhere")))


if __name__ == "__main__":
    unittest.main()
