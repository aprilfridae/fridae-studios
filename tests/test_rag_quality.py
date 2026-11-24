"""Index quality: each passage's context line, the notes' labels and the search filters, and following [[links]]
(rbxrag.chunks, rbxrag.engine, rbxrag.stores, rbxrag.serve and rbxvault's keyword search)."""
import json
import os
import re
import shutil
import tempfile
import threading
import time
import unittest
from unittest import mock

import rbxpaths
import rbxvault
from rbxpaths import KitError
from rbxrag import chunks, engine, serve
from rbxrag.engine import PLANS, Engine
from rbxrag.stores import MemStore, QdrantStore
from tests.test_rag_engine import FakeEmbedders
from tests.test_rag_serve import TOKEN, call

try:
    from qdrant_client import QdrantClient
except ImportError:  # pragma: no cover
    QdrantClient = None


def setUpModule():
    global _TMP, _PATCHES
    _TMP = tempfile.mkdtemp(prefix="rag quality é ")
    _PATCHES = [mock.patch.object(rbxpaths, "WORK", os.path.join(_TMP, "work")),
                mock.patch.object(rbxpaths, "HOME_DIR", os.path.join(_TMP, "home"))]
    for p in _PATCHES:
        p.start()


def tearDownModule():
    for p in _PATCHES:
        p.stop()
    shutil.rmtree(_TMP, True)


def write(path, text, mtime=None):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    if mtime is not None:
        os.utime(path, (mtime, mtime))
    return path


def ts(day):
    return time.mktime(time.strptime(day, "%Y-%m-%d")) + 12 * 3600


NOTE = """---
tags: [datastore]
---

# Saving Data

Some opening words about saving player data safely. More words follow here.

## Sessions

### Session locks

Lock the profile while a server holds it.

## Retries

Retry UpdateAsync with backoff.
"""


class Tmp(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rag q é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.count = chunks.make_token_counter(None)


# ====================================================================== 1. the context line
class ContextLineTests(Tmp):
    def test_every_chunk_has_title_section_path_shelf_labels_and_gist(self):
        path = write(os.path.join(self.tmp, "Saving Data.md"), NOTE, ts("2026-11-03"))
        cs = chunks.chunks_for_file("yours", "Saving Data", path, self.count)
        lock = next(c for c in cs if c["heading"] == "Session locks")
        first, second, body = lock["text"].split("\n", 2)
        self.assertEqual(first, "Saving Data › Session locks")
        self.assertTrue(second.startswith("Context: "), second)
        self.assertIn("Saving Data > Sessions > Session locks", second)
        self.assertIn("| yours |", second)
        self.assertIn("lesson, user |", second)  # no date in its front matter: no date (not the file's day)
        self.assertIn("Some opening words about saving player data safely.", second)  # the first sentence only
        self.assertNotIn("More words", second)
        self.assertEqual(body.strip(), "Lock the profile while a server holds it.")
        for c in cs:
            self.assertEqual(c["text"].split("\n")[1][:9], "Context: ")
            self.assertEqual({k: c[k] for k in chunks.LABEL_KEYS},
                             {"game": "all", "kind": "lesson", "source": "user", "date": ""})

    def test_description_is_the_gist_and_a_game_note_says_its_game(self):
        project = os.path.join(self.tmp, "Lantern Run")
        path = write(os.path.join(project, ".fridae", "notes", "Glow.md"),
                     "---\ndescription: Lantern glow is warm amber, never blue.\nby: Builder\nkind: lesson\n---\n\n"
                     "# Glow\n\nOpening text.\n\n## Colour\n\nAmber at 2700 K.\n")
        cs = chunks.chunks_for_file("game", "Glow", path, self.count, project=project)
        colour = next(c for c in cs if c["heading"] == "Colour")
        line = colour["text"].split("\n")[1]
        self.assertIn("game Lantern Run", line)
        self.assertIn("Lantern glow is warm amber, never blue.", line)
        self.assertEqual((colour["game"], colour["kind"], colour["source"]), ("Lantern Run", "lesson", "game"))
        opening = next(c for c in cs if c["i"] == 0)
        self.assertEqual(opening["text"].count("Lantern glow is warm amber"), 1)  # not twice in the opening

    def test_gist_is_trimmed_to_thirty_words_and_skips_what_is_not_prose(self):
        long = " ".join("word%d" % i for i in range(60)) + "."
        body = ("<!-- tldr:start -->\n> [!tldr] TL;DR\n> - skip me\n<!-- tldr:end -->\n\n```lua\nlocal x = 1\n```\n\n"
                "| a | b |\n|---|---|\n\n" + long + "\n")
        g = rbxvault.note_gist({}, body)
        self.assertEqual(len(g.split()), 30)
        self.assertTrue(g.startswith("word0 word1") and g.endswith("…"), g)
        self.assertEqual(rbxvault.note_gist({"summary": "From the summary."}, body), "From the summary.")
        self.assertEqual(rbxvault.note_gist({}, "## Head\n\n- First item of a list [[Linked Note|alias]].\n- Two.\n"),
                         "First item of a list alias.")

    def test_header_fits_the_window_and_passages_hide_it(self):
        text = "# Big\n\n## Part\n\n" + "\n\n".join("Paragraph %d with a fair few words in it to fill space." % i
                                                   for i in range(300))
        path = write(os.path.join(self.tmp, "Big.md"), text)
        cs = chunks.chunks_for_file("vault", "Big", path, self.count)
        self.assertGreater(len(cs), 3)
        for c in cs:
            self.assertLessEqual(self.count(c["text"]), chunks.LIMIT)
        shown = rbxvault.passage(cs[0]["text"], 2000)
        self.assertFalse(any(l.startswith("Context: ") or "›" in l for l in shown), shown[:2])
        self.assertTrue(shown[0].startswith("Paragraph 0"))
        self.assertEqual(rbxvault.passage("T › H\nBody line.", 100), ["Body line."])  # an old chunk still reads

    def test_a_freak_gist_or_heading_never_eats_the_window(self):
        fm = "---\ndescription: %s\n---\n" % " ".join(["gist"] * 400)
        path = write(os.path.join(self.tmp, "Odd.md"), fm + "# Odd\n\n## %s\n\nText.\n" % ("H" * 900))
        for c in chunks.chunks_for_file("guides", "Odd", path, self.count):
            self.assertLessEqual(self.count(c["text"].split("\n")[0] + "\n" + c["text"].split("\n")[1] + "\n"),
                                 chunks.LIMIT // 4)

    def test_format_version_bumped_so_old_indexes_rebuild_once(self):
        self.assertEqual(engine.FORMAT, 3)
        a = write(os.path.join(self.tmp, "kit", "Alpha.md"), "# Alpha\n\n## Coins\n\nShiny coins. [[Bravo]]\n")
        b = write(os.path.join(self.tmp, "kit", "Bravo.md"), "# Bravo\n\n## Lanterns\n\nA lantern.\n")
        files = [("vault", "Alpha", a), ("vault", "Bravo", b)]
        manifest = os.path.join(self.tmp, "rag", "manifest.json")
        folder = os.path.join(self.tmp, "rag", "light")

        def eng():
            return Engine(MemStore(folder), FakeEmbedders(), manifest, corpus=lambda p: list(files),
                          project_files=lambda p: set(), plan=PLANS["light"])
        first = eng().sync()
        self.assertEqual(first["embedded"], first["chunks"])
        with open(manifest, encoding="utf-8") as f:
            man = json.load(f)
        self.assertEqual(man["format"], 2)
        self.assertEqual(man["files"][a]["links"], ["bravo"])
        self.assertNotIn("links", man["files"][b])
        self.assertEqual(eng().sync()["embedded"], 0)  # incremental as before
        man["format"] = 1  # an index from before the context lines
        with open(manifest, "w", encoding="utf-8") as f:
            json.dump(man, f)
        again = eng().sync()
        self.assertEqual(again["embedded"], again["chunks"])  # rebuilt, once
        self.assertEqual(eng().sync()["embedded"], 0)


# ====================================================================== 2. labels
class LabelTests(Tmp):
    def labels(self, shelf, rel, text, title=None, project="", base=None):
        path = write(os.path.join(base or self.tmp, rel), text)
        fm, _ = rbxvault.split_front(text)
        return rbxvault.note_labels(shelf, title or os.path.basename(rel)[:-3], path, fm, text, project)

    def test_inference_cases(self):
        kb = os.path.join(self.tmp, "vault", "Roblox")
        with mock.patch.object(rbxvault, "KB", kb):
            got = self.labels("vault", "04 Data/Saving.md", "---\ntags: [data, type/guide]\n---\n# S\n", base=kb)
            self.assertEqual((got["kind"], got["source"], got["game"]), ("guide", "lihfolk", "all"))
            got = self.labels("vault", "57 API and Concept Atlas/Actor.md", "# Actor\n\nAn actor.\n", base=kb)
            self.assertEqual((got["kind"], got["source"]), ("reference", "lihfolk"))
            got = self.labels("vault", "Taste/Softer.md", "# Softer\n\nRounder corners.\n")
            self.assertEqual((got["kind"], got["source"]), ("taste", "user"))
        cases = [
            # (shelf, file, text, project) -> (kind, source)
            ("yours", "House rules.md", "- Always test on a phone.\n", "", ("rule", "user")),
            ("yours", "Tip.md", "---\nkind: lesson\nby: Builder\n---\n# Tip\n", "", ("lesson", "agent")),
            ("yours", "Mine.md", "---\nby: user\nkind: note\n---\n# Mine\n", "", ("lesson", "user")),
            ("yours", "Pref.md", "---\ntype: taste\n---\n# Pref\n", "", ("taste", "user")),
            ("yours", "studio-lessons.md", "- 2026-10-01: x\n", "", ("lesson", "agent")),
            ("vault", "Shared.md", "---\ncontributor: someone\n---\n# Shared\n", "", ("reference", "community")),
            ("vault", "Credited.md", "# Old\n\nText.\n\nLicense: this note is CC BY 4.0 (LihFolk).\n", "",
             ("reference", "lihfolk")),
            ("guides", "roblox-ui.md", "# UI\n", "", ("guide", "fridae")),
            ("team", "coder.md", "# Coder\n", "", ("guide", "fridae")),
            ("game", "GAME.md", "# Lantern Run\n", "/games/Lantern Run", ("reference", "game")),
            ("game", "Lava.md", "---\nkind: lesson\nby: Lead\n---\n# Lava\n", "/games/Lantern Run", ("lesson", "game")),
            ("vault", "Sourced.md", "---\nsource: agent\nkind: rule\n---\n# S\n", "", ("rule", "agent")),
        ]
        for shelf, rel, text, project, want in cases:
            got = self.labels(shelf, rel, text, project=project)
            self.assertEqual((got["kind"], got["source"]), want, (shelf, rel))
            self.assertEqual(got["game"], "Lantern Run" if project else "all", rel)

    def test_date_only_from_front_matter(self):
        got = self.labels("yours", "A.md", "---\nwhen: 2026-11-02 09:00\n---\n# A\n")
        self.assertEqual(got["date"], "2026-11-02")
        got = self.labels("vault", "B.md", "---\nverified: 2025-07\n---\n# B\n")
        self.assertEqual(got["date"], "2025-07-01")
        # no date key: no date, never the day the file was installed or checked out (review finding 10)
        path = write(os.path.join(self.tmp, "C.md"), "# C\n", ts("2026-03-04"))
        self.assertEqual(rbxvault.note_labels("vault", "C", path, {}, "# C\n")["date"], "")
        self.assertNotRegex(rbxvault.label_text(rbxvault.note_labels("vault", "C", path, {}, "# C\n")), r"\d")
        self.assertFalse(rbxvault.labels_match(rbxvault.note_labels("vault", "C", path, {}, "# C\n"),
                                               {"since": 20000101}), "since leaves undated notes out")

    def test_learned_in_fridae_section_is_an_agent_lesson(self):
        base = {"game": "all", "kind": "guide", "source": "lihfolk", "date": "2026-10-01"}
        self.assertEqual(rbxvault.section_labels(base, "Learned in Fridae"),
                         dict(base, kind="lesson", source="agent"))
        self.assertEqual(rbxvault.section_labels(base, "Retry rule", ["Learned in Fridae"])["kind"], "lesson")
        self.assertIs(rbxvault.section_labels(base, "Checklist", ["Intro"]), base)
        path = write(os.path.join(self.tmp, "Mixed.md"),
                     "# Mixed\n\n## Checklist\n\nDo it.\n\n## Learned in Fridae\n\nSave before teleport.\n")
        cs = {c["heading"]: c for c in chunks.chunks_for_file("vault", "Mixed", path, self.count)}
        self.assertEqual(cs["Checklist"]["kind"], "reference")
        self.assertEqual((cs["Learned in Fridae"]["kind"], cs["Learned in Fridae"]["source"]), ("lesson", "agent"))

    def test_compact_label_text(self):
        self.assertEqual(rbxvault.label_text({"game": "Lantern Run", "kind": "lesson", "source": "game",
                                              "date": "2026-11-24"}), "[lesson · this game · 2026-11]")
        self.assertEqual(rbxvault.label_text({"game": "all", "kind": "guide", "source": "lihfolk",
                                              "date": "2026-10-02"}), "[guide · lihfolk · 2026-10]")
        self.assertEqual(rbxvault.label_text({}), "")

    def test_wikilinks(self):
        text = "See [[Session Lock]], [[DataStore Fundamentals|the basics]], [[Session Lock#Why]] and [[a/B.md]]."
        self.assertEqual(rbxvault.wikilinks(text), ["session lock", "datastore fundamentals", "b"])
        self.assertEqual(len(rbxvault.wikilinks(" ".join("[[N%d]]" % i for i in range(100)))), rbxvault.LINKS_MAX)


# ====================================================================== 2. filters
class FilterArgTests(unittest.TestCase):
    def test_normalize(self):
        n = rbxvault.normalize_filters
        self.assertEqual(n({}), {})
        self.assertEqual(n({"game": "all", "kind": "", "source": None}), {})
        self.assertEqual(n({"game": "this"}, "/games/Lantern Run"), {"game": "this"})
        self.assertEqual(n({"game": "Lantern Run"}, "/games/Lantern Run"), {"game": "this"})
        self.assertEqual(n({"game": "general"}), {"game": "general"})
        self.assertEqual(n({"kind": "lessons, rule"}), {"kind": ["lesson", "rule"]})
        self.assertEqual(n({"source": ["agent", "user"]}), {"source": ["agent", "user"]})
        self.assertEqual(n({"since": "2026-11"}), {"since": 20261101})
        self.assertEqual(n({"since": "2026"}), {"since": 20260101})
        self.assertEqual(n({"since": 20261124}), {"since": 20261124})  # already normalised (the service gets this)
        for bad in ({"game": "this"}, {"game": "zzz"}, {"kind": "poem"}, {"source": "web"}, {"since": "last week"}):
            with self.assertRaises(KitError):
                n(bad)

    def test_labels_match(self):
        lab = {"game": "Lantern Run", "kind": "lesson", "source": "game", "date": "2026-11-20"}
        m = rbxvault.labels_match
        self.assertTrue(m(lab, {}))
        self.assertTrue(m(lab, {"game": "this", "kind": ["lesson"], "since": 20261101}))
        self.assertFalse(m(lab, {"game": "general"}))
        self.assertFalse(m(dict(lab, game="all"), {"game": "this"}))
        self.assertFalse(m(lab, {"kind": ["rule"]}))
        self.assertFalse(m(lab, {"source": ["user"]}))
        self.assertFalse(m(lab, {"since": 20261121}))


class EngineFilterCases:
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rag filters é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        d = os.path.join(self.tmp, "kit")
        self.project = os.path.join(self.tmp, "Lantern Run")
        self.rule = write(os.path.join(d, "House rules.md"), "- Lantern glow is always warm.\n", ts("2026-11-10"))
        self.lesson = write(os.path.join(d, "Glow tip.md"), "---\nkind: lesson\nby: Builder\nwhen: 2026-11-20\n---\n"
                            "# Glow tip\n\nThe lantern glow fades in 0.2 s.\n")
        self.old = write(os.path.join(d, "Old lantern.md"), "---\nverified: 2025-01-05\n---\n# Old lantern\n\n"
                         "## Glow\n\nAn old lantern glow reference.\n")
        self.game = write(os.path.join(self.project, ".fridae", "notes", "Arena.md"),
                          "---\nwhen: 2026-11-22\n---\n# Arena\n\nThe arena lantern glow is green.\n")
        self.files = [("yours", "House rules", self.rule), ("yours", "Glow tip", self.lesson),
                      ("vault", "Old lantern", self.old)]
        self.client = QdrantClient(":memory:") if QdrantClient else None
        self.eng = Engine(self.new_store(), FakeEmbedders(), os.path.join(self.tmp, "rag", "manifest.json"),
                          corpus=self.corpus, project_files=lambda p: {self.game} if p else set(),
                          plan=PLANS["light"])
        self.eng.sync()
        self.eng.sync(self.project)

    def corpus(self, project):
        return list(self.files) + ([("game", "Arena", self.game)] if project else [])

    def titles(self, filters, project=None, k=8):
        shelves = list(rbxvault.SHELVES)
        return sorted({h["title"] for h in self.eng.query("lantern glow", shelves, k, project or self.project,
                                                          filters)})

    def test_no_filter_is_everything_with_this_game(self):
        self.assertEqual(self.titles(None), ["Arena", "Glow tip", "House rules", "Old lantern"])
        hit = next(h for h in self.eng.query("lantern glow", list(rbxvault.SHELVES), 8, self.project)
                   if h["title"] == "Arena")
        self.assertEqual((hit["game"], hit["kind"], hit["source"]), ("Lantern Run", "lesson", "game"))

    def test_kind_source_since_and_game(self):
        self.assertEqual(self.titles({"kind": ["rule"]}), ["House rules"])
        self.assertEqual(self.titles({"kind": ["lesson"], "source": ["agent"]}), ["Glow tip"])
        self.assertEqual(self.titles({"source": ["user"]}), ["House rules", "Old lantern"])
        self.assertEqual(self.titles({"since": 20261101}), ["Arena", "Glow tip"])  # House rules has no date
        self.assertEqual(self.titles({"since": 20261115}), ["Arena", "Glow tip"])
        self.assertEqual(self.titles({"game": "this"}), ["Arena"])
        self.assertEqual(self.titles({"game": "general"}), ["Glow tip", "House rules", "Old lantern"])
        self.assertEqual(self.titles({"game": "this", "kind": ["rule"]}), [])


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class QdrantFilterTests(EngineFilterCases, unittest.TestCase):
    def new_store(self):
        return QdrantStore(self.client)


class MemFilterTests(EngineFilterCases, unittest.TestCase):
    def new_store(self):
        return MemStore(os.path.join(self.tmp, "rag", "light"))

    def test_labels_survive_a_reload(self):
        again = Engine(self.new_store(), FakeEmbedders(), os.path.join(self.tmp, "rag", "manifest.json"),
                       corpus=self.corpus, project_files=lambda p: {self.game} if p else set(), plan=PLANS["light"])
        self.assertEqual(again.sync()["embedded"], 0)
        hits = again.query("lantern glow", ["yours"], 5, None, {"kind": ["lesson"]})
        self.assertEqual([h["title"] for h in hits], ["Glow tip"])
        self.assertEqual(hits[0]["date"], "2026-11-20")


class KeywordFilterTests(unittest.TestCase):
    """The keyword search (no knowledge index) takes the same filters and shows each result's labels."""

    def setUp(self):
        self.home = os.path.join(_TMP, "home")
        notes = os.path.join(self.home, "notes")
        self.addCleanup(shutil.rmtree, notes, True)
        write(os.path.join(notes, "House rules.md"), "- Zorblat lanterns always glow amber.\n")
        write(os.path.join(notes, "Zorblat tip.md"), "---\nkind: lesson\nby: Builder\nwhen: 2026-11-20\n---\n"
              "# Zorblat tip\n\nZorblat lanterns glow amber after 0.2 s.\n")
        write(os.path.join(notes, "Zorblat old.md"), "---\nwhen: 2024-02-02\nby: user\n---\n# Zorblat old\n\n"
              "Zorblat lanterns used to glow blue.\n")
        self.project = tempfile.mkdtemp(prefix="Zorblat Caves ")
        self.addCleanup(shutil.rmtree, self.project, True)
        write(os.path.join(self.project, ".fridae", "notes", "Cave glow.md"),
              "# Cave glow\n\nZorblat lanterns glow amber in the caves.\n")

    def search(self, **a):
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": self.project, "FRIDAE_RAG_FILE": ""}):
            return rbxvault.run("search", dict({"query": "zorblat lanterns glow amber", "shelf": "all", "k": 8}, **a))

    def titles(self, text):
        return sorted({json.loads(l.split("read: ", 1)[1])["title"] for l in text.splitlines()
                       if l.strip().startswith("read: ")})

    def test_filters(self):
        text = self.search()
        self.assertRegex(text, r"\[yours\] Zorblat tip .*\[lesson · agent · 2026-11\]")
        self.assertRegex(text, r"\[game\] Cave glow .*\[lesson · this game\]")  # undated: no date shown
        self.assertEqual(self.titles(self.search(kind="rule")), ["House rules"])
        self.assertEqual(self.titles(self.search(source="agent")), ["Zorblat tip"])
        self.assertEqual(self.titles(self.search(game="this")), ["Cave glow"])
        self.assertNotIn("Cave glow", self.search(game="general"))
        self.assertNotIn("Zorblat old", self.search(since="2026-01", shelf="yours"))
        self.assertIn("Zorblat old", self.search(since="2024", shelf="yours"))
        self.assertIn("with those filters", self.search(kind="taste", shelf="yours"))
        with self.assertRaises(KitError):
            self.search(kind="poem")

    def test_vault_shelf_with_a_filter_uses_the_combined_search(self):
        text = self.search(shelf="vault", kind="taste", k=3)
        self.assertTrue(text.startswith("No match") or "[vault]" in text)
        self.assertNotIn("[guide ·", text)


class ServiceFilterTests(unittest.TestCase):
    def test_filters_reach_the_engine_and_bad_ones_are_refused(self):
        seen = []

        class FakeEngine:
            synced_projects = set()

            def query(self, q, shelves, k, project, filters=None):
                seen.append(filters)
                return []

            def sync(self, *a, **kw):
                return {"chunks": 0, "files": 0, "embedded": 0, "removed": 0}

        svc = serve.Service("light", FakeEngine)
        svc.engine, svc.ready, svc._last_sync = FakeEngine(), True, svc.clock()
        srv = serve.make_server(svc, TOKEN, 0)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        base = "http://127.0.0.1:%d" % srv.server_address[1]
        self.assertEqual(call(base, "/query", {"q": "x", "filters": {"kind": ["rule"], "since": "2026-11"}})[0], 200)
        self.assertEqual(call(base, "/query", {"q": "x"})[0], 200)
        self.assertEqual(seen, [{"kind": ["rule"], "since": 20261101}, None])
        self.assertEqual(call(base, "/query", {"q": "x", "filters": {"kind": "poem"}})[0], 400)
        self.assertEqual(call(base, "/query", {"q": "x", "filters": ["rule"]})[0], 400)


# ====================================================================== 3. following links
class LinkCases:
    """A hub note links to four notes that match the query but rank below the direct results."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rag links é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        d = os.path.join(self.tmp, "kit")
        self.files = []
        hub = write(os.path.join(d, "Hub.md"), "# Hub\n\n## Lanterns\n\n" + "haunted lantern forest ghosts. " * 6 +
                    "See [[Bravo]], [[Charlie|C]], [[Delta#Part]] and [[Echo]].\n")
        self.files.append(("vault", "Hub", hub))
        for i in range(6):
            p = write(os.path.join(d, "Strong %d.md" % i), "# Strong %d\n\n## Lanterns\n\nhaunted lantern forest "
                      "ghosts with extra%d other%d words%d.\n" % (i, i, i, i))
            self.files.append(("vault", "Strong %d" % i, p))
        for name in ("Bravo", "Charlie", "Delta", "Echo"):
            p = write(os.path.join(d, "%s.md" % name), "# %s\n\n## Part\n\nA haunted lantern sits among %s.\n" % (
                name, " ".join("unrelated%d filler%d words%d" % (j, j, j) for j in range(30))))
            self.files.append(("yours", name, p))
        self.eng = Engine(self.new_store(), FakeEmbedders(), os.path.join(self.tmp, "rag", "manifest.json"),
                          corpus=lambda p: list(self.files), project_files=lambda p: set(), plan=PLANS["light"])
        self.eng.sync()

    def test_linked_hits_are_capped_marked_and_after_the_top_three(self):
        q, shelves = "haunted lantern forest ghosts", ["vault", "yours"]
        hits = self.eng.query(q, shelves, 5, None)
        graph = self.eng._graph
        self.eng._graph = None
        direct = self.eng.query(q, shelves, 5, None)
        self.eng._graph = graph
        self.assertIn("Hub", [h["title"] for h in direct[:3]])
        linked = [n for n, h in enumerate(hits) if h.get("linked_from")]
        self.assertEqual(len(linked), 2)  # four qualify, two at most
        self.assertTrue(all(n >= 3 for n in linked))
        self.assertEqual(hits[:3], direct[:3])  # never in place of a direct top-3 hit
        self.assertEqual(len(hits), 5)
        for n in linked:
            self.assertEqual(hits[n]["linked_from"], "Hub")
            self.assertIn(hits[n]["title"], ("Bravo", "Charlie", "Delta", "Echo"))
            self.assertNotIn(hits[n]["title"], [h["title"] for h in direct])
        self.assertFalse(any(h.get("linked_from") for h in self.eng.query(q, shelves, 3, None)))  # no room

    def test_linked_hits_obey_shelves_and_filters(self):
        q = "haunted lantern forest ghosts"
        self.assertFalse(any(h.get("linked_from") for h in self.eng.query(q, ["vault"], 5, None)))
        self.assertFalse(any(h.get("linked_from") for h in self.eng.query(q, ["vault", "yours"], 5, None,
                                                                         {"kind": ["reference"]})))

    def test_a_linked_note_that_does_not_match_is_left_out(self):
        q = "zebra"  # no linked note holds it, and none is near it
        self.assertFalse(any(h.get("linked_from") for h in self.eng.query(q, ["vault", "yours"], 5, None)))

    def test_the_graph_is_made_per_sync_not_per_query(self):
        with mock.patch.object(Engine, "_link_graph", wraps=self.eng._link_graph) as made:
            self.eng.query("haunted lantern", ["vault", "yours"], 5, None)
            self.eng.query("haunted lantern", ["vault", "yours"], 5, None)
            self.assertEqual(made.call_count, 0)
        links, targets = self.eng._graph
        hub = next(p for s, t, p in self.files if t == "Hub")
        self.assertEqual(links[hub], ["bravo", "charlie", "delta", "echo"])
        self.assertEqual(targets["bravo"][0][:2], ("yours", "Bravo"))
        # a fresh engine on the same index has its graph after its first (nothing to do) sync
        again = Engine(self.new_store(), FakeEmbedders(), os.path.join(self.tmp, "rag", "manifest.json"),
                       corpus=lambda p: list(self.files), project_files=lambda p: set(), plan=PLANS["light"])
        self.assertEqual(again.sync()["embedded"], 0)
        self.assertEqual(again._graph[0][hub], ["bravo", "charlie", "delta", "echo"])

    def test_a_failing_link_lookup_leaves_the_direct_hits(self):
        with mock.patch.object(type(self.eng.store), "best_parts", side_effect=RuntimeError("boom")):
            hits = self.eng.query("haunted lantern forest ghosts", ["vault", "yours"], 5, None)
        self.assertFalse(any(h.get("linked_from") for h in hits))
        self.assertEqual(len(hits), 5)


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class QdrantLinkTests(LinkCases, unittest.TestCase):
    def new_store(self):
        if not hasattr(self, "client"):
            self.client = QdrantClient(":memory:")
        return QdrantStore(self.client)


class MemLinkTests(LinkCases, unittest.TestCase):
    def new_store(self):
        return MemStore(os.path.join(self.tmp, "rag", "light"))


class PlaceLinkedTests(unittest.TestCase):
    def test_placement(self):
        d = ["d1", "d2", "d3", "d4", "d5"]
        self.assertEqual(rbxvault.place_linked(d, ["l1", "l2", "l3"], 5), ["d1", "d2", "d3", "l1", "l2"])
        self.assertEqual(rbxvault.place_linked(d, ["l1"], 5), ["d1", "d2", "d3", "l1", "d4"])
        self.assertEqual(rbxvault.place_linked(d[:3], ["l1"], 3), ["d1", "d2", "d3"])
        self.assertEqual(rbxvault.place_linked(d[:2], ["l1", "l2"], 5), ["d1", "d2", "l1", "l2"])


class KeywordLinkTests(unittest.TestCase):
    def test_keyword_search_follows_links_from_the_first_results(self):
        notes = os.path.join(_TMP, "home", "notes")
        self.addCleanup(shutil.rmtree, notes, True)
        write(os.path.join(notes, "Quibble frobnicators wobble hub.md"), "# Quibble frobnicators wobble hub\n\n"
              "Quibble frobnicators wobble. See [[Quibble deep]] and [[Unrelated]].\n")
        for i in range(5):
            write(os.path.join(notes, "Quibble %d.md" % i), "# Quibble %d\n\nQuibble frobnicators wobble a lot.\n" % i)
        write(os.path.join(notes, "Quibble deep.md"), "# Deep dive\n\nWhy frobnicators wobble: loose quibble pins. " +
              " ".join("filler%d" % j for j in range(200)) + "\n")
        write(os.path.join(notes, "Unrelated.md"), "# Unrelated\n\nNothing about it here.\n")
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "FRIDAE_RAG_FILE": ""}):
            pool, _ = rbxvault._keyword_pool("quibble frobnicators wobble", ["yours"], 5, links=True)
            direct, _ = rbxvault._keyword_pool("quibble frobnicators wobble", ["yours"], 5)
        titles = [item[2]["title"] for item in pool]
        linked = [item for item in pool if len(item) > 3]
        self.assertEqual(pool[:3], direct[:3])
        self.assertIn("Quibble frobnicators wobble hub", [item[2]["title"] for item in direct[:3]])
        self.assertNotIn("Quibble deep", [item[2]["title"] for item in direct])
        self.assertEqual([(item[2]["title"], item[3]) for item in linked],
                         [("Quibble deep", "Quibble frobnicators wobble hub")])
        self.assertEqual(titles.index("Quibble deep"), 3)
        self.assertNotIn("Unrelated", titles)
        self.assertEqual(len(pool), 5)


if __name__ == "__main__":
    unittest.main()
