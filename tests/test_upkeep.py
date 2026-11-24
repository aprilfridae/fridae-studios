"""Vault upkeep: superseded notes (front matter supersedes / superseded_by, the sidecar for shipped notes, ranking in
both searches), the checks when a note is filed (near-duplicates, possible contradictions) and the upkeep sweep.

Every test points HOME_DIR and WORK at temporary folders: the real notes folder is never touched.
"""
import hashlib
import json
import os
import random
import shutil
import tempfile
import time
import unittest
from unittest import mock

import rbxnotes
import rbxpaths
import rbxupkeep
import rbxvault
from rbxpaths import KitError


def write(path, text, days_old=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    if days_old is not None:
        t = time.time() - days_old * 86400
        os.utime(path, (t, t))
    return path


def note_text(title, body, **front):
    lines = ["---", "title: %s" % title] + ["%s: %s" % (k, v) for k, v in front.items()] + ["---", "", "# " + title,
                                                                                            "", body, ""]
    return "\n".join(lines)


class UpkeepCase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="upkeep é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        self.work = os.path.join(self.tmp, "work")
        self.project = os.path.join(self.tmp, "Lantern Gäme")
        os.makedirs(os.path.join(self.project, ".fridae"))
        for p in (mock.patch.object(rbxpaths, "HOME_DIR", self.home), mock.patch.object(rbxpaths, "WORK", self.work),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "RBXKIT_AGENT_NAME": "Librarian",
                                               "RBXKIT_AGENT_ROLE": "", "FRIDAE_RAG_FILE": ""})):
            p.start()
            self.addCleanup(p.stop)
        self.notes = os.path.join(self.home, "notes")
        self.game_notes = os.path.join(self.project, ".fridae", "notes")

    def in_project(self):
        return mock.patch.dict(os.environ, {"RBXKIT_PROJECT": self.project})

    def vault(self, action, **a):
        return rbxvault.run(action, dict(a, action=action))

    def mine(self, title, body, folder=None, days_old=None, **front):
        return write(os.path.join(folder or self.notes, title + ".md"), note_text(title, body, **front), days_old)

    def front(self, path):
        return rbxvault.split_front(rbxvault.read(path))[0]


# ------------------------------------------------------------------ front matter
class FrontMatterTests(unittest.TestCase):
    def test_supersedes_forms(self):
        cases = {
            'supersedes: "Old Title"': ["Old Title"],
            "supersedes: Old Title": ["Old Title"],
            'supersedes: ["Old A", "Old, B"]': ["Old A", "Old, B"],
            "supersedes: [Old A, Old B]": ["Old A", "Old B"],
            "supersedes:\n  - Old A\n  - \"Old B\"": ["Old A", "Old B"],
            'supersedes: "[[Old Title|alias]]"': ["Old Title"],
            "supersedes: [[Old Title]]": ["Old Title"],
            "supersedes: Old Title.md": ["Old Title"],
            "supersedes:": [],
        }
        for fm, want in cases.items():
            text = "---\ntitle: New\n%s\nwhen: 2026-11-01\n---\n\nbody\n" % fm
            self.assertEqual(rbxvault.front_titles(text, "supersedes"), want, fm)
        self.assertEqual(rbxvault.front_titles("no front matter\nsupersedes: X\n", "supersedes"), [])
        self.assertEqual(rbxvault.supersession('---\nsuperseded_by: "New One"\n---\nx'), ([], "New One"))

    def test_map_and_precedence(self):
        entries = [{"title": "New", "supersedes": ["Old"]}, {"title": "Old", "superseded_by": "Newest"},
                   {"title": "Self", "supersedes": ["self"]}]
        sup = rbxvault.superseded_map(entries, {"vault note": "Mine"})
        self.assertEqual(sup, {"old": "Newest", "vault note": "Mine"})  # the old note's own mark wins; no self-loops
        # search's map: by identity, a title naming the nearest note of that title
        known = rbxvault.known_titles([("yours", "New"), ("yours", "Old"), ("vault", "Old"), ("yours", "Newest"),
                                       ("yours", "Self"), ("vault", "Vault note"), ("yours", "Mine")])
        sup = rbxvault.supersession_map([("yours", "New", ["Old"], ""), ("yours", "Self", ["self"], "")],
                                        [("Vault note", None, "Mine", "yours"), ("Old", None, "Mine", "yours")], known)
        self.assertEqual(sup, {("yours", "old"): ("New", "yours"), ("vault", "vault note"): ("Mine", "yours"),
                               ("vault", "old"): ("Mine", "yours")})
        sup = rbxvault.supersession_map([("yours", "Old", [], "Newest")], [("Old", "yours", "Mine", "yours")], known)
        self.assertEqual(sup, {("yours", "old"): ("Newest", "yours")})

    def place(self, titles, k, sup, fetchable=()):
        """place_superseded over vault notes named by title, best first: the titles shown."""
        items = [{"title": t} for t in titles]
        out = rbxvault.place_superseded(
            items, k, lambda d: rbxvault.replacement_of(sup, d), lambda d: rbxvault.note_key(d),
            lambda key, old: {"title": key[1].capitalize(), "fetched": True} if key[1] in fetchable else None)
        return [d["title"] + ("*" if d.get("fetched") else "") for d in out]

    def test_the_replacement_takes_the_old_notes_place_and_the_old_note_follows(self):
        sup = {("vault", "old"): ("New", "vault")}
        self.assertEqual(self.place(["A", "Old", "B", "New"], 3, sup), ["A", "New", "Old"])
        # the replacement above already: the old note goes directly after it
        self.assertEqual(self.place(["New", "A", "Old"], 3, sup), ["New", "Old", "A"])
        # not a candidate: fetched into the results, k kept
        self.assertEqual(self.place(["Old", "A", "B"], 3, sup, {"new"}), ["New*", "Old", "A"])
        # a replacement replaced in turn: the newest first
        sup2 = {**sup, ("vault", "new"): ("Newest", "vault")}
        self.assertEqual(self.place(["Old", "A", "New", "Newest"], 4, sup2), ["Newest", "New", "Old", "A"])
        self.assertEqual(self.place(["Old", "A"], 5, sup), ["A", "Old"], "fewer than k: nothing made up")

    def test_an_old_note_without_its_replacement_stays_in_the_first_k(self):
        sup = {("vault", "old"): ("Gone", None), ("vault", "older"): ("Missing", "vault")}
        self.assertEqual(self.place(["Old", "A", "B", "C"], 3, sup), ["A", "B", "Old"])
        self.assertEqual(self.place(["Old", "Older", "A", "B"], 3, sup), ["A", "Old", "Older"])
        self.assertEqual(self.place(["A", "B", "C", "Old"], 3, sup), ["A", "B", "C"], "only the first k count")


# ------------------------------------------------------------------ filing with supersedes
class SupersedeFilingTests(UpkeepCase):
    def test_marks_the_old_note_and_the_new_one_names_it(self):
        self.vault("note", text="The coin cap per player is 500 coins.", title="Coin cap", scope="all")
        out = self.vault("note", text="The coin cap per player is now 900 coins after the economy pass.",
                         title="Coin cap raised", scope="all", supersedes="Coin cap")
        self.assertIn("Coin cap raised", out)
        self.assertIn("replaces 'Coin cap'", out)
        new = os.path.join(self.notes, "Coin cap raised.md")
        self.assertEqual(rbxvault.front_titles(rbxvault.read(new), "supersedes"), ["Coin cap"])
        old = self.front(os.path.join(self.notes, "Coin cap.md"))
        self.assertEqual(old["superseded_by"], "Coin cap raised")
        self.assertRegex(old["superseded_on"], r"^\d{4}-\d{2}-\d{2}$")
        self.assertIn("500 coins", rbxvault.read(os.path.join(self.notes, "Coin cap.md")), "the old note is kept")

    def test_game_note_can_replace_a_general_one_and_several_at_once(self):
        self.vault("note", text="Buttons are square.", title="Button shape", scope="all")
        self.vault("note", text="Buttons use the blue palette.", title="Button colour", scope="all")
        with self.in_project():
            self.vault("note", text="In this game buttons are round and lantern orange.", title="Lantern buttons",
                       scope="game", supersedes=["Button shape", "Button colour"])
        new = os.path.join(self.game_notes, "Lantern buttons.md")
        self.assertEqual(rbxvault.front_titles(rbxvault.read(new), "supersedes"), ["Button shape", "Button colour"])
        # the general notes hold for every other game: the mark is this game's, in its own sidecar (finding 6)
        with open(os.path.join(self.game_notes, rbxnotes.SIDECAR), encoding="utf-8") as f:
            side = json.load(f)
        for t in ("Button shape", "Button colour"):
            self.assertNotIn("superseded_by", self.front(os.path.join(self.notes, t + ".md")))
            self.assertEqual((side[t]["by"], side[t]["shelf"]), ("Lantern buttons", "yours"))
        self.assertFalse(os.path.exists(os.path.join(self.notes, rbxnotes.SIDECAR)))
        with self.in_project():
            out = self.vault("search", query="buttons square", shelf="yours", k=5)
        self.assertIn("(replaced by Lantern buttons)", out)
        other = os.path.join(self.tmp, "Other game")
        os.makedirs(os.path.join(other, ".fridae"))
        for project in (other, ""):
            with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": project}):
                out = self.vault("search", query="buttons square", shelf="yours", k=5)
            self.assertIn("[yours] Button shape", out)
            self.assertNotIn("replaced by", out)

    def test_an_expert_cannot_mark_a_general_note(self):
        self.vault("note", text="Buttons are square.", title="Button shape", scope="all")
        with self.in_project(), mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": "ui"}):
            with self.assertRaises(KitError):
                self.vault("note", text="Round buttons here.", title="Round", scope="game", supersedes="Button shape")
        self.assertFalse(os.path.exists(os.path.join(self.game_notes, "Round.md")), "nothing is written")
        self.assertNotIn("superseded_by", self.front(os.path.join(self.notes, "Button shape.md")))

    def test_shipped_notes_get_the_sidecar_and_stay_untouched(self):
        vault_path = rbxvault.note_files()[0]
        title = os.path.basename(vault_path)[:-3]
        before = hashlib.sha1(open(vault_path, "rb").read()).hexdigest()
        self.vault("note", text="Our own way of doing this, measured in 2026.", title="Our way", scope="all",
                   supersedes=title)
        self.assertEqual(hashlib.sha1(open(vault_path, "rb").read()).hexdigest(), before)
        with open(os.path.join(self.notes, rbxnotes.SIDECAR), encoding="utf-8") as f:
            side = json.load(f)
        self.assertEqual(side[title]["by"], "Our way")
        self.assertEqual(rbxnotes.sidecars(None), {title.casefold(): "Our way"})

    def test_forgetting_the_replacement_takes_its_marks_away(self):
        # review finding 5: the old note stayed marked and ranked down, pointing at a note in the trash
        self.vault("note", title="Throw meter position", scope="all",
                   text="The throw meter sits at the bottom centre of the HUD, 24 px above the safe area.")
        self.vault("note", title="Throw meter v2", scope="all", force=True, supersedes="Throw meter position",
                   text="The throw meter sits on the left edge of the screen now.")
        vault_title = os.path.basename(rbxvault.note_files()[0])[:-3]
        self.vault("note", title="Our way", scope="all", text="Our own way of doing this, measured in 2026.",
                   supersedes=vault_title)
        self.vault("note", title="Keep me", scope="all", text="Kept marks stay.", supersedes="Our way", force=True)
        self.vault("forget", title="Throw meter v2", scope="all")
        old = self.front(os.path.join(self.notes, "Throw meter position.md"))
        self.assertNotIn("superseded_by", old)
        self.assertNotIn("superseded_on", old)
        self.assertEqual(old["title"], "Throw meter position", "the rest of its front matter is kept")
        self.assertIn("24 px above", rbxvault.read(os.path.join(self.notes, "Throw meter position.md")))
        out = self.vault("search", query="throw meter position HUD", shelf="yours", k=5)
        self.assertNotIn("replaced by", out)
        self.vault("forget", title="Our way", scope="all")
        with open(os.path.join(self.notes, rbxnotes.SIDECAR), encoding="utf-8") as f:
            self.assertNotIn(vault_title, json.load(f))
        self.assertEqual(self.front(os.path.join(self.notes, ".trash", "Our way.md"))["superseded_by"], "Keep me",
                         "a mark on the note forgotten is not another note's")
        # a game's note that replaced a general note: its entry in the game's sidecar goes with it
        self.vault("note", text="Buttons are square and outlined.", title="Button shape", scope="all")
        with self.in_project():
            self.vault("note", text="Round lantern buttons.", title="Lantern buttons", scope="game",
                       supersedes="Button shape")
            self.vault("forget", title="Lantern buttons", scope="game")
            out = self.vault("search", query="buttons square outlined", shelf="yours", k=5)
        self.assertNotIn("replaced by", out)
        with open(os.path.join(self.game_notes, rbxnotes.SIDECAR), encoding="utf-8") as f:
            self.assertEqual(json.load(f), {})

    def test_unknown_or_own_title_is_refused_before_writing(self):
        with self.assertRaises(KitError) as cm:
            self.vault("note", text="x y z", title="New", scope="all", supersedes="No such note anywhere")
        self.assertIn("No note called", str(cm.exception))
        self.assertFalse(os.path.isdir(self.notes) and os.listdir(self.notes))
        with self.assertRaises(KitError):
            self.vault("note", text="x y z", title="Same", scope="all", supersedes="Same")

    def test_replace_with_supersedes_updates_in_place(self):
        self.vault("note", text="Max 4 players per server.", title="Server size", scope="all")
        self.vault("note", text="Each server holds a small group.", title="Server notes", scope="all")
        self.vault("note", text="Max 8 players per server since the update.", title="Server notes", scope="all",
                   replace=True, supersedes="Server size")
        self.assertEqual(sorted(os.listdir(self.notes)), ["Server notes.md", "Server size.md"])
        self.assertEqual(self.front(os.path.join(self.notes, "Server size.md"))["superseded_by"], "Server notes")


# ------------------------------------------------------------------ search: keyword
class KeywordSearchTests(UpkeepCase):
    def test_superseded_ranks_below_its_replacement_and_says_so(self):
        self.mine("Coin cap", "The coin cap per player is 500 coins in the shop economy.")
        self.mine("Coin cap raised", "The coin cap per player is now 900 coins in the economy.",
                  supersedes='"Coin cap"')  # as the exam builder writes it: only the newer note says so
        out = self.vault("search", query="coin cap per player", shelf="yours", k=5)
        self.assertIn("Coin cap raised", out)
        self.assertLess(out.index("[yours] Coin cap raised"), out.index("[yours] Coin cap ›"))
        old_line = next(l for l in out.splitlines() if "[yours] Coin cap ›" in l)
        self.assertIn("(replaced by Coin cap raised)", old_line)
        self.assertIn("[lesson", old_line, "the labels are still shown")
        new_line = next(l for l in out.splitlines() if "[yours] Coin cap raised" in l)
        self.assertNotIn("replaced by", new_line)

    def test_superseded_ranks_below_even_when_it_matches_the_words_better(self):
        # the old note holds more of the query's words than its replacement (review finding 1)
        self.mine("Throw meter position", "The throw meter sits at the bottom centre of the HUD, 24 px above the safe "
                  "area.")
        self.mine("Throw meter v2", "The throw meter sits on the left edge of the screen now.",
                  supersedes='"Throw meter position"')
        for q in ("throw meter position HUD", "HUD throw meter safe area"):
            out = self.vault("search", query=q, shelf="yours", k=5)
            self.assertLess(out.index("[yours] Throw meter v2"), out.index("[yours] Throw meter position"), out)
            hits = rbxvault._keyword_passages(q, ["yours"], 3, None, None)  # the Librarian's keyword lookup
            titles = [h["title"] for h in hits]
            self.assertLess(titles.index("Throw meter v2"), titles.index("Throw meter position"), q)

    def test_superseded_by_on_the_old_note_alone_is_enough(self):
        self.mine("Spawn rule", "Players spawn at the lantern shrine.", superseded_by='"Spawn rule two"')
        self.mine("Spawn rule two", "Players spawn at the village gate.")
        out = self.vault("search", query="where do players spawn", shelf="yours", k=5)
        self.assertLess(out.index("Spawn rule two"), out.index("[yours] Spawn rule ›"))
        self.assertIn("(replaced by Spawn rule two)", out)

    def test_never_dropped_when_it_is_the_only_match(self):
        self.mine("Torch fuel", "Torches burn kerosene for ninety seconds.", superseded_by="Lamp oil")
        out = self.vault("search", query="torches kerosene", shelf="yours", k=5)
        self.assertIn("[yours] Torch fuel", out)
        self.assertIn("(replaced by Lamp oil)", out)

    def test_the_replacement_is_fetched_into_the_old_notes_place(self):
        # its words do not match the query, but it is the right answer (review finding 4)
        self.mine("Throw meter position", "The throw meter sits at the bottom centre of the HUD, 24 px above the safe "
                  "area.", superseded_by="Lamp layout")
        self.mine("Lamp layout", "Everything on the left edge now.")
        for n in range(6):
            self.mine("Filler %d" % n, "The HUD shows a throw meter filler %d." % n)
        out = self.vault("search", query="throw meter position HUD safe area", shelf="yours", k=4)
        firsts = [l for l in out.splitlines() if l[:3] in ("1. ", "2. ", "3. ", "4. ", "5. ")]
        self.assertEqual(len(firsts), 4, out)
        self.assertTrue(firsts[0].startswith("1. [yours] Lamp layout"), out)
        self.assertTrue(firsts[1].startswith("2. [yours] Throw meter position"), out)
        self.assertIn("(replaced by Lamp layout)", firsts[1])

    def test_an_old_note_whose_replacement_is_gone_stays_in_the_first_k(self):
        self.mine("Throw meter position", "The throw meter sits at the bottom centre of the HUD, 24 px above the safe "
                  "area.", superseded_by="Trashed note")
        for n in range(6):
            self.mine("Filler %d" % n, "The HUD shows a throw meter position filler %d." % n)
        out = self.vault("search", query="throw meter position HUD safe area", shelf="yours", k=3)
        firsts = [l for l in out.splitlines() if l[:3] in ("1. ", "2. ", "3. ", "4. ")]
        self.assertEqual(len(firsts), 3, out)
        self.assertTrue(firsts[2].startswith("3. [yours] Throw meter position"), out)

    def test_game_notes_and_all_shelves(self):
        self.mine("Lantern colour", "Lanterns glow warm yellow at night in the village.")
        with self.in_project():
            self.mine("Lantern colour here", "Lanterns glow cold blue at night in the village.",
                      folder=self.game_notes, supersedes="Lantern colour")
            out = self.vault("search", query="lanterns glow at night village", k=8, source="user,game")
            self.assertLess(out.index("Lantern colour here"), out.index("[yours] Lantern colour ›"))
            self.assertIn("(replaced by Lantern colour here)", out)
            # over every shelf: the pair stays together, the replacement first
            out = self.vault("search", query="lanterns glow at night village", k=3)
        self.assertIn("[game] Lantern colour here", out)
        self.assertNotIn("[yours] Lantern colour ›", out.split("[game] Lantern colour here")[0])

    def test_a_vault_cache_from_before_the_labels_is_rebuilt(self):
        # an updated app keeps the vault files' times: a cache signed the old way (version and file times only) must
        # not be read as current (review finding 8)
        h = hashlib.sha1(rbxpaths.VERSION.encode())
        for p in rbxvault.note_files() + [os.path.join(rbxvault.KB, "SYMPTOMS.md"), os.path.abspath(rbxvault.__file__)]:
            if os.path.exists(p):
                st = os.stat(p)
                h.update(f"{p}|{st.st_mtime_ns}|{st.st_size}".encode())
        write(rbxvault.cache_path(), json.dumps({"sig": h.hexdigest(), "symptoms": [], "docs": [
            {"title": "Label-less note", "path": "Label-less note.md", "tags": [], "tldr": "", "summary": "", "kb": 1,
             "tf": {f: {} for f in rbxvault.FIELDS}, "len": {f: 0 for f in rbxvault.FIELDS}}]}))
        idx = rbxvault.load_index()
        self.assertNotIn("Label-less note", [d["title"] for d in idx["docs"]])
        self.assertTrue(all("labels" in d and "links" in d for d in idx["docs"]))
        self.assertEqual(rbxvault.load_index()["sig"], idx["sig"], "rebuilt once, then read")

    def test_a_mark_names_one_note_not_every_note_of_that_title(self):
        # the user's own note has a vault note's exact title; a newer note replaces the user's (review finding 7)
        idx = rbxvault.load_index()
        vt = next(d["title"] for d in idx["docs"] if len(d["title"].split()) >= 4 and d["tldr"])
        q = " ".join(vt.split()[:6])
        self.vault("note", title=vt, text="My own take on this topic for my game, short.", scope="all", force=True)
        self.vault("note", title="My newer take", text="Totally different newer take on something else entirely.",
                   scope="all", supersedes=vt, force=True)
        self.assertEqual(self.front(os.path.join(self.notes, vt + ".md"))["superseded_by"], "My newer take")
        out = self.vault("search", query=q, shelf="all", k=8)
        vault_line = next(l for l in out.splitlines() if "[vault] %s " % vt in l)
        self.assertNotIn("replaced by", vault_line)
        self.assertTrue(out.splitlines()[1].startswith("1. [vault] %s " % vt), out)
        out = self.vault("search", query=q, shelf="yours", k=5)
        self.assertIn("(replaced by My newer take)", next(l for l in out.splitlines() if "[yours] %s " % vt in l))
        # the Librarian's lookup and the vault on its own agree
        out = self.vault("search", query=q, shelf="vault", k=3)
        self.assertNotIn("replaced by", out)
        hits = rbxvault._keyword_passages(q, ["vault", "yours"], 3, None, None)
        self.assertNotIn("replaced", next(h for h in hits if h["shelf"] == "vault" and h["title"] == vt)["text"])

    def test_a_shipped_note_mark_in_the_sidecar_leaves_a_user_note_of_that_title(self):
        self.mine("Spawn rule", "Players spawn at the lantern shrine.")
        write(os.path.join(self.notes, rbxnotes.SIDECAR), json.dumps({"Spawn rule": {"by": "Our spawn", "date": "x"}}))
        out = self.vault("search", query="where do players spawn shrine", shelf="yours", k=5)
        self.assertIn("[yours] Spawn rule", out)
        self.assertNotIn("replaced by", out)

    def test_a_vault_search_reads_the_vault_index_once(self):
        # review finding 11: the superseded check read and parsed the whole vault cache a second time
        rbxvault.load_index()
        idx = rbxvault.load_index()
        title = next(d["title"] for d in idx["docs"] if "DataStore" in d["title"])
        real = rbxvault.load_index
        for marked in (False, True):
            if marked:
                write(os.path.join(self.notes, rbxnotes.SIDECAR), json.dumps({title: {"by": "Mine", "date": "x"}}))
            with mock.patch.object(rbxvault, "load_index", side_effect=real) as loads:
                out = self.vault("search", query="datastore session locking", shelf="vault")
                self.assertEqual(loads.call_count, 1, "marked" if marked else "plain")
                self.vault("search", query="datastore session locking", shelf="all")
                self.assertEqual(loads.call_count, 2)
            self.assertEqual("replaced by" in self.vault("search", query=title, shelf="vault"), marked, out)
        # the maps over the whole vault are worked out once per loaded index, not per search
        with mock.patch.object(rbxvault, "_sup_entries", wraps=rbxvault._sup_entries) as made:
            rbxvault.superseded_for(idx, [], "")
            rbxvault.superseded_for(rbxvault.load_index(), [], "")
        self.assertEqual([c.args[1:] for c in made.call_args_list], [(), ()], "the vault's are kept")
        self.assertIs(rbxvault._vault_by_title(idx), rbxvault._vault_by_title(idx))

    def test_a_shipped_vault_note_marked_in_the_sidecar(self):
        idx = rbxvault.load_index()
        d = next(d for d in idx["docs"] if len(d["title"].split()) >= 3 and d["tldr"])
        q = d["title"]
        before = rbxvault.combined_search(q, ["vault"], 5)
        self.assertIn("[vault] %s " % d["title"], before)
        os.makedirs(self.notes, exist_ok=True)
        with open(os.path.join(self.notes, rbxnotes.SIDECAR), "w", encoding="utf-8") as f:
            json.dump({d["title"]: {"by": "My own note", "date": "2026-11-20"}}, f)
        out = self.vault("search", query=q, shelf="vault", k=8)
        line = next(l for l in out.splitlines() if "[vault] %s " % d["title"] in l)
        self.assertIn("(replaced by My own note)", line)
        first = [l for l in out.splitlines() if l[:3] in ("1. ", "2. ")]
        self.assertFalse(any(d["title"] + " " in l for l in first[:1]), out)

    def test_links_still_followed(self):
        self.mine("Shop layout", "The shop layout puts the lantern stall first. See [[Lantern stall prices]].")
        self.mine("Lantern stall prices", "Lantern stall prices: shop lantern layout costs ten coins.",
                  superseded_by="Stall prices 2")
        self.mine("Stall prices 2", "Prices moved to the market board.")
        for n in range(4):
            self.mine("Filler %d" % n, "The shop layout lantern stall filler %d text." % n)
        out = self.vault("search", query="shop layout lantern stall", shelf="yours", k=6)
        self.assertIn("Lantern stall prices", out)
        self.assertIn("(replaced by Stall prices 2)", out)

    def test_librarian_passages_say_it_was_replaced(self):
        self.mine("Coin cap", "The coin cap per player is 500 coins in the shop economy.")
        self.mine("Coin cap raised", "The coin cap per player is now 900 coins in the economy.",
                  supersedes="Coin cap")
        hits = rbxvault._keyword_passages("coin cap per player", ["yours"], 3, None, None)
        titles = [h["title"] for h in hits]
        self.assertLess(titles.index("Coin cap raised"), titles.index("Coin cap"))
        old = next(h for h in hits if h["title"] == "Coin cap")
        self.assertIn("replaced by 'Coin cap raised'", old["text"])
        self.assertNotIn("replaced", hits[0]["text"])
        [part] = rbxvault.lookup(["coin cap per player"], k=3)  # over every shelf: the replacement comes first
        titles = [h["title"] for h in part["hits"]]
        self.assertIn("Coin cap raised", titles)
        self.assertNotIn("Coin cap", titles[:titles.index("Coin cap raised")])


# ------------------------------------------------------------------ search: the knowledge index's hit lines
class RagLineTests(UpkeepCase):
    def test_rag_hit_lines_show_the_replacement(self):
        from tests.test_rag_vault import StubRag, hit_for
        old = self.mine("Coin cap", "## Cap\n\nThe coin cap per player is 500 coins.")
        new = self.mine("Coin cap raised", "## Cap\n\nThe coin cap per player is 900 coins.", supersedes="Coin cap")
        marked = self.mine("Spawn", "## Where\n\nPlayers spawn at the shrine.", superseded_by="Spawn two")
        hits = [dict(hit_for("yours", "Coin cap raised", new, "Cap")),
                dict(hit_for("yours", "Coin cap", old, "Cap"), replaced_by="Coin cap raised"),
                dict(hit_for("yours", "Spawn", marked, "Where"))]
        stub = StubRag(self, hits)
        path = os.path.join(self.tmp, "endpoint.json")
        write(path, json.dumps({"url": stub.url, "token": "t"}))
        with mock.patch.dict(os.environ, {"FRIDAE_RAG_FILE": path}):
            out = self.vault("search", query="coin cap", shelf="yours", k=5)
        lines = out.splitlines()
        self.assertIn("(replaced by Coin cap raised)", next(l for l in lines if l.startswith("2. ")))
        self.assertNotIn("replaced by", next(l for l in lines if l.startswith("1. ")))
        self.assertIn("(replaced by Spawn two)", next(l for l in lines if l.startswith("3. ")))


# ------------------------------------------------------------------ checks when a note is filed
class FilingCheckTests(UpkeepCase):
    def test_same_title_is_not_filed_twice(self):
        self.vault("note", text="Menus fade in over a quarter second.", title="Menu motion", scope="all")
        out = self.vault("note", text="Something else about menus entirely.", title="Menu motion", scope="all")
        self.assertTrue(out.startswith("Not saved"), out)
        self.assertIn("'Menu motion'", out)
        self.assertIn("Menus fade in over a quarter second", out, "the existing note is shown")
        self.assertIn("supersedes", out)
        self.assertIn("force: true", out)
        # another text under the same title: overwriting it would lose the existing note, so it is not offered
        self.assertNotIn("replace: true", out)
        self.assertEqual(os.listdir(self.notes), ["Menu motion.md"])
        self.vault("note", text="Something else about menus entirely.", title="Menu motion", scope="all", force=True)
        self.assertEqual(sorted(os.listdir(self.notes)), ["Menu motion 2.md", "Menu motion.md"])

    def test_same_title_near_copy_offers_replace_last_and_says_it_overwrites(self):
        self.vault("note", text="Menus fade in over a quarter second on every screen.", title="Menu motion",
                   scope="all")
        out = self.vault("note", text="Menus fade in over a quarter second on every screen!", title="Menu motion",
                         scope="all")
        self.assertTrue(out.startswith("Not saved"), out)
        self.assertIn("replace: true", out)
        self.assertIn("overwrites", out)
        self.assertLess(out.index("supersedes"), out.index("replace: true"))
        self.assertLess(out.index("force: true"), out.index("replace: true"))

    def test_untitled_notes_are_compared_by_their_text_only(self):
        # both get the title "The user wants the shop to use coins…" (review finding 9)
        first = self.vault("note", text="The user wants the shop to use coins only, never Robux, because kids cannot "
                           "buy Robux.", scope="all")
        self.assertTrue(first.startswith("Note saved"), first)
        out = self.vault("note", text="The user wants the shop to use coins for pets but gems for skins and trails.",
                         scope="all")
        self.assertTrue(out.startswith("Note saved"), out)
        self.assertEqual(len(rbxnotes.note_files(self.notes)), 2)
        # an untitled near-copy of a note's text is still caught, and the reply leads with supersedes and force
        out = self.vault("note", text="The user wants the shop to use coins only, never Robux, because kids cannot "
                         "buy Robux!", scope="all")
        self.assertTrue(out.startswith("Not saved"), out)
        self.assertNotIn("the same title", out)
        lines = out.splitlines()
        advice = lines[-1]
        self.assertTrue(advice.index("supersedes") < advice.index("force: true"), advice)

    def test_near_duplicate_body_under_another_title(self):
        self.vault("note", text="The user wants thin outlines on every button, never thick black strokes.",
                   title="Button outlines", scope="all")
        out = self.vault("note", text="User wants thin outlines on every button and never thick black strokes!",
                         title="Outline style", scope="all")
        self.assertTrue(out.startswith("Not saved"), out)
        self.assertIn("'Button outlines'", out)
        self.assertEqual(os.listdir(self.notes), ["Button outlines.md"])

    def test_replace_updates_without_a_duplicate_warning(self):
        self.vault("note", text="Menus fade in.", title="Menu motion", scope="all")
        out = self.vault("note", text="Menus fade in, and fade out faster.", title="Menu motion", scope="all",
                         replace=True)
        self.assertTrue(out.startswith("Note saved"), out)

    def test_supersedes_skips_the_duplicate_check_against_its_target(self):
        self.vault("note", text="The user wants thin outlines on every button.", title="Outlines", scope="all")
        out = self.vault("note", text="The user wants thin outlines on every button, in gold.", title="Outlines gold",
                         scope="all", supersedes="Outlines")
        self.assertTrue(out.startswith("Note saved"), out)
        self.assertNotIn("contradiction", out)

    def test_game_scope_checks_general_notes_and_not_the_other_way(self):
        self.vault("note", text="The user wants thin outlines on every button.", title="Outlines", scope="all")
        with self.in_project():
            out = self.vault("note", text="The user wants thin outlines on every button.", title="Game outlines",
                             scope="game")
            self.assertTrue(out.startswith("Not saved"), out)
            self.vault("note", text="Shrine doors open at dusk for twelve seconds.", title="Doors", scope="game")
            out = self.vault("note", text="Shrine doors open at dusk for twelve seconds.", title="Doors general",
                             scope="all")
        self.assertTrue(out.startswith("Note saved"), out)

    def test_contradictions_are_filed_and_shown_side_by_side(self):
        cases = [
            ("Max 4 players per server in the lobby.", "Max 8 players per server in the lobby.", "numbers"),
            ("Players can trade pets with friends in the hub.", "Players cannot trade pets with friends in the hub.",
             "yes/no"),
            ("Save player data with UpdateAsync on the server.", "Save player data with SetAsync on the server.",
             "API"),
        ]
        for n, (old, new, why) in enumerate(cases):
            self.vault("note", text=old + " Other words %d about the topic." % n, title="Old %d" % n, scope="all")
            out = self.vault("note", text=new + " Different context sentence %d here." % n, title="New %d" % n,
                             scope="all")
            self.assertTrue(out.startswith("Note saved"), out)
            self.assertIn("Possible contradiction with 'Old %d'" % n, out)
            self.assertIn(old, out)
            self.assertIn(new, out)
            self.assertIn(why, out)
            self.assertIn("supersedes", out)
            self.assertIn("both hold", out)

    def test_no_false_alarms_on_related_notes(self):
        olds = ["Players start with 100 coins and a wooden sword.",
                "The shop sells 3 hats and 2 capes.",
                "Use 2 spaces for indentation in Luau files.",
                "Lanterns cost 40 coins at the market stall.",
                "The boss fight has 3 phases and lasts about 4 minutes.",
                "Never use gradients on buttons."]
        news = ["Players earn 5 coins per kill in the arena.",
                "Coins drop 3 at a time from chests.",
                "Run the 4 linters before each commit.",
                "Lantern oil refills every 40 seconds at night.",
                "The boss arena is lit by 12 torches.",
                "Buttons use flat colours and soft shadows."]
        for n, t in enumerate(olds):
            self.vault("note", text=t, title="Old fact %d" % n, scope="all")
        for n, t in enumerate(news):
            out = self.vault("note", text=t, title="New fact %d" % n, scope="all")
            self.assertEqual(out.splitlines()[0][:10], "Note saved", out)
            self.assertNotIn("contradiction", out, t)

    def test_superseded_notes_do_not_raise_alarms(self):
        self.vault("note", text="Max 4 players per server in the lobby.", title="Lobby size", scope="all")
        self.vault("note", text="Lobby servers changed this month.", title="Lobby size 2", scope="all",
                   supersedes="Lobby size")
        out = self.vault("note", text="Max 8 players per server in the lobby.", title="Lobby size 3", scope="all")
        self.assertNotIn("Possible contradiction with 'Lobby size'", out)


# ------------------------------------------------------------------ the sweep
class SweepTests(UpkeepCase):
    def fill(self):
        self.mine("Button outlines", "The user wants thin outlines on every button, never thick black strokes.")
        self.mine("Outline style", "User wants thin outlines on every button and never thick black strokes!")
        self.mine("Lobby size", "Max 4 players per server in the lobby. Lobby music is calm.")
        self.mine("Lobby update", "Max 8 players per server in the lobby. The update shipped in May.")
        self.mine("Old map", "The map has a river and a mill.", superseded_by="New map")
        self.mine("New map", "The map has a lake and a windmill.", supersedes="Old map")
        self.mine("Map tour", "Start the tour at [[Old map]] then visit [[Shop layout]] and [[Nowhere note]].")
        self.mine("Shop layout", "Shop layout notes.")
        self.mine("Torch fuel", "Torches in the lantern village burn kerosene oil for ninety seconds each night.",
                  when="2025-03-01 10:00")
        self.mine("Torch fuel update", "Torches in the lantern village burn lamp oil for ninety seconds each night "
                  "and kerosene is gone.", when="2026-11-01 10:00")
        self.mine("Ancient unrelated", "Bananas ripen in the greenhouse on tuesdays.", when="2024-01-01 10:00")
        with self.in_project():
            self.mine("Game outlines", "The user wants thin outlines on every button, never thick black strokes.",
                      folder=self.game_notes)

    def snapshot(self):
        out = {}
        for root in (self.notes, self.game_notes):
            for f in sorted(os.listdir(root)):
                p = os.path.join(root, f)
                if os.path.isfile(p):
                    out[p] = (open(p, "rb").read(), os.stat(p).st_mtime_ns)
        return out

    def test_report(self):
        self.fill()
        before = self.snapshot()
        now = time.mktime(time.strptime("2026-11-24 12:00", "%Y-%m-%d %H:%M"))
        res = rbxupkeep.sweep(projects=[self.project], now=now)
        self.assertEqual(res["path"], os.path.join(self.notes, "upkeep", "2026-11-24.md"))
        self.assertEqual(self.snapshot(), before, "the sweep never changes a note")
        text = rbxvault.read(res["path"])
        sections = {}
        cur = None
        for line in text.splitlines():
            if line.startswith("## "):
                cur = line[3:]
                sections[cur] = []
            elif cur:
                sections[cur].append(line)
        body = {k.split(" (")[0]: "\n".join(v) for k, v in sections.items()}
        self.assertIn("'Button outlines'", body["Near-duplicates"])
        self.assertIn("'Outline style'", body["Near-duplicates"])
        self.assertIn("'Game outlines'", body["Near-duplicates"])
        self.assertIn("'Lobby size'", body["Possible contradictions"])
        self.assertIn("Max 8 players", body["Possible contradictions"])
        self.assertNotIn("Old map", body["Possible contradictions"])
        self.assertIn("'Map tour' links to [[Old map]], replaced by 'New map'", body["Superseded notes still linked"])
        self.assertIn("'Torch fuel'", body["Old notes with newer ones on the same topic"])
        self.assertIn("'Torch fuel update'", body["Old notes with newer ones on the same topic"])
        self.assertNotIn("Ancient unrelated", body["Old notes with newer ones on the same topic"])
        self.assertIn("[[Nowhere note]]", body["Broken links"])
        self.assertNotIn("[[Shop layout]]", body["Broken links"])
        self.assertNotIn("[[Old map]]", body["Broken links"])
        self.assertEqual(res["counts"]["broken_links"], 1)

    def test_tool_action_and_empty_folder(self):
        out = self.vault("upkeep")
        self.assertIn(os.path.join(self.notes, "upkeep"), out)
        self.assertIn("0 notes", out)
        self.fill()
        with self.in_project():
            out = self.vault("upkeep")
        self.assertIn("near-duplicate", out)

    def test_ten_thousand_notes_in_time(self):
        rnd = random.Random(7)
        vocab = ["w%s" % hashlib.md5(str(i).encode()).hexdigest()[:7] for i in range(6000)]
        weights = [1.0 / (i + 1) for i in range(len(vocab))]
        common = ["the", "player", "shop", "coins", "server", "lantern", "button", "use", "never", "max"]
        os.makedirs(self.notes)
        n = 10000
        for i in range(n):
            words = rnd.choices(vocab, weights, k=rnd.randint(40, 160))
            sents = []
            while words:
                chunk, words = words[:rnd.randint(6, 14)], words[14:]
                if rnd.random() < 0.3:
                    chunk.insert(rnd.randrange(len(chunk) + 1), str(rnd.randint(1, 500)))
                if rnd.random() < 0.2:
                    chunk.insert(0, rnd.choice(common))
                sents.append(" ".join(chunk) + ".")
            links = " ".join("[[Note %d]]" % rnd.randrange(n + 50) for _ in range(rnd.randint(0, 3)))
            day = "2026-%02d-%02d" % (rnd.randint(1, 11), rnd.randint(1, 28)) if rnd.random() < 0.7 else "2025-01-05"
            with open(os.path.join(self.notes, "Note %d.md" % i), "w", encoding="utf-8") as f:
                f.write("---\ntitle: Note %d\nwhen: %s 10:00\n---\n\n# Note %d\n\n%s %s\n" % (
                    i, day, i, " ".join(sents), links))
        for i in range(0, 40, 2):  # some real duplicates and contradictions among them
            shutil.copy(os.path.join(self.notes, "Note %d.md" % i), os.path.join(self.notes, "Copy %d.md" % i))
        start = time.time()
        res = rbxupkeep.sweep(now=time.mktime(time.strptime("2026-11-24", "%Y-%m-%d")))
        took = time.time() - start
        print("\nupkeep sweep over %d notes: %.1f s (%s)" % (n + 20, took, res["counts"]))
        self.assertLess(took, 30.0)
        self.assertGreaterEqual(res["counts"]["near_duplicates"], 20)
        self.assertGreater(res["counts"]["broken_links"], 0)
