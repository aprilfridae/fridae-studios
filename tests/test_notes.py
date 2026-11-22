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
        self.assertIn("this game", out)
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
        self.assertIn("no game project", str(cm.exception))
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
        cases = {"../../etc/passwd": "etc passwd", "..\\..\\boot.ini": "boot.ini", "CON": "CON note",
                 "com1.txt": "com1.txt note", "House rules": "House rules note", "a:b*c?d\"e<f>g|h": "a b c d e f g h",
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
        for _ in range(3):
            self.vault("note", text="same", title="Palette")
        self.assertEqual(self.files(os.path.join(self.home, "notes")), ["Palette 2.md", "Palette 3.md", "Palette.md"])

    def test_concurrent_writers_lose_nothing(self):
        threads = [threading.Thread(target=self.vault, args=("note",), kwargs={"text": "n%d" % n, "title": "Same"})
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
        self.vault("note", text="wrong idea again", title="Wrong idea")
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
        self.assertIn("[yours] studio-lessons", text)
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


if __name__ == "__main__":
    unittest.main()
