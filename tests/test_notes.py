"""The user's notes (rbxnotes.py, rbx_vault actions note/notes/forget/rule_edit, shelves yours and game).

Every test points HOME_DIR and WORK at temporary folders: the real home folder is never touched.
"""
import os
import shutil
import tempfile
import threading
import unittest
from unittest import mock

import rbxkit_mcp
import rbxnotes
import rbxpaths
import rbxvault
from rbxpaths import KitError


class NotesTestCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="notes é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home é")
        self.work = os.path.join(self.tmp, "work é")
        self.project = os.path.join(self.tmp, "My Gäme é")
        os.makedirs(os.path.join(self.project, ".fridae"))
        for p in (mock.patch.object(rbxpaths, "HOME_DIR", self.home), mock.patch.object(rbxpaths, "WORK", self.work),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "RBXKIT_AGENT_NAME": "UI expert"})):
            p.start()
            self.addCleanup(p.stop)

    def in_project(self):
        return mock.patch.dict(os.environ, {"RBXKIT_PROJECT": self.project})

    def vault(self, action, **a):
        return rbxvault.run(action, dict(a, action=action))

    def tool(self, **a):
        res = rbxkit_mcp.t_vault(a)
        return res["isError"], res["content"][0]["text"]

    def files(self, folder):
        return sorted(f for f in os.listdir(folder) if f.endswith(".md")) if os.path.isdir(folder) else []


class WriteTests(NotesTestCase):
    def test_note_scope_defaults_and_front_matter(self):
        out = self.vault("note", text="User wants thin outlines on every button, not thick black strokes.")
        self.assertIn("everything", out)
        notes = os.path.join(self.home, "notes")
        [name] = self.files(notes)
        self.assertEqual(name, "User wants thin outlines on every button, not….md")
        with open(os.path.join(notes, name), encoding="utf-8") as f:
            text = f.read()
        for line in ("scope: all", "by: UI expert", "when: ", "thick black strokes"):
            self.assertIn(line, text)
        with self.in_project():
            out = self.vault("note", text="The shop uses the lantern palette.", title="Shop palette")
        self.assertIn("this project", out)
        self.assertEqual(self.files(os.path.join(self.project, ".fridae", "notes")), ["Shop palette.md"])
        with self.in_project():
            self.vault("note", text="Menus fade, never slide.", scope="all", title="Menu motion")
        self.assertIn("Menu motion.md", self.files(notes))

    def test_caps(self):
        with self.assertRaises(KitError):
            self.vault("note", text="x" * (rbxnotes.NOTE_CAP + 1))
        with self.assertRaises(KitError):
            self.vault("note", text="   ")
        with self.assertRaises(KitError):
            self.vault("note", text="y" * (rbxnotes.RULE_CAP + 1), rule=True)
        with self.assertRaises(KitError) as cm:
            self.vault("note", text="game only", scope="game")
        self.assertIn("has no project", str(cm.exception))
        for n in range(40):
            try:
                self.vault("note", text="Rule number %d: %s" % (n, "keep it plain " * 4), rule=True)
            except KitError as e:
                self.assertIn("rule_edit", str(e))
                break
        else:
            self.fail("the House rules file was never full")
        with open(os.path.join(self.home, "notes", rbxnotes.RULES_NAME), encoding="utf-8") as f:
            self.assertLessEqual(len(f.read()), rbxnotes.RULES_FILE_CAP)

    def test_rule_goes_to_house_rules_not_a_note(self):
        isError, out = self.tool(action="note", text="Always ask for my Illustrator exports first.", rule="true")
        self.assertFalse(isError, out)
        notes = os.path.join(self.home, "notes")
        self.assertEqual(self.files(notes), [rbxnotes.RULES_NAME])
        with open(os.path.join(notes, rbxnotes.RULES_NAME), encoding="utf-8") as f:
            self.assertEqual(f.read(), "# House rules\n\n- Always ask for my Illustrator exports first.\n")

    def test_titles_are_sanitised(self):
        cases = {"../../etc/passwd": "etc passwd", "..\\..\\boot.ini": "boot.ini", "CON": "Note - CON",
                 "com1.txt": "Note - com1.txt", "con.txt": "Note - con.txt", "LPT¹ draft": "LPT¹ draft",
                 "lpt¹.md": "Note - lpt¹.md", "COM³": "Note - COM³", "House rules": "House rules note", "a:b*c?d\"e<f>g|h": "a b c d e f g h",
                 "  Café menü  ": "Café menü", "...": "Note", "line\nbreak\ttab": "line break tab"}
        for raw, want in cases.items():
            self.assertEqual(rbxnotes.safe_title(raw), want, raw)
        self.assertLessEqual(len(rbxnotes.safe_title("word " * 60)), rbxnotes.TITLE_MAX)
        self.vault("note", text="one", title="../../escape")
        self.vault("note", text="two", title="/abs/escape")
        notes = os.path.join(self.home, "notes")
        self.assertEqual(self.files(notes), ["abs escape.md", "escape.md"])
        self.assertEqual(sorted(os.listdir(self.tmp)), ["My Gäme é", "home é"], "nothing written outside the notes folder")

    def test_duplicates_get_a_number(self):
        # the same title again is refused (rbxupkeep's filing check); with force it gets a number
        self.vault("note", text="same", title="Palette")
        self.assertTrue(self.vault("note", text="same", title="Palette").startswith("Not saved"))
        for _ in range(2):
            self.vault("note", text="same", title="Palette", force=True)
        self.assertEqual(self.files(os.path.join(self.home, "notes")), ["Palette 2.md", "Palette 3.md", "Palette.md"])

    def test_concurrent_writers_lose_nothing(self):
        threads = [threading.Thread(target=self.vault, args=("note",), kwargs={"text": "n%d" % n, "title": "Same", "force": True})
                   for n in range(15)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(len(self.files(os.path.join(self.home, "notes"))), 15)
        self.assertFalse([f for f in os.listdir(os.path.join(self.home, "notes")) if f.startswith(".tmp-")])


class ManageTests(NotesTestCase):
    def test_notes_list_newest_first(self):
        self.assertEqual(self.vault("notes"), "No notes yet.")
        with mock.patch("time.strftime", return_value="2026-11-01 10:00"):
            self.vault("note", text="old one", title="Older")
        with mock.patch("time.strftime", return_value="2026-11-20 10:00"):
            self.vault("note", text="new one", title="Newer")
        out = self.vault("notes")
        self.assertLess(out.index("Newer"), out.index("Older"))
        with self.in_project():
            self.vault("note", text="game", title="Game one")
            self.assertIn("Game one", self.vault("notes"))
            self.assertNotIn("Game one", self.vault("notes", scope="all"))

    def test_forget_moves_to_trash(self):
        self.vault("note", text="wrong idea", title="Wrong idea")
        self.vault("note", text="wrong idea again", title="Wrong idea", force=True)
        # a partial title moves nothing: it lists the candidates
        with self.assertRaises(KitError) as cm:
            self.vault("forget", title="wrong", scope="all")
        self.assertIn("Wrong idea 2", str(cm.exception))
        self.assertEqual(len(self.files(os.path.join(self.home, "notes"))), 2)
        out = self.vault("forget", title="wrong idea", scope="all")
        self.assertIn(".trash", out)
        notes = os.path.join(self.home, "notes")
        self.assertEqual(self.files(notes), ["Wrong idea 2.md"])
        self.assertEqual(self.files(os.path.join(notes, ".trash")), ["Wrong idea.md"])
        self.vault("note", text="third", title="Wrong idea")
        self.vault("forget", title="Wrong idea", scope="all")
        self.assertEqual(self.files(os.path.join(notes, ".trash")), ["Wrong idea 2.md", "Wrong idea.md"])
        with self.assertRaises(KitError):
            self.vault("forget", title="../House rules", scope="all")
        with self.assertRaises(KitError):
            self.vault("forget", title="nothing like this", scope="all")

    def test_rule_edit(self):
        self.vault("note", text="Never use gradients on buttons.", rule=True)
        self.vault("note", text="Always show prices in Robux.", rule=True)
        self.assertEqual(self.vault("rule_edit", old="gradients", text="Never use gradients anywhere.", scope="all"),
                         "House rule changed.")
        self.assertEqual(self.vault("rule_edit", old="robux", text="", scope="all"), "House rule removed.")
        with open(os.path.join(self.home, "notes", rbxnotes.RULES_NAME), encoding="utf-8") as f:
            self.assertEqual(f.read(), "# House rules\n\n- Never use gradients anywhere.\n")
        with self.assertRaises(KitError):
            self.vault("rule_edit", old="zebra", text="x", scope="all")
        self.vault("note", text="Never use bevels.", rule=True)
        with self.assertRaises(KitError) as cm:
            self.vault("rule_edit", old="never use", text="x", scope="all")
        self.assertIn("Several", str(cm.exception))


class SearchTests(NotesTestCase):
    def test_yours_with_no_project(self):
        self.vault("note", text="We chose ProfileStore over raw datastores because session locking is built in.",
                   title="Data library")
        text = self.vault("search", query="which datastore library did we choose", shelf="yours")
        self.assertIn("1. [yours] Data library", text)
        text = self.vault("search", query="which data store library did we choose", k=8)
        self.assertIn("[yours] Data library", text)
        out = self.vault("read", shelf="yours", title="Data library")
        self.assertIn("ProfileStore", out)

    def test_game_shelf_with_a_project(self):
        with self.in_project():
            self.vault("note", text="Lantern glow is warm amber, never blue.", title="Lantern colour")
            text = self.vault("search", query="lantern glow colour", shelf="game")
            self.assertIn("[game] Lantern colour", text)
            self.assertIn("[game] Lantern colour", self.vault("search", query="lantern glow colour", k=8))
        with self.assertRaises(KitError):
            self.vault("search", query="lantern glow colour", shelf="game")

    def test_short_notes_are_not_damped(self):
        self.vault("note", text="Coins reset on rejoin: save on leave.", title="Coins reset")
        hits = rbxvault.search_sections("coins reset rejoin", ["yours"], 3)
        self.assertEqual(hits[0][2]["title"], "Coins reset")
        self.assertLess(hits[0][2]["own"], 120)

    def test_old_studio_lessons_still_read_and_moved(self):
        os.makedirs(self.work)
        old = os.path.join(self.work, rbxnotes.LESSONS_NAME)
        with open(old, "w", encoding="utf-8") as f:
            f.write("- 2026-10-01: Test every menu on a phone emulator.\n")
        text = self.vault("search", query="phone emulator menu", shelf="yours")
        self.assertRegex(text, r"\[yours\] studio-lessons .*\[lesson · agent\]")
        self.assertFalse(os.path.exists(old))
        self.assertTrue(os.path.isfile(os.path.join(self.home, "notes", rbxnotes.LESSONS_NAME)))
        # if it could not be moved, the old place is still read
        with open(old, "w", encoding="utf-8") as f:
            f.write("- 2026-10-02: Old place still read.\n")
        os.remove(os.path.join(self.home, "notes", rbxnotes.LESSONS_NAME))
        with mock.patch("os.replace", side_effect=OSError("locked")):
            self.assertIn(("studio-lessons", old), rbxnotes.yours_files())

    def test_notes_are_never_cached(self):
        self.vault("note", text="Zebra crossings on every road.", title="Zebra")
        self.vault("search", query="zebra", shelf="yours")
        with open(rbxvault.cache_path(rbxvault.GUIDES_CACHE_NAME), encoding="utf-8") as f:
            self.assertNotIn("Zebra", f.read())
        self.vault("forget", title="Zebra", scope="all")
        self.assertIn("No match", self.vault("search", query="zebra crossings", shelf="yours"))


class Round3Tests(NotesTestCase):
    def rules(self, folder=None):
        with open(os.path.join(folder or os.path.join(self.home, "notes"), rbxnotes.RULES_NAME), encoding="utf-8") as f:
            return f.read()

    def test_parallel_rule_writers_lose_nothing(self):
        errors = []

        def add(n):
            try:
                self.vault("note", text="Rule %d." % n, rule=True)
            except KitError as e:
                errors.append(e)
        threads = [threading.Thread(target=add, args=(n,)) for n in range(25)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        self.assertEqual(errors, [])
        text = self.rules()
        for n in range(25):
            self.assertIn("- Rule %d.\n" % n, text)
        self.assertFalse(os.path.exists(os.path.join(self.home, "notes", ".lock")))

    def test_an_overfull_rules_file_can_shrink(self):
        notes = os.path.join(self.home, "notes")
        os.makedirs(notes)
        with open(os.path.join(notes, rbxnotes.RULES_NAME), "w", encoding="utf-8") as f:
            f.write("# House rules\n\n" + "".join("- Hand-written rule %d %s\n" % (n, "x" * 60) for n in range(40)))
        self.assertEqual(self.vault("rule_edit", old="rule 7 ", text="", scope="all"), "House rule removed.")
        self.assertEqual(self.vault("rule_edit", old="rule 8 ", text="Shorter.", scope="all"), "House rule changed.")
        with self.assertRaises(KitError):
            self.vault("rule_edit", old="rule 9 ", text="y" * 200, scope="all")
        with self.assertRaises(KitError):
            self.vault("note", text="One more.", rule=True)

    def test_a_long_rule_is_refused_with_how_to_shorten(self):
        with self.assertRaises(KitError) as cm:
            self.vault("note", text="z " * 200, rule=True)
        self.assertIn("keeping the user's key words", str(cm.exception))
        self.assertIn(str(rbxnotes.RULE_CAP), str(cm.exception))

    @unittest.skipIf(os.name == "nt", "symlinks need extra rights on Windows")
    def test_symlinks_are_not_followed(self):
        outside = os.path.join(self.tmp, "outside")
        os.makedirs(outside)
        with open(os.path.join(outside, "Secret.md"), "w", encoding="utf-8") as f:
            f.write("secret zebra")
        notes = os.path.join(self.home, "notes")
        os.makedirs(notes)
        os.symlink(os.path.join(outside, "Secret.md"), os.path.join(notes, "Linked.md"))
        self.assertEqual(rbxnotes.note_files(notes), [])
        self.assertIn("No match", self.vault("search", query="secret zebra", shelf="yours"))
        # a notes folder that is itself a link elsewhere is refused for writing
        game = os.path.join(self.project, ".fridae")
        os.symlink(outside, os.path.join(game, "notes"))
        with self.in_project(), self.assertRaises(KitError) as cm:
            self.vault("note", text="game note", scope="game")
        self.assertIn("leads somewhere else", str(cm.exception))
        self.assertEqual(sorted(os.listdir(outside)), ["Secret.md"])

    def test_one_shared_word_does_not_promote_a_note(self):
        self.vault("note", text="The lobby music is a calm piano loop.", title="Lobby music")
        text = self.vault("search", query="save player coins between sessions datastore", k=5)
        first = [l for l in text.splitlines() if l[:3] in ("1. ", "2. ", "3. ")]
        self.assertFalse(any("[yours]" in l for l in first), text[:600])
        self.vault("note", text="Player coins are saved with ProfileStore between sessions.", title="Coins saving")
        text = self.vault("search", query="save player coins between sessions", k=5)
        self.assertIn("[yours] Coins saving", text)

    def test_a_fact_only_in_game_notes_is_found(self):
        with self.in_project():
            self.vault("note", text="The boss arena's lava rises every 45 seconds; the user wants it slower on phones.",
                       title="Boss arena lava")
            self.assertIn("[game] Boss arena lava", self.vault("search", query="how fast does the lava rise in the boss arena", shelf="game"))
            self.assertIn("[game] Boss arena lava", self.vault("search", query="how fast does the lava rise in the boss arena"))

    def test_helpers_keep_lessons_to_the_game(self):
        import rbxstage
        with self.in_project(), mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": "worker"}):
            for call in (lambda: self.vault("note", text="x", scope="all"),
                         lambda: self.vault("note", text="Always x.", rule=True, scope="game"),
                         lambda: self.vault("rule_edit", old="x", text="y", scope="game"),
                         lambda: rbxstage.run("lesson", {"project": self.project, "text": "Gaps for JumpPower 50.",
                                                         "scope": "all"})):
                with self.assertRaises(KitError):
                    call()
            rbxstage.run("lesson", {"project": self.project, "text": "Gaps for JumpPower 50 need 12 studs.", "scope": "game"})
            text = self.vault("search", query="JumpPower gaps", shelf="game")
        self.assertRegex(text, r"\[game\] .*\[lesson · this game ")
        self.assertFalse(os.path.exists(os.path.join(self.home, "notes")))
        with self.in_project():  # the Lead or an ordinary chat may save for every game
            rbxstage.run("lesson", {"project": self.project, "text": "Brief experts with PROJECT.md's remotes.", "scope": "all"})
        self.assertRegex(self.vault("search", query="brief experts remotes", shelf="yours"), r"\[yours\] .*\[lesson · ")


class StudioOffTests(unittest.TestCase):
    def test_studio_tools_refuse_when_studio_is_off(self):
        import rbxstudio
        with mock.patch.dict(os.environ, {"FRIDAE_NO_STUDIO": "1", "RBXKIT_STUDIO_MCP": __file__}):
            self.assertIsNone(rbxstudio.launcher())
            with self.assertRaises(rbxstudio.StudioError) as cm:
                with rbxstudio.Studio():
                    pass
            self.assertIn("Studio is off", str(cm.exception))
            res = rbxkit_mcp.t_snippet({"name": "vfx_capture"})
            self.assertTrue(res["isError"])
            self.assertIn("Studio is off", res["content"][0]["text"])


if __name__ == "__main__":
    unittest.main()
