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
        self.kit = os.path.join(self.home, "kit")
        self.user = os.path.join(self.home, "userskills")
        patches = [mock.patch.object(rbxskills, "HOME", self.home),
                   mock.patch.object(rbxskills, "KIT", self.kit),
                   mock.patch.object(rbxskills, "user_dir", lambda: self.user),
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

    @unittest.skip("imported in Task 4")
    def test_every_source_is_found_with_plugin_names(self):
        ids = self.ids()
        for want in ("ui-ux-pro-max", "kit:tdd", "superpowers:debugging", "runpod:pods", "pdf", "codex-only"):
            self.assertIn(want, ids)
        self.assertNotIn("off:hidden", ids, "a disabled plugin's skills are not offered")
        self.assertNotIn("computer-use", ids, "a desktop-app-only skill is left out")

    @unittest.skip("imported in Task 4")
    def test_quoted_descriptions_are_unescaped(self):
        d = {s["id"]: s for s in rbxskills.skills()}["ui-ux-pro-max"]["description"]
        self.assertEqual(d, 'UI/UX for games and apps. Use for "menus".')

    def test_the_project_wins_for_a_name(self):
        game = os.path.join(self.home, "game")
        skill(os.path.join(game, ".claude", "skills", "ui-ux-pro-max"), "ui-ux-pro-max", "The game's own.")
        s = {s["id"]: s for s in rbxskills.skills(project=game)}["ui-ux-pro-max"]
        self.assertEqual((s["origin"], s["description"]), ("project", "The game's own."))

    @unittest.skip("imported in Task 4")
    def test_other_ais_get_every_skill_in_the_catalog(self):
        text = rbxskills.catalog("gemini")
        for want in ("## Skills", "rbx_skill", "- ui-ux-pro-max:", "- kit:tdd:", "- superpowers:debugging:",
                     "- runpod:pods:", "- pdf:", "- codex-only:"):
            self.assertIn(want, text)

    @unittest.skip("imported in Task 4")
    def test_claude_gets_only_what_claude_code_does_not_load(self):
        text = rbxskills.catalog("claude")
        for native in ("ui-ux-pro-max", "kit:tdd", "superpowers:debugging", "- pdf:"):
            self.assertNotIn(native, text, "Claude Code lists its own, and its own account's synced skills")
        self.assertIn("- runpod:pods:", text, "another account's synced skill")
        self.assertIn("- codex-only:", text, "another tool's skill")

    @unittest.skip("imported in Task 4")
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

    @unittest.skip("imported in Task 4")
    def test_load_fills_paths_and_lists_files(self):
        out = rbxskills.load("ui-ux-pro-max")
        base = os.path.join(self.claude, "skills", "ui-ux-pro-max")
        self.assertIn("Base directory: %s" % base, out)
        self.assertIn("Run %s/scripts/search.py" % base, out)
        self.assertIn("references/rules.md", out)
        self.assertIn("scripts/search.py", out)
        self.assertIn("See %s/x" % os.path.join(self.home, "plugins", "kit-on"), rbxskills.load("tdd"))

    @unittest.skip("imported in Task 4")
    def test_names_are_found_loosely_but_not_wrongly(self):
        self.assertEqual(rbxskills.find("/Superpowers:Debugging")["id"], "superpowers:debugging")
        self.assertEqual(rbxskills.find("debugging")["id"], "superpowers:debugging", "a bare name that only one has")
        with self.assertRaises(LookupError) as e:
            rbxskills.find("debug")
        self.assertIn("superpowers:debugging", str(e.exception), "a near name is suggested")

    @unittest.skip("imported in Task 4")
    def test_read_stays_inside_the_skill(self):
        self.assertIn("# Rules", rbxskills.read_file("ui-ux-pro-max", "references/rules.md"))
        for bad in ("../../settings.json", "/etc/passwd", "../tdd/SKILL.md"):
            with self.assertRaises(LookupError):
                rbxskills.read_file("ui-ux-pro-max", bad)

    def test_the_account_file_is_read_for_its_id_only(self):
        self.assertEqual(rbxskills.claude_account(), "acct-a")
        self.assertIsNone(rbxskills.claude_account(os.path.join(self.home, "nowhere")))

    # ---- the library: app, shipped, user, project
    def kit_skill(self, group, name, extra_fm="", desc="Kit skill."):
        d = os.path.join(self.kit, *group.split("/"), name)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: %s\ndescription: %s\n%s---\n\nBody.\n" % (name, desc, extra_fm))

    def test_kit_guides_are_skills_with_owner_from_front_matter(self):
        self.kit_skill("vault/Fridae/Guides/Core", "critic", "owner: app\n")
        self.kit_skill("vault/Fridae/Guides/Roblox subjects", "ui")
        got = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual((got["critic"]["owner"], got["critic"]["origin"]), ("app", "app"))
        self.assertEqual((got["ui"]["owner"], got["ui"]["origin"]), ("shipped", "shipped"))

    def test_app_owned_name_cannot_be_taken(self):
        self.kit_skill("vault/Fridae/Guides/Core", "critic", "owner: app\n", desc="The app's.")
        skill(os.path.join(self.user, "critic"), "critic", "Mine.")
        skill(os.path.join(self.home, "game", ".claude", "skills", "critic"), "critic", "The game's.")
        for project in (None, os.path.join(self.home, "game")):
            hit = [s for s in rbxskills.library(project) if s["id"] == "critic"]
            self.assertEqual([(s["description"], s["origin"]) for s in hit], [("The app's.", "app")])

    def test_later_roots_take_shipped_names_and_the_project_wins(self):
        self.kit_skill("vault/Fridae/Guides/LihFolk", "style", desc="Shipped.")
        skill(os.path.join(self.user, "style"), "style", "User's.")
        self.assertEqual([(s["description"], s["origin"]) for s in rbxskills.library()], [("User's.", "user")])
        game = os.path.join(self.home, "game")
        skill(os.path.join(game, ".agents", "skills", "style"), "style", "Project's.")
        self.assertEqual([(s["description"], s["origin"]) for s in rbxskills.library(game)], [("Project's.", "project")])

    def test_user_skill_cannot_claim_app_owner(self):
        skill(os.path.join(self.user, "mine"), "mine", "Mine.")
        p = os.path.join(self.user, "mine", "SKILL.md")
        with open(p, "w") as f:
            f.write("---\nname: mine\nowner: app\n---\n")
        self.assertEqual(rbxskills.library()[0]["owner"], "user")

    def test_broken_front_matter_is_listed_by_folder_name(self):
        for folder, text in (("oops", "---\nname: [oops\n"), ("nothing", "just text, no front matter"),
                             ("bin", "\x00\x01\xff---\n")):
            d = os.path.join(self.user, folder)
            os.makedirs(d)
            with open(os.path.join(d, "SKILL.md"), "wb") as f:
                f.write(text.encode("latin-1"))
        got = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual(sorted(got), ["bin", "nothing", "oops"])
        self.assertEqual((got["oops"]["name"], got["oops"]["description"]), ("oops", ""))
        self.assertIn("- oops: (no description)", rbxskills.catalog("gemini"))
        self.assertIn("oops", rbxskills.listing())

    def test_meta_defaults(self):
        self.assertEqual(rbxskills.meta({}, "x", "user"),
                         {"name": "x", "description": "", "examples": [], "applies": "both", "core": False,
                          "owner": "user", "source": "", "licence": "", "supersedes": [], "updated": ""})

    def test_meta_reads_every_field_and_rejects_bad_values(self):
        m = rbxskills.meta({"name": "n", "description": "a  b", "examples": ["x", "y"], "applies": "game",
                            "core": "true", "owner": "community", "source": "u", "licence": "MIT",
                            "supersedes": "old", "updated": "2026-01-01"}, "x", "shipped")
        self.assertEqual((m["applies"], m["core"], m["owner"], m["supersedes"], m["description"]),
                         ("game", True, "community", ["old"], "a b"))
        m = rbxskills.meta({"applies": "nope", "owner": "nope", "core": "maybe"}, "x", "shipped")
        self.assertEqual((m["applies"], m["owner"], m["core"]), ("both", "shipped", False))

    def test_front_reads_lists(self):
        fm, body = rbxskills.front("---\nname: a\nexamples:\n  - one\n  - 'two, too'\nsupersedes: [b, c]\n"
                                   "description: >\n  - not a list\n---\nbody")
        self.assertEqual(fm["examples"], ["one", "two, too"])
        self.assertEqual(fm["supersedes"], ["b", "c"])
        self.assertEqual(fm["description"], "- not a list")
        self.assertEqual(body, "body")


if __name__ == "__main__":
    unittest.main()
