"""rbxskills: one list of the user's skills for every AI, the catalog in an agent's instructions, and rbx_skill's
load and read. Built on a made-up home folder, never the real ~/.claude."""
import json
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxpaths
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
                   mock.patch.dict(os.environ, {"FRIDAE_USER_CLAUDE": "", "CLAUDE_CONFIG_DIR": "", "FRIDAE_RAG_FILE": ""})]
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

    def test_every_source_is_imported_under_its_skill_name(self):
        rbxskills.import_external()
        ids = self.ids()
        for want in ("ui-ux-pro-max", "tdd", "debugging", "pods", "pdf", "codex-only"):
            self.assertIn(want, ids)
        self.assertNotIn("hidden", ids, "a disabled plugin's skills are not imported")
        self.assertNotIn("computer-use", ids, "a desktop-app-only skill is left out")
        for s in rbxskills.skills():
            self.assertEqual((s["origin"], os.path.dirname(s["dir"])), ("user", self.user), s["id"])

    def test_quoted_descriptions_are_unescaped(self):
        rbxskills.import_external()
        d = {s["id"]: s for s in rbxskills.skills()}["ui-ux-pro-max"]["description"]
        self.assertEqual(d, 'UI/UX for games and apps. Use for "menus".')

    def test_the_project_wins_for_a_name(self):
        game = os.path.join(self.home, "game")
        skill(os.path.join(game, ".claude", "skills", "ui-ux-pro-max"), "ui-ux-pro-max", "The game's own.")
        s = {s["id"]: s for s in rbxskills.skills(project=game)}["ui-ux-pro-max"]
        self.assertEqual((s["origin"], s["description"]), ("project", "The game's own."))

    def pin(self, *names):
        """Pin these skills by name, as the CORE list does, for one test."""
        p = mock.patch.object(rbxskills, "CORE", tuple(names))
        p.start()
        self.addCleanup(p.stop)

    def test_other_ais_get_the_pinned_skills_in_the_catalog(self):
        self.pin("ui-ux-pro-max", "tdd", "debugging", "pods", "pdf", "codex-only")
        text = rbxskills.catalog("gemini")  # the catalog imports first
        for want in ("## Skills", "rbx_skill", "- ui-ux-pro-max:", "- tdd:", "- debugging:", "- pods:", "- pdf:",
                     "- codex-only:"):
            self.assertIn(want, text)
        self.assertTrue(text.endswith("\n" + rbxskills.OTHER_SKILLS + "\n"), text)
        self.assertNotIn(rbxskills.CLAUDE_PREFERS, text, "only Claude has a skill tool of its own")

    def test_claude_gets_only_what_claude_code_does_not_load(self):
        self.pin("ui-ux-pro-max", "tdd", "debugging", "pods", "pdf", "codex-only")
        text = rbxskills.catalog("claude")
        for native in ("ui-ux-pro-max", "- tdd:", "- debugging:", "- pdf:"):
            self.assertNotIn(native, text, "Claude Code lists its own, and its own account's synced skills")
        self.assertIn("- pods:", text, "another account's synced skill")
        self.assertIn("- codex-only:", text, "another tool's skill")
        self.assertTrue(text.endswith("\n" + rbxskills.OTHER_SKILLS + "\n" + rbxskills.CLAUDE_PREFERS + "\n"), text)

    def test_catalog_is_core_only_and_kind_filtered(self):
        p = mock.patch.object(rbxskills, "maybe_import", lambda: None)
        p.start()
        self.addCleanup(p.stop)
        for i in range(20):
            fm = {0: "core: true\n", 1: "core: true\n", 2: "core: true\napplies: game\n"}.get(i, "")
            d = os.path.join(self.user, "skill-%02d" % i)
            os.makedirs(d)
            with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
                f.write("---\nname: skill-%02d\ndescription: Skill number %d.\n%s---\nBody.\n" % (i, i, fm))
        text = rbxskills.catalog("gemini", kind="plain")
        lines = [l for l in text.splitlines() if l.startswith("- ")]
        self.assertEqual(lines, ["- skill-00: Skill number 0.", "- skill-01: Skill number 1."])
        self.assertEqual(text.splitlines()[-1], rbxskills.OTHER_SKILLS)
        self.assertEqual(rbxskills.OTHER_SKILLS, "Other skills: search with rbx_skill (action search) when a task "
                         "needs know-how you do not have; the app also suggests matching skills on your messages.")
        self.assertEqual(rbxskills.CLAUDE_PREFERS, "When a Fridae skill (rbx_skill) and a Claude Code skill cover "
                         "the same thing, use the Fridae one.")
        game = [l for l in rbxskills.catalog("gemini", kind="game").splitlines() if l.startswith("- ")]
        self.assertEqual(len(game), 3, "a game gets the game-only core skill too")
        # the pinned set by name: one in the library is pinned, one missing is skipped
        d = os.path.join(self.user, "writing-plans")
        os.makedirs(d)
        with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: writing-plans\ndescription: Plans.\n---\nBody.\n")
        self.assertIn("- writing-plans: Plans.", rbxskills.catalog("gemini", kind="plain"))
        self.assertNotIn("brainstorming", rbxskills.catalog("gemini", kind="plain"))
        index = {s["id"]: s for s in rbxskills.index()}
        self.assertEqual({i for i, s in index.items() if s["pinned"]}, {"skill-00", "skill-01", "skill-02", "writing-plans"})
        # the CLI passes the kind on
        import argparse, contextlib, io
        ap = argparse.ArgumentParser()
        rbxskills.register(ap.add_subparsers(dest="cmd", required=True))
        a = ap.parse_args(["skills", "catalog", "--ai", "cursor", "--kind", "plain"])
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            a.fn(a)
        self.assertEqual(buf.getvalue(), rbxskills.catalog("cursor", kind="plain"))

    def test_the_pinned_names(self):
        self.assertEqual(set(rbxskills.CORE), {
            "brainstorming", "systematic-debugging", "test-driven-development", "verification-before-completion",
            "writing-plans", "ui-ux-pro-max", "workflow", "playtest", "ui-design", "game-bible", "game-design", "critic"})

    def test_claude_gets_an_edited_import_of_its_own_skill(self):
        rbxskills.import_external()
        with open(os.path.join(self.user, "ui-ux-pro-max", "SKILL.md"), "a", encoding="utf-8") as f:
            f.write("\nMy own extra rule.\n")
        self.assertIn("- ui-ux-pro-max:", rbxskills.catalog("claude"), "the library's copy differs: Fridae's is listed")

    def test_a_second_claude_account_uses_its_own_synced_skills(self):
        second = os.path.join(self.home, "second")
        os.makedirs(second)
        with open(os.path.join(second, ".claude.json"), "w") as f:
            json.dump({"oauthAccount": {"accountUuid": "acct-b"}}, f)
        self.pin("debugging", "pods")
        text = rbxskills.catalog("claude", config=second)
        self.assertIn("- debugging:", text)
        self.assertNotIn("- pods:", text)

    def test_nothing_pinned_still_says_how_to_find_skills(self):
        self.pin("ui-ux-pro-max", "tdd", "debugging", "pdf")
        shutil.rmtree(os.path.join(self.home, ".agents"))
        shutil.rmtree(os.path.join(self.claude, "plugins", "synced", "org_acct-b"))
        text = rbxskills.catalog("claude")
        self.assertNotIn("\n- ", text, "every pinned skill is one Claude Code lists itself")
        self.assertTrue(text.endswith(rbxskills.OTHER_SKILLS + "\n" + rbxskills.CLAUDE_PREFERS + "\n"), text)

    def test_an_empty_library_is_empty(self):
        p = mock.patch.object(rbxskills, "maybe_import", lambda: None)
        p.start()
        self.addCleanup(p.stop)
        self.assertEqual(rbxskills.catalog("claude"), "")

    def test_load_fills_paths_and_lists_files(self):
        rbxskills.import_external()
        out = rbxskills.load("ui-ux-pro-max")
        base = os.path.join(self.user, "ui-ux-pro-max")
        self.assertIn("Base directory: %s" % base, out)
        self.assertIn("Run %s/scripts/search.py" % base, out)
        self.assertIn("references/rules.md", out)
        self.assertIn("scripts/search.py", out)
        self.assertIn("See %s/x" % os.path.join(self.home, "plugins", "kit-on"), rbxskills.load("tdd"))

    def test_names_are_found_loosely_but_not_wrongly(self):
        rbxskills.import_external()
        self.assertEqual(rbxskills.find("/Superpowers:Debugging")["id"], "debugging", "Claude Code's plugin:skill name")
        self.assertEqual(rbxskills.find("DEBUGGING")["id"], "debugging")
        with self.assertRaises(LookupError) as e:
            rbxskills.find("debug")
        self.assertIn("debugging", str(e.exception), "a near name is suggested")

    def test_read_stays_inside_the_skill(self):
        rbxskills.import_external()
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

    # ---- the Fridae vault's skills (Knowledge/Skills)
    def vault(self, name, extra_fm="", desc="Shared."):
        know = os.path.join(self.home, "Knowledge")
        if not hasattr(self, "_know_patch"):
            self._know_patch = mock.patch.object(rbxpaths, "KNOWLEDGE_DIR", know)
            self._know_patch.start()
            self.addCleanup(self._know_patch.stop)
        d = os.path.join(know, "Skills", name)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: %s\ndescription: %s\n%s---\n\nBody.\n" % (name, desc, extra_fm))
        return d

    def vault_base(self, data):
        folder = os.path.join(self.home, "Knowledge", ".fridae-vault")
        os.makedirs(folder, exist_ok=True)
        with open(os.path.join(folder, "base.json"), "w", encoding="utf-8") as f:
            json.dump(data, f)

    def test_vault_skill_is_listed_as_community(self):
        self.vault("tidy", "credit: someone\n")
        got = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual((got["tidy"]["origin"], got["tidy"]["owner"], got["tidy"]["credit"]),
                         ("vault", "community", "someone"))
        self.assertEqual(rbxskills.vault_dir(), os.path.join(self.home, "Knowledge", "Skills"))
        self.assertIn("credit", rbxskills.INDEX_FIELDS)
        self.assertIn("the Fridae vault (shared by someone)", rbxskills.load_head(got["tidy"]))
        self.vault("anon")
        anon = {s["id"]: s for s in rbxskills.library()}["anon"]
        self.assertTrue(rbxskills.load_head(anon).startswith("# Skill anon (the Fridae vault)"))

    def test_vault_skips_update_and_dot_folders(self):
        self.vault("tidy (update)")
        self.vault(".hidden")
        self.assertEqual([s["id"] for s in rbxskills.library() if s["origin"] == "vault"], [])

    def test_vault_front_matter_cannot_pin_hide_or_claim(self):
        self.kit_skill("vault/Fridae/Guides/Core", "critic", "owner: app\n")
        self.kit_skill("vault/Fridae/Guides/LihFolk", "level-design")
        self.vault("tidy", "core: true\nsupersedes: [critic, level-design]\nowner: app\n")
        got = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual((got["tidy"]["core"], got["tidy"]["supersedes"], got["tidy"]["owner"]), (False, [], "community"))
        self.assertIn("level-design", got)
        self.assertIn("critic", got)

    def test_vault_skill_is_known_by_its_folder_not_its_front_matter_name(self):
        self.kit_skill("vault/Fridae/Guides/LihFolk", "level-design", desc="Fridae's own.")
        for folder in ("tidy-desk", "tidy-shelf"):
            d = self.vault(folder)
            with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
                f.write("---\nname: level-design\ndescription: Not Fridae's.\n---\n\nBody.\n")
        got = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual((got["level-design"]["origin"], got["level-design"]["description"]), ("shipped", "Fridae's own."))
        self.assertEqual([(got[n]["origin"], got[n]["name"]) for n in ("tidy-desk", "tidy-shelf")],
                         [("vault", "tidy-desk"), ("vault", "tidy-shelf")])

    def test_vault_patch_supersedes_hides_shipped(self):
        self.kit_skill("vault/Fridae/Guides/LihFolk", "level-design")
        self.vault("tidy")
        self.vault_base({"patch": {"tidy": {"supersedes": ["level-design"]}}})
        got = {s["id"]: s for s in rbxskills.library()}
        self.assertNotIn("level-design", got)
        self.assertEqual(got["tidy"]["supersedes"], ["level-design"])

    def test_user_skill_takes_vault_name(self):
        self.kit_skill("vault/Fridae/Guides/Core", "critic", "owner: app\n", desc="The app's.")
        self.vault("tidy", desc="Vault's.")
        self.vault("critic", desc="Vault critic.")
        skill(os.path.join(self.user, "tidy"), "tidy", "Mine.")
        got = [(s["id"], s["description"], s["origin"]) for s in rbxskills.library() if s["id"] in ("tidy", "critic")]
        self.assertEqual(sorted(got), [("critic", "The app's.", "app"), ("tidy", "Mine.", "user")])

    def test_read_base_gives_empty_values_for_a_missing_or_broken_file(self):
        import rbxvaultsync
        empty = {"format": 1, "release": "", "files": {}, "deleted": [], "declined": [], "renamed": {}, "waiting": {},
                 "patch": {}}
        self.vault("tidy")
        self.assertEqual(rbxvaultsync.read_base(), empty)
        self.vault_base({"release": "vault-2026.11.24", "files": [1], "patch": {"a": {}}})
        got = rbxvaultsync.read_base()
        self.assertEqual((got["release"], got["files"], got["patch"]), ("vault-2026.11.24", {}, {"a": {}}))
        with open(os.path.join(self.home, "Knowledge", ".fridae-vault", "base.json"), "w") as f:
            f.write("{broken")
        self.assertEqual(rbxvaultsync.read_base(), empty)

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
        with mock.patch.object(rbxskills, "CORE", ("oops",)):
            self.assertIn("- oops: (no description)", rbxskills.catalog("gemini"))
        self.assertIn("oops", rbxskills.listing())

    def test_meta_defaults(self):
        self.assertEqual(rbxskills.meta({}, "x", "user"),
                         {"name": "x", "description": "", "examples": [], "applies": "both", "core": False,
                          "owner": "user", "source": "", "licence": "", "supersedes": [], "updated": "", "credit": ""})

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


# The rbx_guide topics before guides became skills (frozen): every one still loads, the same through either tool.
OLD_GUIDES = [
    "workflow", "playtest", "ui-design", "ui-helpers", "ui-audit", "sim-ui", "vfx", "vfx-basics", "tester",
    "web-to-roblox", "emil-design-eng", "motion-design", "redesign-skill", "reference-design-contract", "unslop",
    "soft-skill", "minimalist-skill", "brutalist-skill", "game-brainstorm", "game-bible", "game-design",
    "economy-design", "player-testing", "sound-design", "market-research", "critic", "producer", "studio", "promo",
    "roblox-analytics-liveops", "roblox-architecture", "roblox-audio", "roblox-characters-animation",
    "roblox-code-review", "roblox-data-stores", "roblox-game-systems", "roblox-input", "roblox-luau",
    "roblox-monetization", "roblox-networking", "roblox-npc-ai", "roblox-open-cloud", "roblox-performance",
    "roblox-physics", "roblox-security", "roblox-teleport-matchmaking", "roblox-testing", "roblox-text-chat",
    "roblox-tooling", "roblox-ui", "roblox-world-building", "lihfolk-figma", "lihfolk-sfx", "lihfolk-systems",
    "lihfolk-tutorial", "lihfolk-vfx", "lihfolk-worldbuilding", "quality-score", "roblox-asset-import",
    "roblox-blender-modeling", "roblox-debugging", "roblox-system-builder", "roblox-trend-game"]


class GuideSkillTests(unittest.TestCase):
    """The app's guides are skills (<name>/SKILL.md) in the real kit; rbx_skill load serves them as rbx_guide did."""

    def setUp(self):
        import rbxkit_mcp
        self.mcp = rbxkit_mcp
        self.user = tempfile.mkdtemp(prefix="user skills é ")
        self.addCleanup(shutil.rmtree, self.user, True)
        for p in (mock.patch.object(rbxskills, "user_dir", lambda: self.user),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "RBXKIT_AGENT_ROLE": ""})):
            p.start()
            self.addCleanup(p.stop)

    def text(self, res):
        return res["content"][0]["text"]

    def guide(self, **a):
        return self.mcp.t_guide(a)

    def load(self, **a):
        return self.mcp.t_skill(dict(action="load", **a))

    def test_guides_are_skill_folders(self):
        for rel, _ in rbxskills.KIT_ROOTS:
            root = os.path.join(rbxskills.KIT, *rel.split("/"))
            for name in os.listdir(root):
                d = os.path.join(root, name)
                if os.path.isdir(d):
                    self.assertTrue(os.path.isfile(os.path.join(d, "SKILL.md")), name)
                    self.assertFalse(os.path.exists(os.path.join(d, name + ".md")), name)

    def test_front_matter(self):
        kit = {s["id"]: s for s in rbxskills.kit_skills()}
        for n, s in kit.items():
            self.assertEqual(n, os.path.basename(s["dir"]), "one id per skill: its folder's name")
        self.assertEqual(sorted(n for n, s in kit.items() if s["owner"] == "app"),
                         ["critic", "producer", "promo", "studio", "workflow"])
        self.assertEqual(sorted(n for n, s in kit.items() if s["core"]),
                         ["critic", "game-bible", "game-design", "playtest", "ui-design", "workflow"])
        for n, s in kit.items():
            self.assertTrue(s["description"], n)
            self.assertTrue(3 <= len(s["examples"]) <= 5, (n, s["examples"]))
            lihfolk = "/LihFolk/" in s["dir"].replace(os.sep, "/")
            self.assertEqual((s["source"], s["licence"]) if lihfolk else ("", ""),
                             ("LihFolk Vault V2", "CC BY 4.0") if lihfolk else ("", ""), n)
            self.assertIn(s["applies"], ("both", "game"), n)
        for n in ("unslop", "soft-skill", "emil-design-eng", "motion-design"):
            self.assertEqual(kit[n]["applies"], "both", n)
        for n in ("workflow", "game-design", "roblox-luau", "lihfolk-vfx"):
            self.assertEqual(kit[n]["applies"], "game", n)

    def test_no_description_repeats_its_opening(self):
        """A pinned skill's catalog line is its description cut at DESC_MAX: a repeated opening fills it with nothing."""
        for s in rbxskills.kit_skills():
            words = s["description"].split()
            head = " ".join(words[:5])
            self.assertEqual(s["description"].count(head), 1, s["id"])
        critic = next(s for s in rbxskills.kit_skills() if s["id"] == "critic")
        line = rbxskills.short(critic["description"])
        for what in ("reviewer", "phone player", "exploiter"):
            self.assertIn(what, line, "the pinned line says what the critic does")

    def test_guides_table_is_built_from_the_library(self):
        kit = rbxskills.kit_skills()
        self.assertGreater(len(kit), 50)
        for s in kit:
            rel = os.path.relpath(os.path.join(s["dir"], "SKILL.md"), self.mcp.KIT).replace(os.sep, "/")
            self.assertEqual(self.mcp.KIT_GUIDES[s["id"]][0], rel, s["id"])
        for name in ("ui-helpers", "ui-audit", "tester"):
            self.assertIn(name, self.mcp.GUIDE_FILES)
            self.assertIn(name, self.mcp.KIT_GUIDES)
        self.assertIs(self.mcp.GUIDES, self.mcp.KIT_GUIDES)

    def test_every_old_guide_name_still_loads(self):
        for name in OLD_GUIDES:
            g, s = self.guide(topic=name), self.load(name=name)
            self.assertFalse(g["isError"], name)
            self.assertEqual(self.text(g), self.text(s), name)
        g = self.guide(topics=["playtest", "unslop", "roblox-ui"])
        self.assertEqual(self.text(g), self.text(self.load(names=["playtest", "unslop", "roblox-ui"])))

    def test_load_section_and_outline(self):
        res = self.load(name="workflow", section="Ground rules")
        text = self.text(res)
        self.assertFalse(res["isError"], text)
        self.assertIn("Section 'Ground rules", text)
        self.assertNotIn("## Sections of this guide", text)
        self.assertEqual(text, self.text(self.guide(topic="workflow", section="Ground rules")))
        whole = self.mcp._one_guide("workflow")
        self.assertLess(len(text), len(whole) // 2, "only that section")
        big = self.text(self.load(name="unslop"))
        self.assertGreater(len(self.mcp._one_guide("unslop").encode("utf-8")), self.mcp.OUTLINE_OVER)
        self.assertIn("## Sections of this guide", big)

    def test_reference_topic(self):
        a, b = self.load(name="roblox-luau/type-system"), self.guide(topic="roblox-luau/type-system")
        self.assertFalse(a["isError"], self.text(a))
        self.assertEqual(self.text(a), self.text(b))
        ref = os.path.join(os.path.dirname(self.mcp._guide_path("roblox-luau")), "references", "type-system.md")
        with open(ref, encoding="utf-8") as f:
            self.assertIn(f.read()[:300], self.text(a))

    def test_a_user_skill_loads_with_its_folder_and_files(self):
        d = os.path.join(self.user, "my-style")
        skill(d, "my-style", "My own style.", "Run ${CLAUDE_SKILL_DIR}/scripts/go.py\n\n## Colours\n\nWarm.",
              {"scripts/go.py": "print(1)", "references/palette.md": "# Palette\n\nOrange."})
        text = self.text(self.load(name="my-style"))
        self.assertIn("Run %s/scripts/go.py" % d.replace(os.sep, "/"), text)
        self.assertIn(d, text)
        self.assertIn("scripts/go.py", text)
        self.assertNotIn("name: my-style", text, "front matter dropped")
        self.assertIn("Warm.", self.text(self.load(name="my-style", section="Colours")))
        self.assertIn("Orange.", self.text(self.load(name="my-style/palette")))
        self.assertTrue(self.load(name="no-such-skill-here")["isError"])

    def test_load_fills_plugin_root_and_frames_only_guides_for_roblox(self):
        """Through the rbx_skill tool: an imported plugin skill's ${CLAUDE_PLUGIN_ROOT} is filled from its import
        link, and only the app's own guides carry the Roblox Studio preface; any other skill gets rbxskills.load's
        neutral framing."""
        d = os.path.join(self.user, "design-x")
        skill(d, "design-x", "Design things.",
              "Run ${CLAUDE_PLUGIN_ROOT}/scripts/x.py then invoke the superpowers:brainstorming skill.\n\n"
              "## Tokens\n\nSee ${CLAUDE_PLUGIN_ROOT}/tokens.md",
              {"references/more.md": "# More\n\nAlso ${CLAUDE_PLUGIN_ROOT}/y.py"})
        with open(os.path.join(d, rbxskills.SOURCE_FILE), "w") as f:
            json.dump({"source": "/x", "key": "plugin:design-x", "hash": "h", "upstream": "h",
                       "plugin_root": "/PLUGROOT"}, f)
        preface = self.mcp.PREFACE.strip().splitlines()[0]
        for res in (self.load(name="design-x"), self.load(name="design-x", section="Tokens"),
                    self.load(name="design-x/more"), self.guide(topic="design-x")):
            text = self.text(res)
            self.assertFalse(res["isError"], text)
            self.assertNotIn("${CLAUDE_PLUGIN_ROOT}", text)
            self.assertIn("/PLUGROOT/", text)
            self.assertNotIn(preface, text, "no Roblox preface on a skill that is not the app's guide")
            self.assertNotIn("skills are Claude Code details you can ignore", text)
        whole = self.text(self.load(name="design-x"))
        self.assertIn("Run /PLUGROOT/scripts/x.py", whole)
        self.assertTrue(whole.startswith("# Skill design-x (%s)" % rbxskills.ORIGIN_LABEL["user"]), whole[:200])
        self.assertIn("Base directory: %s" % d, whole)
        self.assertTrue(self.text(self.load(name="workflow")).startswith(self.mcp.PREFACE), "a shipped guide keeps it")
        both = self.text(self.load(names=["workflow", "design-x"]))
        self.assertEqual(both.count(preface), 1)
        self.assertIn("Run /PLUGROOT/scripts/x.py", both)

    def test_user_skill_with_shipped_name_keeps_roster_valid(self):
        import rbxroster
        import rbxvault
        skill(os.path.join(self.user, "unslop"), "unslop", "My own unslop.", "User unslop.")
        self.assertIn("unslop", self.mcp._kit_guides(), "a user copy never removes a kit name")
        self.assertIn("unslop", self.mcp.KIT_GUIDES)
        self.assertTrue(rbxroster.load())
        kit_path = os.path.normpath(os.path.join(self.mcp.KIT, self.mcp.KIT_GUIDES["unslop"][0]))
        self.assertIn(("unslop", kit_path), rbxvault.guide_files(), "the guides shelf keeps the kit's copy")
        text = self.text(self.load(name="unslop"))
        self.assertIn("User unslop.", text, "the user's copy is served (precedence)")
        self.assertNotIn("This guide was written for websites", text)

    def test_project_skill_shadows_shipped_on_load(self):
        game = tempfile.mkdtemp(prefix="game é ")
        self.addCleanup(shutil.rmtree, game, True)
        skill(os.path.join(game, ".claude", "skills", "game-design"), "game-design", "This game's design.", "Project design.")
        self.assertNotIn("Project design.", self.text(self.load(name="game-design")))
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": game}):
            self.assertIn("Project design.", self.text(self.load(name="game-design")))
            self.assertIn("Project design.", self.text(self.guide(topic="game-design")))
            skill(os.path.join(game, ".claude", "skills", "critic"), "critic", "Not the app's.", "Fake critic.")
            self.assertNotIn("Fake critic.", self.text(self.load(name="critic")), "an app-owned name is never taken")

    def test_skill_added_after_import_is_loadable(self):
        self.assertTrue(self.load(name="late-skill")["isError"])
        skill(os.path.join(self.user, "late-skill"), "late-skill", "Added later.", "Late body.")
        res = self.load(name="late-skill")
        self.assertFalse(res["isError"], self.text(res))
        self.assertIn("Late body.", self.text(res))

    def test_old_name_is_an_alias_after_exact_lookup(self):
        a, b = self.text(self.load(name="roblox-studio-workflow")), self.text(self.load(name="workflow"))
        self.assertEqual(a, b)
        self.assertIn("GUIDE: workflow ", a)
        self.assertIn("GUIDE: minimalist-skill ", self.text(self.load(name="minimalist-ui")))
        skill(os.path.join(self.user, "minimalist-ui"), "minimalist-ui", "Mine, literally named so.", "My minimal.")
        text = self.text(self.load(name="minimalist-ui"))
        self.assertIn("My minimal.", text, "a skill with the exact name wins over the alias")
        self.assertIn("# Skill minimalist-ui (", text, "framed as the user's skill, not the app's guide")
        self.assertNotIn("My minimal.", self.text(self.load(name="minimalist-skill")))

    def test_read_accepts_alias_and_id(self):
        a = self.mcp.t_skill({"action": "read", "name": "roblox-ui-design", "path": "web-to-roblox.md"})
        b = self.mcp.t_skill({"action": "read", "name": "ui-design", "path": "web-to-roblox.md"})
        self.assertFalse(a["isError"], self.text(a))
        self.assertEqual(self.text(a), self.text(b))
        self.assertTrue(self.text(a).startswith("ui-design/web-to-roblox.md"), "named by its id")
        listed = rbxskills.listing()
        self.assertIn("- ui-design [", listed)
        self.assertNotIn("roblox-ui-design", listed)


def skill_with(folder, name, desc, examples, applies="both", body="Do the thing."):
    os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, "SKILL.md"), "w", encoding="utf-8") as f:
        f.write("---\nname: %s\ndescription: %s\nexamples:\n%s\napplies: %s\n---\n\n%s\n" % (
            name, desc, "".join("  - %s\n" % e for e in examples), applies, body))


class SkillsShelfTests(unittest.TestCase):
    """The `skills` search shelf: one summary entry per skill, found by what it is for."""

    def setUp(self):
        import rbxkit_mcp
        import rbxvault
        self.mcp, self.vault = rbxkit_mcp, rbxvault
        self.user = tempfile.mkdtemp(prefix="user skills shelf é ")
        self.addCleanup(shutil.rmtree, self.user, True)
        for p in (mock.patch.object(rbxskills, "user_dir", lambda: self.user),
                  mock.patch.object(rbxvault, "rag_endpoint", lambda: None),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "RBXKIT_AGENT_ROLE": "", "FRIDAE_RAG_FILE": ""})):
            p.start()
            self.addCleanup(p.stop)
        skill_with(os.path.join(self.user, "menu-polish"), "menu-polish", "Make menus feel good.",
                   ["make a shop menu feel juicy", "polish the settings screen"])
        skill_with(os.path.join(self.user, "tax-notes"), "tax-notes", "Keep a tidy tax folder.",
                   ["file my receipts"], applies="plain")
        skill_with(os.path.join(self.user, "spawn-tuning"), "spawn-tuning", "Tune enemy spawns.",
                   ["enemies spawn too fast"], applies="game")

    def test_search_finds_skill_by_example(self):
        hits = rbxskills.search("shop menu feels flat")
        self.assertEqual(hits[0]["id"], "menu-polish")
        self.assertEqual(set(hits[0]), {"id", "description", "why", "score"})
        self.assertLessEqual(len(hits[0]["why"]), 60)
        self.assertIn("shop menu", hits[0]["why"])

    def test_search_respects_kind(self):
        self.assertIn("spawn-tuning", [h["id"] for h in rbxskills.search("enemies spawn too fast", kind="game")])
        self.assertNotIn("spawn-tuning", [h["id"] for h in rbxskills.search("enemies spawn too fast", kind="plain")])
        self.assertNotIn("tax-notes", [h["id"] for h in rbxskills.search("file my receipts", kind="game")])
        self.assertIn("tax-notes", [h["id"] for h in rbxskills.search("file my receipts", kind="plain")])

    def test_search_uses_the_index_service_when_it_runs(self):
        hits = [{"shelf": "skills", "title": "tax-notes", "score": 0.9}, {"shelf": "skills", "title": "gone", "score": 0.8},
                {"shelf": "skills", "title": "menu-polish", "score": 0.5}]
        with mock.patch.object(self.vault, "rag_endpoint", lambda: ("http://127.0.0.1:1", "t")), \
                mock.patch.object(self.vault, "_rag_hits", lambda end, q, shelves, k, filters=None: hits) as m:
            got = rbxskills.search("anything", kind="plain")
        self.assertEqual([h["id"] for h in got], ["tax-notes", "menu-polish"])
        self.assertEqual(got[0]["score"], 0.9)
        self.assertEqual(got[0]["description"], "Keep a tidy tax folder.")

    def test_search_falls_back_when_the_service_fails(self):
        def boom(*a, **k):
            raise OSError("down")
        with mock.patch.object(self.vault, "rag_endpoint", lambda: ("http://127.0.0.1:1", "t")), \
                mock.patch.object(self.vault, "_rag_hits", boom):
            self.assertEqual(rbxskills.search("shop menu feels flat")[0]["id"], "menu-polish")

    def test_skill_docs_are_one_summary_each(self):
        docs = {d["title"]: d for d in self.vault.skill_docs()}
        d = docs["menu-polish"]
        self.assertEqual(d["shelf"], "skills")
        self.assertEqual(d["text"], "menu-polish\nMake menus feel good.\nFor: make a shop menu feel juicy; polish the settings screen")
        self.assertEqual((d["applies"], d["owner"]), ("both", "user"))
        self.assertEqual(docs["tax-notes"]["applies"], "plain")
        self.assertIn("workflow", docs, "the app's own guides are on the shelf too")

    def test_skill_chunk_is_one_summary(self):
        from rbxrag import chunks
        folder = os.path.join(self.user, "huge")
        skill_with(folder, "huge", "A big one.", ["do the huge thing"], body="Paragraph of words.\n\n" * 2000 + "## Part\n\nMore.\n")
        path = os.path.join(folder, "SKILL.md")
        self.assertGreater(os.path.getsize(path), 40000)
        out = chunks.chunks_for_file("skills", "huge", path, chunks.make_token_counter())
        self.assertEqual(len(out), 1)
        self.assertTrue(out[0]["text"].startswith("huge\n"))
        self.assertEqual(out[0]["shelf"], "skills")
        self.assertNotIn("Paragraph of words", out[0]["text"])

    def test_corpus_has_one_skills_entry_per_skill_and_guides_keep_their_bodies(self):
        from rbxrag import chunks
        files = chunks.corpus_files(None)
        mine = [f for f in files if f[0] == "skills"]
        self.assertIn("menu-polish", [t for _, t, _ in mine])
        self.assertEqual(len({t for _, t, _ in mine}), len(mine))
        keys = [p for _, _, p in files]
        self.assertEqual(len(keys), len(set(keys)), "no index key twice (a guide's SKILL.md is on both shelves)")
        self.assertTrue(any(s == "guides" and t == "workflow" for s, t, _ in files))
        for _, t, p in mine:
            self.assertTrue(os.path.isfile(chunks.real_path(p)), t)

    def test_skills_are_not_in_ordinary_searches(self):
        self.assertNotIn("skills", self.vault.KNOWLEDGE)
        self.assertIn("skills", self.vault.SHELVES)
        text = self.vault.run("search", {"query": "enemies spawn too fast", "shelf": "guides"})
        self.assertNotIn("spawn-tuning", text.split("Skills that may apply")[0])

    def test_vault_search_points_at_skill(self):
        res = self.mcp.t_vault({"action": "search", "query": "shop menu feels flat"})
        text = res["content"][0]["text"]
        self.assertFalse(res["isError"], text)
        self.assertIn("rbx_skill load menu-polish", text)

    def test_vault_search_on_the_skills_shelf_lists_skills(self):
        text = self.vault.run("search", {"query": "shop menu feels flat", "shelf": "skills"})
        self.assertIn("- menu-polish:", text)

    def test_rbx_skill_search_answers_lines(self):
        res = self.mcp.t_skill({"action": "search", "query": "shop menu feels flat"})
        text = res["content"][0]["text"]
        self.assertFalse(res["isError"], text)
        self.assertIn("- menu-polish: Make menus feel good. (load with rbx_skill load menu-polish)", text)
        self.assertTrue(self.mcp.t_skill({"action": "search"})["isError"])
        res = self.mcp.t_skill({"action": "search", "query": "file my receipts", "kind": "game"})
        self.assertNotIn("tax-notes", res["content"][0]["text"])


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


class ImportTests(unittest.TestCase):
    """Import from other tools' folders into the user's library: copied once, refreshed while unedited, overlaps wait
    for the user's decision, nothing overwritten. A made-up home, kit and library; the real ~/.claude is never read."""

    def setUp(self):
        self.home = tempfile.mkdtemp(prefix="import home é ")
        self.addCleanup(shutil.rmtree, self.home, True)
        self.kit = os.path.join(self.home, "kit")
        self.user = os.path.join(self.home, "library")
        for p in (mock.patch.object(rbxskills, "HOME", self.home),
                  mock.patch.object(rbxskills, "KIT", self.kit),
                  mock.patch.object(rbxskills, "user_dir", lambda: self.user),
                  mock.patch.dict(os.environ, {"FRIDAE_USER_CLAUDE": "", "CLAUDE_CONFIG_DIR": "", "FRIDAE_RAG_FILE": ""})):
            p.start()
            self.addCleanup(p.stop)
        self.claude = os.path.join(self.home, ".claude", "skills")
        self.agents = os.path.join(self.home, ".agents", "skills")
        os.makedirs(self.claude)
        os.makedirs(self.agents)

    def ids(self):
        return sorted(s["id"] for s in rbxskills.library())

    def kit_skill(self, name, desc, owner="", body="Kit body."):
        d = os.path.join(self.kit, "vault", "Fridae", "Guides", "Core", name)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: %s\ndescription: %s\n%s---\n\n%s\n" % (name, desc, "owner: %s\n" % owner if owner else "", body))
        return d

    def test_folder_hash_ignores_dot_files(self):
        d = os.path.join(self.home, "h")
        skill(d, "h", "Hash me.", extra={"scripts/a.py": "print(1)"})
        h = rbxskills.folder_hash(d)
        self.assertEqual(len(h), 64)
        for junk in (".DS_Store", ".fridae-source.json", "scripts/.DS_Store"):
            with open(os.path.join(d, *junk.split("/")), "w") as f:
                f.write("junk")
        self.assertEqual(rbxskills.folder_hash(d), h)
        with open(os.path.join(d, "scripts", "a.py"), "w") as f:
            f.write("print(2)")
        self.assertNotEqual(rbxskills.folder_hash(d), h)

    def test_import_copies_new_and_links_identical(self):
        src = os.path.join(self.claude, "alpha")
        skill(src, "alpha", "Draw maps of caves.", extra={"scripts/map.py": "print('map')"})
        skill(os.path.join(self.user, "beta"), "beta", "Bake bread at home.")
        skill(os.path.join(self.agents, "beta"), "beta", "Bake bread at home.")
        got = rbxskills.import_external()
        self.assertEqual((got["added"], got["linked"], got["refreshed"], got["pending"]), (["alpha"], ["beta"], [], []))
        copy = os.path.join(self.user, "alpha")
        self.assertEqual(read(os.path.join(copy, "SKILL.md")), read(os.path.join(src, "SKILL.md")))
        self.assertEqual(read(os.path.join(copy, "scripts", "map.py")), "print('map')")
        link = json.loads(read(os.path.join(copy, ".fridae-source.json")))
        self.assertEqual((link["source"], link["hash"], link["gone"]), (src, rbxskills.folder_hash(src), False))
        self.assertTrue(link["imported"])
        beta = json.loads(read(os.path.join(self.user, "beta", ".fridae-source.json")))
        self.assertEqual(beta["source"], os.path.join(self.agents, "beta"))
        self.assertEqual(self.ids(), ["alpha", "beta"])
        self.assertEqual(rbxskills.pending(), [])
        again = rbxskills.import_external()
        self.assertEqual(again, {"added": [], "linked": [], "refreshed": [], "pending": []}, "nothing new the second time")

    def test_same_name_different_content_is_one_decision(self):
        mine, theirs = os.path.join(self.claude, "brainstorming"), os.path.join(self.agents, "brainstorming")
        skill(mine, "brainstorming", "Explore ideas before building.", "Claude's way.")
        skill(theirs, "brainstorming", "Explore ideas before building.", "Agents' way.")
        before = (read(os.path.join(mine, "SKILL.md")), read(os.path.join(theirs, "SKILL.md")))
        for _ in range(2):
            rbxskills.import_external()
            hits = [s for s in rbxskills.library() if s["id"] == "brainstorming"]
            self.assertEqual(len(hits), 1, "one library entry")
            self.assertIn("Claude's way.", read(os.path.join(hits[0]["dir"], "SKILL.md")), "the first found is in use")
            open_ = rbxskills.pending()
            self.assertEqual(len(open_), 1, "one decision, also after a second import")
            d = open_[0]
            self.assertEqual((d["kind"], d["a"], d["b"], d["state"], d["recommendation"]),
                             ("skill", "brainstorming", "brainstorming", "open", None))
            self.assertEqual(d["a_dir"], hits[0]["dir"])
            self.assertTrue(d["b_dir"].startswith(os.path.join(self.user, ".incoming") + os.sep))
            self.assertIn("Agents' way.", read(os.path.join(d["b_dir"], "SKILL.md")))
            self.assertTrue(d["id"] and d["made"])
        self.assertEqual((read(os.path.join(mine, "SKILL.md")), read(os.path.join(theirs, "SKILL.md"))), before,
                         "neither source is overwritten")

    def test_unedited_copy_refreshes_edited_copy_waits(self):
        one, two = os.path.join(self.claude, "one"), os.path.join(self.agents, "two")
        skill(one, "one", "Paint watercolour skies.", "Old one.")
        skill(two, "two", "Tune guitar strings.", "Old two.")
        rbxskills.import_external()
        mine = os.path.join(self.user, "two", "SKILL.md")
        with open(mine, "a", encoding="utf-8") as f:
            f.write("My own tip.\n")
        skill(one, "one", "Paint watercolour skies.", "New one.")
        skill(two, "two", "Tune guitar strings.", "New two.")
        got = rbxskills.import_external()
        self.assertEqual((got["refreshed"], got["pending"], got["added"]), (["one"], ["two"], []))
        self.assertIn("New one.", read(os.path.join(self.user, "one", "SKILL.md")), "the unedited copy follows its source")
        self.assertIn("My own tip.", read(mine), "the edited copy is never overwritten")
        self.assertIn("Old two.", read(mine))
        update = os.path.join(self.user, "two (update)")
        self.assertIn("New two.", read(os.path.join(update, "SKILL.md")), "the new one waits beside it")
        self.assertEqual([s["dir"] for s in rbxskills.library() if s["id"] == "two"], [os.path.join(self.user, "two")])
        [d] = rbxskills.pending()
        self.assertEqual((d["a"], d["b"], d["a_dir"], d["b_dir"]), ("two", "two", os.path.join(self.user, "two"), update))
        self.assertEqual(rbxskills.import_external()["pending"], [], "no second decision for the same update")
        self.assertEqual(len(rbxskills.pending()), 1)
        rbxskills.decide(d["id"], "keep")
        self.assertFalse(os.path.exists(update))
        self.assertEqual(rbxskills.import_external(), {"added": [], "linked": [], "refreshed": [], "pending": []},
                         "a kept copy is not asked about again for the same update")

    def test_source_deleted_keeps_copy(self):
        src = os.path.join(self.agents, "solo")
        skill(src, "solo", "Sing solo parts.", "Solo body.")
        rbxskills.import_external()
        shutil.rmtree(src)
        got = rbxskills.import_external()
        self.assertEqual(got, {"added": [], "linked": [], "refreshed": [], "pending": []})
        link = json.loads(read(os.path.join(self.user, "solo", ".fridae-source.json")))
        self.assertTrue(link["gone"])
        self.assertIn("Solo body.", rbxskills.load("solo"), "the library copy is still served")
        self.assertEqual(rbxskills.pending(), [])
        self.assertEqual(rbxskills.import_external(), got, "not re-imported or flagged")

    def overlap(self):
        """The user's own brainstorm-ideas and another tool's brainstorming that does the same: one open decision."""
        skill(os.path.join(self.user, "brainstorm-ideas"), "brainstorm-ideas",
              "Turn rough ideas into designs before building anything.", "Mine.")
        skill(os.path.join(self.agents, "brainstorming"), "brainstorming",
              "Turn rough ideas into designs before building anything.", "Theirs.")
        got = rbxskills.import_external()
        self.assertEqual(got["pending"], ["brainstorming"])
        self.assertEqual(self.ids(), ["brainstorm-ideas"], "the newcomer waits outside the library")
        [d] = rbxskills.pending()
        return d

    def test_pending_shows_each_side_from_its_own_folder(self):
        """a_meta and b_meta: read from each side's folder, the waiting newcomer's too; scripts is true for a
        non-empty scripts/ folder or an executable file, false for a skill that is only text."""
        skill(os.path.join(self.user, "brainstorm-ideas"), "brainstorm-ideas",
              "Turn rough ideas into designs before building anything.", "Mine.")
        theirs = os.path.join(self.agents, "brainstorming")
        os.makedirs(theirs)
        with open(os.path.join(theirs, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: brainstorming\ndescription: Turn rough ideas into designs before building anything.\n"
                    "owner: community\nlicence: MIT\nsource: github.com/someone/skills\n---\n\nTheirs.\n")
        os.makedirs(os.path.join(theirs, "scripts"))
        with open(os.path.join(theirs, "scripts", "go.sh"), "w") as f:
            f.write("echo hi\n")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        self.assertTrue(d["b_dir"].startswith(os.path.join(self.user, ".incoming")))
        self.assertEqual(d["a_meta"], {"description": "Turn rough ideas into designs before building anything.",
                                       "owner": "user", "licence": "", "source": "", "scripts": False})
        self.assertEqual(d["b_meta"], {"description": "Turn rough ideas into designs before building anything.",
                                       "owner": "community", "licence": "MIT", "source": "github.com/someone/skills",
                                       "scripts": True})
        self.assertNotIn("a_meta", json.loads(read(os.path.join(self.user, rbxskills.DECISIONS)))[0], "never stored")
        tool = os.path.join(self.user, "brainstorm-ideas", "tool.cmd" if os.name == "nt" else "tool")  # Windows: by kind
        with open(tool, "w") as f:
            f.write("#!/bin/sh\n")
        os.chmod(tool, 0o755)
        self.assertTrue(rbxskills.pending()[0]["a_meta"]["scripts"], "an executable file counts")
        shutil.rmtree(os.path.join(self.user, "brainstorm-ideas"))
        self.assertIsNone(rbxskills.pending()[0]["a_meta"], "a folder that is gone has no meta")

    def test_decide_replace_moves_loser_and_marks_supersedes(self):
        d = self.overlap()
        out = rbxskills.decide(d["id"], "replace")
        self.assertEqual((out["state"], out["choice"]), ("done", "replace"))
        self.assertFalse(os.path.exists(os.path.join(self.user, "brainstorm-ideas")))
        self.assertIn("Mine.", read(os.path.join(self.user, ".replaced", "brainstorm-ideas", "SKILL.md")), "never deleted")
        lib = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual(sorted(lib), ["brainstorming"])
        self.assertEqual(lib["brainstorming"]["supersedes"], ["brainstorm-ideas"])
        self.assertIn("Theirs.", read(os.path.join(self.user, "brainstorming", "SKILL.md")))
        self.assertEqual(os.listdir(os.path.join(self.user, ".incoming")), [])
        self.assertEqual(rbxskills.pending(), [])
        self.assertEqual(rbxskills.import_external(), {"added": [], "linked": [], "refreshed": [], "pending": []})
        with self.assertRaises(LookupError):
            rbxskills.decide(d["id"], "keep")

    def test_decide_keep_moves_newcomer_aside_and_is_not_asked_again(self):
        d = self.overlap()
        rbxskills.decide(d["id"], "keep")
        self.assertEqual(self.ids(), ["brainstorm-ideas"])
        self.assertEqual(rbxskills.library()[0]["supersedes"], ["brainstorming"])
        self.assertIn("Theirs.", read(os.path.join(self.user, ".replaced", "brainstorming", "SKILL.md")))
        self.assertEqual(rbxskills.import_external()["pending"], [])
        self.assertEqual(rbxskills.pending(), [])

    def test_decide_keep_both_renames_a_same_name_newcomer(self):
        skill(os.path.join(self.claude, "notes"), "notes", "Take meeting notes.", "Claude's.")
        skill(os.path.join(self.agents, "notes"), "notes", "Take meeting notes.", "Agents'.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        rbxskills.decide(d["id"], "keep_both")
        lib = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual(sorted(lib), ["notes", "notes-2"])
        self.assertEqual(lib["notes-2"]["dir"], os.path.join(self.user, "notes-2"))
        self.assertIn("Agents'.", read(os.path.join(self.user, "notes-2", "SKILL.md")))
        self.assertEqual(rbxskills.import_external(), {"added": [], "linked": [], "refreshed": [], "pending": []})

    def test_decide_merge_writes_merged_and_keeps_old(self):
        d = self.overlap()
        with self.assertRaises(ValueError):
            rbxskills.decide(d["id"], "merge")
        merged = "---\nname: brainstorm-ideas\ndescription: Both, merged.\n---\n\nMerged body.\n"
        rbxskills.decide(d["id"], "merge", merged)
        lib = {s["id"]: s for s in rbxskills.library()}
        self.assertEqual(sorted(lib), ["brainstorm-ideas"])
        self.assertEqual((lib["brainstorm-ideas"]["description"], lib["brainstorm-ideas"]["supersedes"]),
                         ("Both, merged.", ["brainstorming"]))
        self.assertIn("Merged body.", read(os.path.join(self.user, "brainstorm-ideas", "SKILL.md")))
        self.assertIn("Mine.", read(os.path.join(self.user, ".replaced", "brainstorm-ideas", "SKILL.md")))
        self.assertIn("Theirs.", read(os.path.join(self.user, ".replaced", "brainstorming", "SKILL.md")))
        self.assertEqual(rbxskills.import_external()["pending"], [])

    def test_decide_refuses_app_skill(self):
        app = self.kit_skill("critic", "Review the game with a critical eye.", owner="app", body="The app's critic.")
        skill(os.path.join(self.agents, "critic"), "critic", "Review the game with a critical eye.", "Another critic.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        self.assertEqual((d["a"], d["a_dir"]), ("critic", app))
        for choice in ("replace", "merge"):
            with self.assertRaises(ValueError) as e:
                rbxskills.decide(d["id"], choice, "---\nname: critic\n---\nMerged.")
            self.assertEqual(str(e.exception), "That is one of Fridae's own skills.")
        self.assertIn("The app's critic.", read(os.path.join(app, "SKILL.md")))
        self.assertEqual(len(rbxskills.pending()), 1, "still open")
        with self.assertRaises(ValueError):
            rbxskills.decide(d["id"], "delete")
        rbxskills.decide(d["id"], "keep")
        self.assertEqual([(s["id"], s["origin"]) for s in rbxskills.library()], [("critic", "app")])

    def test_replacing_a_shipped_skill_hides_it(self):
        self.kit_skill("ui-design", "Design game menus and HUD screens.")
        skill(os.path.join(self.agents, "menu-maker"), "menu-maker", "Design game menus and HUD screens.", "Mine.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        self.assertEqual(d["a"], "ui-design")
        rbxskills.decide(d["id"], "replace")
        self.assertEqual(self.ids(), ["menu-maker"], "the shipped one is superseded, its files untouched")
        self.assertTrue(os.path.isfile(os.path.join(self.kit, "vault", "Fridae", "Guides", "Core", "ui-design", "SKILL.md")))

    def test_app_only_never_imported(self):
        skill(os.path.join(self.agents, "computer-use"), "computer-use", "Desktop app only.")
        skill(os.path.join(self.claude, "folder-name"), "morning", "Desktop app only.")
        skill(os.path.join(self.home, ".claude", "skills", "synced", "org_a", "docs"), "docs", "Desktop app only.")
        skill(os.path.join(self.user, "chrome-browser"), "chrome-browser", "Copied by hand.")
        got = rbxskills.import_external()
        self.assertEqual(got, {"added": [], "linked": [], "refreshed": [], "pending": []})
        self.assertEqual([n for n in os.listdir(self.user) if not n.startswith(".")], ["chrome-browser"])
        incoming = os.path.join(self.user, ".incoming")
        self.assertEqual(os.listdir(incoming) if os.path.isdir(incoming) else [], [])
        self.assertEqual(rbxskills.pending(), [])
        self.assertEqual(self.ids(), [], "never listed either")

    def test_installed_core_copies_are_linked_not_imported(self):
        import install
        core = os.path.join(self.kit, "vault", "Fridae", "Guides", "Core")
        self.kit_skill("workflow", "Run the Roblox studio workflow for every game task.", owner="app",
                       body="Scripts live in {{RBXKIT}}/studio.")
        self.kit_skill("unslop", "Remove generic AI looks from designs.")
        install.build_claude_skills(self.claude, core=core)  # as install.py writes ~/.claude/skills
        self.assertTrue(os.path.isdir(os.path.join(self.claude, "roblox-studio-workflow")))
        got = rbxskills.import_external()
        self.assertEqual((got["added"], got["pending"]), ([], []), "Fridae's own installed copies are not imported")
        self.assertEqual(sorted(got["linked"]), ["roblox-studio-workflow", "unslop"])
        self.assertEqual(self.ids(), ["unslop", "workflow"])
        self.assertIn("workflow", rbxskills.native_ids(), "Claude Code lists it itself (under its old name)")
        with open(os.path.join(self.claude, "roblox-studio-workflow", "SKILL.md"), "a", encoding="utf-8") as f:
            f.write("\nMy own house rule.\n")
        got = rbxskills.import_external()
        self.assertEqual(got["pending"], ["roblox-studio-workflow"], "an edited copy is a real skill of the user's")
        [d] = rbxskills.pending()
        self.assertEqual(d["a"], "workflow")
        self.assertNotIn("workflow", rbxskills.native_ids())

    def test_running_from_source_user_skills_are_not_kit(self):
        """From source the user's library is <KIT>/skills: its skills are the user's, not the kit's."""
        user = os.path.join(self.home, "skills")
        with mock.patch.object(rbxskills, "KIT", self.home), mock.patch.object(rbxskills, "user_dir", lambda: user):
            for name, desc in (("notes", "Take meeting notes."), ("tips", "Share cooking tips."),
                               ("ideas-list", "Collect ideas for garden planting.")):
                skill(os.path.join(user, name), name, desc, "Mine.")
            for name, desc in (("notes", "Take meeting notes."), ("tips", "Share cooking tips."),
                               ("ideas", "Collect ideas for garden planting.")):
                skill(os.path.join(self.agents, name), name, desc, "Theirs.")
            rbxskills.import_external()
            open_ = {d["b"]: d for d in rbxskills.pending()}
            self.assertEqual(sorted(open_), ["ideas", "notes", "tips"])
            self.assertEqual(rbxskills._kit_owner(open_["notes"]["a_dir"]), ("user", False))
            rbxskills.decide(open_["notes"]["id"], "replace")
            rbxskills.decide(open_["tips"]["id"], "merge", "---\nname: tips\ndescription: Merged tips.\n---\nMerged.\n")
            rbxskills.decide(open_["ideas"]["id"], "keep")
            lib = {s["id"]: s for s in rbxskills.library()}
            self.assertEqual(sorted(lib), ["ideas-list", "notes", "tips"])
            self.assertIn("Theirs.", read(os.path.join(user, "notes", "SKILL.md")))
            self.assertEqual(lib["tips"]["description"], "Merged tips.")
            self.assertEqual(lib["ideas-list"]["supersedes"], ["ideas"])
            for name in ("notes", "tips", "ideas"):
                self.assertTrue(os.path.isdir(os.path.join(user, ".replaced", name)), name)

    def core_guide(self, body):
        """The kit's workflow guide (Core), whose Claude Code copy install.py names roblox-studio-workflow."""
        return self.kit_skill("workflow", "Run the Roblox studio workflow for every game task.", owner="app", body=body)

    GUIDE = ("## Setup\n\nOpen the place, start the plugin and run {{RBXKIT}}/rbx.py doctor before anything else.\n\n"
             "## Building\n\nBuild each system as a module with a server part and a client part, tested alone.\n\n"
             "## Testing\n\nPlaytest with two clients, watch the output window, and fix every warning you see.\n\n"
             "## Saving\n\nCommit the place file and the scripts together, with a short note of what changed.\n\n"
             "## Handover\n\nWrite what is done, what is left and what broke into the handover notes for tomorrow.\n")

    def old_copy(self, body, kit_path="/Applications/Old Fridae/kit"):
        """A copy an older Fridae installed: the Claude name, the kit path filled in, no marker."""
        d = os.path.join(self.claude, "roblox-studio-workflow")
        skill(d, "roblox-studio-workflow", "Run the Roblox studio workflow for every game task.",
              body.replace("{{RBXKIT}}", kit_path).strip())
        return d

    def test_installed_copy_with_marker_is_linked(self):
        import install
        self.core_guide("Changed since: run {{RBXKIT}}/rbx.py first.")
        install.build_claude_skills(self.claude, core=os.path.join(self.kit, "vault", "Fridae", "Guides", "Core"))
        d = os.path.join(self.claude, "roblox-studio-workflow")
        mark = json.loads(read(os.path.join(d, ".fridae-installed.json")))
        self.assertEqual((mark["skill"], mark["hash"]), ("workflow", rbxskills.folder_hash(d)))
        self.core_guide("The guide changed again after this install.")  # no longer the current install output
        got = rbxskills.import_external()
        self.assertEqual((got["linked"], got["pending"], got["added"]), (["roblox-studio-workflow"], [], []))
        self.assertIn("workflow", rbxskills.native_ids())

    def test_installed_copy_with_marker_but_edited_is_the_users(self):
        import install
        self.core_guide("Run {{RBXKIT}}/rbx.py first.")
        install.build_claude_skills(self.claude, core=os.path.join(self.kit, "vault", "Fridae", "Guides", "Core"))
        with open(os.path.join(self.claude, "roblox-studio-workflow", "SKILL.md"), "a", encoding="utf-8") as f:
            f.write("\nMy own house rule.\n")
        got = rbxskills.import_external()
        self.assertEqual((got["linked"], got["pending"]), ([], ["roblox-studio-workflow"]))

    def test_old_installed_copy_without_marker_is_linked(self):
        self.core_guide(self.GUIDE)
        self.old_copy(self.GUIDE.replace("fix every warning you see", "note every error the console prints"))
        got = rbxskills.import_external()
        self.assertEqual((got["linked"], got["pending"], got["added"]), (["roblox-studio-workflow"], [], []))
        self.assertEqual(self.ids(), ["workflow"])

    def test_heavily_edited_old_copy_is_the_users(self):
        self.core_guide(self.GUIDE)
        self.old_copy("## My way\n\nI sketch levels on paper, then block them out with parts and invite friends to "
                      "try the obby before adding any coins, shops or pets.\n")
        got = rbxskills.import_external()
        self.assertEqual((got["linked"], got["pending"]), ([], ["roblox-studio-workflow"]))

    def test_malformed_name_falls_back_to_folder(self):
        d = os.path.join(self.user, "folder-name")
        os.makedirs(d)
        with open(os.path.join(d, "SKILL.md"), "w") as f:
            f.write("---\nname: [oops\ndescription: x\n---\nBody.\n")
        [s] = rbxskills.library()
        self.assertEqual((s["id"], s["name"], s["description"]), ("folder-name", "folder-name", "x"))

    def test_similar(self):
        a = {"id": "brainstorming", "name": "brainstorming", "description": "Explore ideas.", "examples": []}
        self.assertEqual(rbxskills.similar(a, dict(a, description="Something else entirely.")), 1.0, "same name")
        far = {"id": "tax-notes", "name": "tax-notes", "description": "Keep receipts tidy for tax time.", "examples": []}
        self.assertLess(rbxskills.similar(a, far), rbxskills.OVERLAP)
        near = dict(far, id="tax-folder", name="tax-folder")
        self.assertGreaterEqual(rbxskills.similar(far, near), rbxskills.OVERLAP, "same description")
        self.assertEqual(rbxskills.OVERLAP, 0.60)

    def test_catalog_imports_at_most_once_a_minute(self):
        p = mock.patch.object(rbxskills, "CORE", ("first", "second"))
        p.start()
        self.addCleanup(p.stop)
        skill(os.path.join(self.agents, "first"), "first", "Fold paper cranes.")
        self.assertIn("- first:", rbxskills.catalog("gemini"))
        skill(os.path.join(self.agents, "second"), "second", "Brew green tea.")
        self.assertNotIn("- second:", rbxskills.catalog("gemini"), "not again within a minute")
        stamp = os.path.join(self.user, ".imported")
        old = os.path.getmtime(stamp) - 61
        os.utime(stamp, (old, old))
        self.assertIn("- second:", rbxskills.catalog("gemini"))

    def test_the_apps_catalog_call_never_imports(self):
        """The app asks for the catalog on the turn (--no-import): the import is the background index load's."""
        p = mock.patch.object(rbxskills, "CORE", ("first",))
        p.start()
        self.addCleanup(p.stop)
        skill(os.path.join(self.agents, "first"), "first", "Fold paper cranes.")
        with mock.patch.object(rbxskills, "import_external", side_effect=AssertionError("imported on the turn")):
            self.assertNotIn("- first:", rbxskills.catalog("gemini", do_import=False))
            self.assertNotIn("- first:", self.cli("catalog", "--ai", "gemini", "--no-import"))
        self.assertFalse(os.path.exists(os.path.join(self.user, ".imported")), "no import ran")
        self.assertIn("- first:", self.cli("catalog", "--ai", "gemini"), "the CLI's default still imports")

    def cli(self, *argv):
        import argparse
        import contextlib
        import io
        ap = argparse.ArgumentParser()
        rbxskills.register(ap.add_subparsers(dest="cmd", required=True))
        a = ap.parse_args(["skills"] + list(argv))
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            a.fn(a)
        return buf.getvalue()

    def test_cli_import_pending_decide_index(self):
        skill(os.path.join(self.claude, "notes"), "notes", "Take meeting notes.", "Claude's.")
        skill(os.path.join(self.agents, "notes"), "notes", "Take meeting notes.", "Agents'.")
        self.assertEqual(json.loads(self.cli("import"))["pending"], ["notes"])
        [d] = json.loads(self.cli("pending", "--json"))
        index = {s["id"]: s for s in json.loads(self.cli("index", "--json"))}
        self.assertTrue(index["notes"]["claude_native"], "an unedited import of Claude Code's own skill")
        for key in ("id", "name", "description", "examples", "applies", "core", "owner", "source", "licence",
                    "supersedes", "updated", "dir", "origin", "pinned"):
            self.assertIn(key, index["notes"])
        merged = os.path.join(self.home, "merged.md")
        with open(merged, "w", encoding="utf-8") as f:
            f.write("---\nname: notes\ndescription: Merged notes.\n---\nMerged.\n")
        out = json.loads(self.cli("decide", d["id"], "merge", "--merged-file", merged, "--json"))
        self.assertEqual((out["ok"], out["decision"]["choice"]), (True, "merge"))
        self.assertEqual(json.loads(self.cli("pending", "--json")), [])
        index = {s["id"]: s for s in json.loads(self.cli("index", "--json"))}
        self.assertFalse(index["notes"]["claude_native"], "merged: no longer Claude Code's copy")
        bad = json.loads(self.cli("decide", "nope", "keep", "--json"))
        self.assertFalse(bad["ok"])

    def advice_file(self, rec):
        path = os.path.join(self.home, "advice.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(rec, f)
        return path

    def test_advise_only_stores_the_recommendation(self):
        d = self.overlap()
        before = sorted(os.listdir(self.user))
        rec = {"choice": "replace", "why": "The newcomer covers more.", "merged": "", "by": "GLM 5 Flash"}
        out = json.loads(self.cli("decide", d["id"], "advise", "--json-file", self.advice_file(rec), "--json"))
        self.assertTrue(out["ok"])
        [p] = rbxskills.pending()
        self.assertEqual(p["state"], "open", "still waiting for the user")
        self.assertEqual({k: p["recommendation"][k] for k in ("choice", "why", "merged", "by")}, rec)
        self.assertTrue(p["recommendation"]["made"])
        self.assertEqual(sorted(os.listdir(self.user)), before, "no folder moved")
        self.assertEqual(self.ids(), ["brainstorm-ideas"])
        # a merge without its text, or an unknown choice, keeps the why but no choice
        out = json.loads(self.cli("decide", d["id"], "advise", "--json-file",
                                  self.advice_file({"choice": "merge", "why": "Both.", "merged": " "}), "--json"))
        self.assertEqual((out["ok"], rbxskills.pending()[0]["recommendation"]["choice"]), (True, None))
        rbxskills.advise(d["id"], {"choice": "delete", "why": "Gone."})
        self.assertIsNone(rbxskills.pending()[0]["recommendation"]["choice"])
        bad = json.loads(self.cli("decide", "nope", "advise", "--json-file", self.advice_file(rec), "--json"))
        self.assertFalse(bad["ok"])

    def recommend_merge(self, d, merged):
        p = next(x for x in rbxskills.pending() if x["id"] == d["id"])
        self.assertTrue(p["a_hash"] and p["b_hash"], "pending gives each side's hash as it is now")
        rbxskills.advise(d["id"], {"choice": "merge", "why": "Both.", "merged": merged, "by": "GLM",
                                   "a_hash": p["a_hash"], "b_hash": p["b_hash"]})
        rec = next(x for x in rbxskills.pending() if x["id"] == d["id"])["recommendation"]
        self.assertEqual((rec["a_hash"], rec["b_hash"]), (p["a_hash"], p["b_hash"]), "stored as the advisor read them")

    def test_a_recommended_merge_is_refused_when_a_skill_changed(self):
        d = self.overlap()
        merged = "---\nname: brainstorm-ideas\ndescription: Both, merged.\n---\n\nMerged body.\n"
        self.recommend_merge(d, merged)
        mine = os.path.join(self.user, "brainstorm-ideas", "SKILL.md")
        with open(mine, "a", encoding="utf-8") as f:
            f.write("\nA step the user added after the suggestion.\n")
        with self.assertRaises(ValueError) as e:
            rbxskills.decide(d["id"], "merge", merged)
        self.assertIn("changed since the suggestion", str(e.exception))
        [p] = rbxskills.pending()
        self.assertEqual((p["state"], p["recommendation"]), ("open", None), "open, and the suggestion is written again")
        self.assertIn("A step the user added", read(mine), "nothing was overwritten")
        self.assertFalse(os.path.isdir(os.path.join(self.user, ".replaced")))

    def test_an_unchanged_skill_pair_merges_from_the_recommendation(self):
        d = self.overlap()
        merged = "---\nname: brainstorm-ideas\ndescription: Both, merged.\n---\n\nMerged body.\n"
        self.recommend_merge(d, merged)
        self.assertEqual(rbxskills.decide(d["id"], "merge", merged)["state"], "done")
        self.assertIn("Merged body.", read(os.path.join(self.user, "brainstorm-ideas", "SKILL.md")))

    def test_the_users_own_merge_text_is_not_held_to_the_suggestion(self):
        d = self.overlap()
        self.recommend_merge(d, "---\nname: brainstorm-ideas\ndescription: Suggested.\n---\nSuggested.\n")
        with open(os.path.join(self.user, "brainstorm-ideas", "SKILL.md"), "a", encoding="utf-8") as f:
            f.write("\nEdited.\n")
        mine = "---\nname: brainstorm-ideas\ndescription: My own merge.\n---\nMine, written now.\n"
        self.assertEqual(rbxskills.decide(d["id"], "merge", mine)["state"], "done")

    def test_advise_never_recommends_changing_an_app_skill(self):
        app = self.kit_skill("critic", "Review the game with a critical eye.", owner="app", body="The app's critic.")
        skill(os.path.join(self.agents, "critic"), "critic", "Review the game with a critical eye.", "Another critic.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        for choice in ("replace", "merge"):
            rbxskills.advise(d["id"], {"choice": choice, "why": "Mine is better.", "merged": "---\nname: critic\n---\nX"})
            rec = rbxskills.pending()[0]["recommendation"]
            self.assertEqual((rec["choice"], rec["merged"]), ("keep", ""), "dropped to keep")
            self.assertNotIn("Mine is better.", rec["why"], "the why argued for the other choice")
            self.assertIn("Fridae's own skill is kept", rec["why"])
        self.assertIn("The app's critic.", read(os.path.join(app, "SKILL.md")))

    def store(self, edit):
        """Change the stored decisions as another copy of the app might have written them."""
        path = os.path.join(self.user, rbxskills.DECISIONS)
        decs = json.loads(read(path))
        edit(decs)
        with open(path, "w", encoding="utf-8") as f:
            json.dump(decs, f)

    def test_a_decision_on_a_folder_outside_the_library_moves_and_writes_nothing(self):
        """A decision written by another copy of the app (its own kit) shares the store: a side that is neither this
        app's skill nor in the user's library is never moved or written."""
        d = self.overlap()
        other = os.path.join(self.home, "Downloads", "Old Fridae.app", "kit", "Guides", "Core", "brainstorm-ideas")
        skill(other, "brainstorm-ideas", "Turn rough ideas into designs before building anything.", "Another app's.")
        before = read(os.path.join(other, "SKILL.md"))
        merged = "---\nname: brainstorm-ideas\ndescription: Both, merged.\n---\n\nMerged body.\n"
        self.store(lambda decs: decs[0].update(a_dir=other))
        for choice in ("replace", "merge"):
            with self.assertRaises(ValueError) as e:
                rbxskills.decide(d["id"], choice, merged)
            self.assertIn("check them again", str(e.exception))
        self.assertEqual(read(os.path.join(other, "SKILL.md")), before, "never written")
        self.assertEqual(sorted(os.listdir(other)), ["SKILL.md"])
        self.assertTrue(os.path.isdir(d["b_dir"]), "the newcomer stays waiting")
        self.assertFalse(os.path.isdir(os.path.join(self.user, ".replaced")))
        self.assertEqual(len(rbxskills.pending()), 1, "still open")
        # the newcomer's side outside the library: nothing that would move it
        self.store(lambda decs: decs[0].update(a_dir=os.path.join(self.user, "brainstorm-ideas"), b_dir=other))
        for choice in ("replace", "merge", "keep_both"):
            with self.assertRaises(ValueError) as e:
                rbxskills.decide(d["id"], choice, merged)
            self.assertIn("check them again", str(e.exception))
        self.assertEqual(read(os.path.join(other, "SKILL.md")), before)
        self.assertIn("Mine.", read(os.path.join(self.user, "brainstorm-ideas", "SKILL.md")))
        # keep closes it, still touching neither foreign folder
        self.store(lambda decs: decs[0].update(a_dir=other, b_dir=d["b_dir"]))
        self.assertEqual(rbxskills.decide(d["id"], "keep")["state"], "done")
        self.assertEqual(read(os.path.join(other, "SKILL.md")), before, "no supersedes written into it")
        self.assertEqual(rbxskills.pending(), [])

    def test_a_decision_whose_waiting_folder_is_gone_closes(self):
        two = os.path.join(self.agents, "two")
        skill(two, "two", "Tune guitar strings.", "Old two.")
        rbxskills.import_external()
        with open(os.path.join(self.user, "two", "SKILL.md"), "a", encoding="utf-8") as f:
            f.write("My own tip.\n")
        skill(two, "two", "Tune guitar strings.", "New two.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        shutil.rmtree(d["b_dir"])
        for choice in ("replace", "merge"):
            with self.assertRaises(LookupError):
                rbxskills.decide(d["id"], choice, "---\nname: two\n---\nMerged.")
        self.assertEqual(rbxskills.decide(d["id"], "keep")["state"], "done", "keep closes it anyway")
        self.assertIn("My own tip.", read(os.path.join(self.user, "two", "SKILL.md")))
        # another update, its folder gone before anyone decides: pending closes it
        skill(two, "two", "Tune guitar strings.", "Newer two.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        shutil.rmtree(d["b_dir"])
        self.assertEqual(rbxskills.pending(), [], "left out, and the count drops")
        stored = {x["id"]: x for x in json.loads(read(os.path.join(self.user, rbxskills.DECISIONS)))}
        self.assertEqual(stored[d["id"]]["state"], "gone", "closed in the store")
        skill(two, "two", "Tune guitar strings.", "Newest two.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        shutil.rmtree(d["b_dir"])
        self.assertEqual(rbxskills.decide(d["id"], "keep_both")["state"], "done", "keep both closes it too")
        self.assertEqual(self.ids(), ["two"])

    def test_a_merge_keeps_the_fridae_fields_of_the_skill_in_use(self):
        mine = os.path.join(self.user, "brainstorm-ideas")
        os.makedirs(mine)
        with open(os.path.join(mine, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: brainstorm-ideas\ndescription: Turn rough ideas into designs before building anything.\n"
                    "examples:\n  - plan a feature, then build it\n  - shape a rough idea\napplies: plain\ncore: true\n"
                    "owner: community\nsource: github.com/me/skills\nlicence: MIT\nsupersedes: [old-ideas]\n---\n\nMine.\n")
        skill(os.path.join(self.agents, "brainstorming"), "brainstorming",
              "Turn rough ideas into designs before building anything.", "Theirs.")
        rbxskills.import_external()
        [d] = rbxskills.pending()
        rbxskills.decide(d["id"], "merge", "---\nname: brainstorm-ideas\ndescription: Both, merged.\napplies: both\n---\n\nMerged.\n")
        [s] = rbxskills.library()
        self.assertEqual((s["description"], s["applies"]), ("Both, merged.", "both"), "the merged text's own keys win")
        self.assertEqual(s["examples"], ["plan a feature, then build it", "shape a rough idea"])
        self.assertEqual((s["core"], s["owner"], s["source"], s["licence"]),
                         (True, "community", "github.com/me/skills", "MIT"))
        self.assertEqual(s["supersedes"], ["old-ideas", "brainstorming"])

    def test_an_imported_skill_cannot_pin_itself_claim_fridae_or_hide_a_guide(self):
        self.kit_skill("ui-design", "Design game menus and HUD screens.")
        src = os.path.join(self.agents, "tidy-desk")
        os.makedirs(src)
        with open(os.path.join(src, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: tidy-desk\ndescription: Sort the papers on a desk into piles.\ncore: true\n"
                    "owner: shipped\nsupersedes: [ui-design]\n---\n\nSort.\n")
        rbxskills.import_external()
        lib = {s["id"]: s for s in rbxskills.library()}
        self.assertIn("ui-design", lib, "a guide stays visible")
        self.assertEqual((lib["tidy-desk"]["core"], lib["tidy-desk"]["owner"], lib["tidy-desk"]["supersedes"]),
                         (False, "user", []))
        self.assertFalse(rbxskills.pinned(lib["tidy-desk"]))
        self.assertNotIn("- tidy-desk:", rbxskills.catalog("gemini"))
        # the user's own skill (no import link) says what it likes
        skill(os.path.join(self.user, "mine"), "mine", "My own.")
        with open(os.path.join(self.user, "mine", "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: mine\ndescription: My own.\ncore: true\nsupersedes: [ui-design]\n---\n\nMine.\n")
        lib = {s["id"]: s for s in rbxskills.library()}
        self.assertTrue(lib["mine"]["core"])
        self.assertNotIn("ui-design", lib)

    def test_refreshing_an_unedited_copy_keeps_the_files_the_user_added(self):
        src = os.path.join(self.agents, "env-skill")
        skill(src, "env-skill", "Talk to a web service.", "Old.")
        rbxskills.import_external()
        copy = os.path.join(self.user, "env-skill")
        extra = {".env": "TOKEN=mine\n", ".venv/bin/python": "#!", "node_modules/x/index.js": "1",
                 "scripts/__pycache__/a.pyc": "c"}
        for rel, text in extra.items():
            path = os.path.join(copy, *rel.split("/"))
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w") as f:
                f.write(text)
        skill(src, "env-skill", "Talk to a web service.", "New.")
        self.assertEqual(rbxskills.import_external()["refreshed"], ["env-skill"], "still an unedited copy")
        self.assertIn("New.", read(os.path.join(copy, "SKILL.md")))
        for rel, text in extra.items():
            self.assertEqual(read(os.path.join(copy, *rel.split("/"))), text, rel)
        link = json.loads(read(os.path.join(copy, rbxskills.SOURCE_FILE)))
        self.assertEqual(link["hash"], rbxskills.folder_hash(copy), "still unedited after the refresh")
        self.assertFalse([n for n in os.listdir(self.user) if n.startswith(".tmp-")], "no temporary folder left")


if __name__ == "__main__":
    unittest.main()


class OverviewWordingTests(unittest.TestCase):
    def test_the_overview_count_is_called_what_it_counts(self):
        """The count of open decisions (skOpen) holds note decisions too: the Overview's line must not call them all
        skill decisions."""
        with open(os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "ui", "index.html"),
                  encoding="utf-8") as f:
            ui = f.read()
        paint = ui[ui.index("function skPaintCount()"):]
        paint = paint[:paint.index("\n}\n")]
        self.assertIn("skOpen().length", paint)
        self.assertNotIn("skill decision", paint)
        self.assertIn('" decisions wait"', paint)


class NoteDecideTests(unittest.TestCase):
    """Deciding a note decision (kind "note", made by the notes upkeep): the losing note goes to the notes folder's
    .trash as rbx_vault 'forget' moves it; nothing is deleted."""

    def setUp(self):
        import rbxnotes
        import rbxpaths
        self.notes_mod = rbxnotes
        self.tmp = tempfile.mkdtemp(prefix="note decide é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        for p in (mock.patch.object(rbxpaths, "HOME_DIR", self.home),
                  mock.patch.object(rbxpaths, "WORK", os.path.join(self.tmp, "work")),
                  mock.patch.object(rbxskills, "user_dir", lambda: os.path.join(self.home, "skills")),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "FRIDAE_RAG_FILE": ""})):
            p.start()
            self.addCleanup(p.stop)
        self.notes = os.path.join(self.home, "notes")
        self.trash = os.path.join(self.notes, ".trash")
        self.a = self.note("Hit freeze", "hit freeze 80-150 ms")
        self.b = self.note("Hit freeze timing", "hit freeze 66 ms")
        out = rbxskills.add_note_decisions([{
            "kind": "note", "relation": "conflict", "a": "Hit freeze", "b": "Hit freeze timing", "a_dir": self.a,
            "b_dir": self.b, "reason": "numbers differ", "lines": {"a": "hit freeze 80-150 ms", "b": "hit freeze 66 ms"},
            "source_key": "note:hit freeze.md|hit freeze timing.md"}])
        self.assertEqual(out, {"added": 1, "open": 1})
        self.id = rbxskills.pending()[0]["id"]

    def note(self, title, body):
        os.makedirs(self.notes, exist_ok=True)
        path = os.path.join(self.notes, title + ".md")
        with open(path, "w", encoding="utf-8") as f:
            f.write("---\ntitle: %s\nwhen: 2026-01-05 10:00\nscope: all\n---\n\n# %s\n\n%s\n" % (title, title, body))
        return path

    def read(self, path):
        with open(path, encoding="utf-8") as f:
            return f.read()

    def trashed(self):
        return sorted(os.listdir(self.trash)) if os.path.isdir(self.trash) else []

    def test_keep_moves_the_second_to_the_trash(self):
        dec = rbxskills.decide(self.id, "keep")
        self.assertEqual((dec["state"], dec["choice"]), ("done", "keep"))
        self.assertTrue(os.path.isfile(self.a))
        self.assertFalse(os.path.exists(self.b))
        self.assertEqual(self.trashed(), ["Hit freeze timing.md"])
        self.assertEqual(rbxskills.pending(), [])

    def test_replace_moves_the_first_to_the_trash(self):
        rbxskills.decide(self.id, "replace")
        self.assertFalse(os.path.exists(self.a))
        self.assertTrue(os.path.isfile(self.b))
        self.assertEqual(self.trashed(), ["Hit freeze.md"])

    def test_merge_writes_the_merged_note_and_trashes_the_other(self):
        with self.assertRaises(ValueError):
            rbxskills.decide(self.id, "merge", "  ")
        rbxskills.decide(self.id, "merge", "Hit freeze is 66 ms for light hits and 80-150 ms for heavy ones.")
        text = self.read(self.a)
        self.assertIn("title: Hit freeze", text, "the first note keeps its front matter")
        self.assertIn("# Hit freeze\n\nHit freeze is 66 ms for light hits", text)
        self.assertNotIn("hit freeze 80-150 ms\n", text)
        self.assertFalse(os.path.exists(self.b))
        self.assertEqual(self.trashed(), ["Hit freeze timing.md", "Hit freeze.md"],
                         "the first note's old text is in the trash too")
        self.assertIn("hit freeze 80-150 ms", self.read(os.path.join(self.trash, "Hit freeze.md")))
        self.assertIn("hit freeze 66 ms", self.read(os.path.join(self.trash, "Hit freeze timing.md")))

    def test_keep_both_moves_nothing(self):
        rbxskills.decide(self.id, "keep_both")
        self.assertTrue(os.path.isfile(self.a) and os.path.isfile(self.b))
        self.assertEqual(self.trashed(), [])

    def test_a_note_outside_the_users_notes_is_refused(self):
        with open(os.path.join(self.home, "skills", ".decisions.json"), encoding="utf-8") as f:
            decs = json.load(f)
        elsewhere = os.path.join(self.tmp, "elsewhere.md")
        with open(elsewhere, "w", encoding="utf-8") as f:
            f.write("x")
        decs[0]["b_dir"] = elsewhere
        rbxskills._write_json(os.path.join(self.home, "skills", ".decisions.json"), decs)
        with self.assertRaises(ValueError):
            rbxskills.decide(self.id, "keep")
        self.assertTrue(os.path.isfile(elsewhere))

    def advise_merge(self, id, merged):
        p = next(x for x in rbxskills.pending() if x["id"] == id)
        rbxskills.advise(id, {"choice": "merge", "why": "Both.", "merged": merged, "by": "GLM",
                              "a_hash": p["a_hash"], "b_hash": p["b_hash"]})

    def test_sibling_merges_never_drop_the_first_merge(self):
        """Three copies give a-b and a-c; Accept all settles both merges in turn. The second's text was written
        from the old a, so it is refused and a keeps what the first merge put in it."""
        c = self.note("Hit freeze heavy", "hit freeze 150 ms on heavy hits")
        rbxskills.add_note_decisions([{
            "kind": "note", "relation": "conflict", "a": "Hit freeze", "b": "Hit freeze heavy", "a_dir": self.a,
            "b_dir": c, "reason": "numbers differ", "lines": {}, "source_key": "note:conflict:a|c"}])
        ac = next(d["id"] for d in rbxskills.pending() if d["b"] == "Hit freeze heavy")
        self.advise_merge(self.id, "Light hits freeze 66 ms (B's fact); heavy ones 80-150 ms.")
        self.advise_merge(ac, "Heavy hits freeze 150 ms (C's fact); others 80-150 ms.")
        # Accept all, in the order the decisions are listed
        for d in rbxskills.pending():
            try:
                rbxskills.decide(d["id"], "merge", d["recommendation"]["merged"])
            except ValueError as e:
                self.assertEqual(d["id"], ac)
                self.assertIn("changed since the suggestion", str(e))
        self.assertIn("B's fact", self.read(self.a), "the first merge stays in the live note")
        self.assertNotIn("C's fact", self.read(self.a))
        [left] = rbxskills.pending()
        self.assertEqual((left["id"], left["recommendation"]), (ac, None), "asked again, with a fresh suggestion")
        self.assertTrue(os.path.isfile(c), "the third note was not trashed")

    def test_a_hand_edit_after_the_suggestion_refuses_its_merge(self):
        self.advise_merge(self.id, "Both, merged.")
        with open(self.a, "a", encoding="utf-8") as f:
            f.write("An agent's replace, after the suggestion.\n")
        with self.assertRaises(ValueError):
            rbxskills.decide(self.id, "merge", "Both, merged.")
        self.assertIn("An agent's replace", self.read(self.a))
        self.assertTrue(os.path.isfile(self.b))
        self.assertIsNone(rbxskills.pending()[0]["recommendation"])

    def test_an_unchanged_note_pair_merges_from_the_suggestion(self):
        self.advise_merge(self.id, "Both, merged.")
        self.assertEqual(rbxskills.decide(self.id, "merge", "Both, merged.")["state"], "done")
        self.assertIn("Both, merged.", self.read(self.a))

    def test_a_suggestion_stored_without_hashes_takes_them_when_stored(self):
        rbxskills.advise(self.id, {"choice": "keep", "why": "x"})
        rec = rbxskills.pending()[0]["recommendation"]
        self.assertEqual(rec["a_hash"], rbxskills.side_hash(self.a))

    def test_advice_on_a_note_is_stored(self):
        dec = rbxskills.advise(self.id, {"choice": "merge", "why": "Both hold.", "merged": "Both.", "by": "GLM"})
        self.assertEqual((dec["recommendation"]["choice"], dec["recommendation"]["merged"]), ("merge", "Both."))
        self.assertEqual(dec["state"], "open")
