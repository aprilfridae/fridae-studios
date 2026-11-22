"""The knowledge lookup: section parsing, shelves, read caps, the game shelf, caches, rbx_guide outlines, relevance."""
import json
import os
import re
import shutil
import tempfile
import time
import unittest
from unittest import mock

import rbxkit_mcp
import rbxpaths
import rbxvault
from rbxpaths import KitError
from tests.knowledge_bench import TASKS

_WORK = None


def setUpModule():
    """One temporary work folder for the whole module: index caches never touch the user's real work folder."""
    global _WORK, _PATCH
    _WORK = tempfile.mkdtemp(prefix="knowledge work é ")
    _PATCH = mock.patch.object(rbxpaths, "WORK", _WORK)
    _PATCH.start()


def tearDownModule():
    _PATCH.stop()
    shutil.rmtree(_WORK, True)


def no_project():
    return mock.patch.dict(os.environ, {"RBXKIT_PROJECT": ""})


SAMPLE = """---
name: sample-guide
description: >
  A sample guide about saving coins
  and menus.
tags: [one, two]
---

# Sample Guide

Opening words for the guide.

## Saving coins

Use UpdateAsync for coins.

```lua
# not a heading inside a fence
## also not a heading
local x = 1
```

### Retries

Retry with backoff.

## Café menüs — ünïcode

Les menus du café.

~~~
## fenced with tildes
~~~

## Checklist

- one
"""


class SectionParsingTests(unittest.TestCase):
    def test_front_matter_label_and_sections(self):
        p = rbxvault.parse_sections(SAMPLE)
        self.assertEqual(p["fm"]["name"], "sample-guide")
        self.assertEqual(p["description"], "A sample guide about saving coins and menus.")
        self.assertEqual(p["label"], "Sample Guide")
        heads = [s["heading"] for s in p["sections"]]
        self.assertEqual(heads, [rbxvault.OPENING, "Saving coins", "Retries", "Café menüs — ünïcode", "Checklist"])

    def test_code_fences_hide_hashes(self):
        p = rbxvault.parse_sections(SAMPLE)
        saving = p["sections"][1]
        text = p["body"][saving["start"]:saving["end"]]
        self.assertIn("# not a heading inside a fence", text)
        self.assertIn("### Retries", text, "a read of a ## section includes its ### parts")
        own = p["body"][saving["start"]:saving["own_end"]]
        self.assertNotIn("Retry with backoff", own)
        self.assertNotIn("fenced with tildes", [s["heading"] for s in p["sections"]])
        self.assertEqual(p["sections"][2]["parents"], ["Saving coins"])

    def test_no_front_matter_no_headings(self):
        p = rbxvault.parse_sections("Just a few lines\nof text.\n")
        self.assertEqual(p["fm"], {})
        self.assertEqual([s["heading"] for s in p["sections"]], [rbxvault.WHOLE])

    def test_find_section(self):
        p = rbxvault.parse_sections(SAMPLE)
        self.assertEqual(rbxvault.find_section(p, "saving coins")["heading"], "Saving coins")
        self.assertEqual(rbxvault.find_section(p, "retr")["heading"], "Retries")
        self.assertEqual(rbxvault.find_section(p, "café")["heading"], "Café menüs — ünïcode")
        self.assertEqual(rbxvault.find_section(p, "Saving coins > Retries")["heading"], "Retries")
        self.assertEqual(rbxvault.find_section(p, "opening")["heading"], rbxvault.OPENING)
        self.assertIsNone(rbxvault.find_section(p, "nothing like this"))

    def test_duplicate_headings_get_a_parent(self):
        text = "# T\n\n## A\n\n### Example\n\nfirst\n\n## B\n\n### Example\n\nsecond\n"
        p = rbxvault.parse_sections(text)
        second = p["sections"][-1]
        arg = rbxvault.heading_arg(p, second)
        self.assertEqual(arg, "B > Example")
        found = rbxvault.find_section(p, arg)
        self.assertIn("second", p["body"][found["start"]:found["end"]])


class Project:
    """A temporary game project whose path has a space and an é in it."""

    def __init__(self, tc):
        self.tmp = tempfile.mkdtemp(prefix="knowledge é ")
        tc.addCleanup(shutil.rmtree, self.tmp, True)
        self.path = os.path.join(self.tmp, "My Gäme é")
        os.makedirs(os.path.join(self.path, ".fridae"))

    def write(self, rel, text):
        path = os.path.join(self.path, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return path


GAME = """# Lantern Run

## One-line pitch

Carry a lantern through a haunted forest.

## Decision log

- 2026-11-01: The shop sells lantern skins only; no pay-to-win upgrades. Decided with the user.
- 2026-11-03: Phones first: every button at least 44 px.
"""


class ShelfTests(unittest.TestCase):
    def setUp(self):
        p = no_project()
        p.start()
        self.addCleanup(p.stop)

    def search(self, **a):
        return rbxvault.run("search", a)

    def test_shelf_filter(self):
        for shelf in ("guides", "team"):
            text = self.search(query="save player data", shelf=shelf)
            labels = set(re.findall(r"^\d+\. \[(\w+)\]", text, re.M))
            self.assertEqual(labels, {shelf}, text)
        text = self.search(query="save player data", shelf="vault")
        self.assertNotIn("[guides]", text)
        self.assertIn("Next: rbx_vault action 'read'", text)

    def test_all_mixes_shelves_and_gives_read_arguments(self):
        text = self.search(query="save player coins between sessions", k=8)
        labels = set(re.findall(r"^\d+\. \[(\w+)\]", text, re.M))
        self.assertTrue({"vault", "guides"} <= labels, text)
        reads = [json.loads(l.split("read: ", 1)[1]) for l in text.splitlines() if l.strip().startswith("read: ")]
        self.assertEqual(len(reads), 8)
        for args in reads:
            if args["shelf"] != "vault":
                out = rbxvault.run("read", args)
                self.assertTrue(out.startswith("[%s] %s" % (args["shelf"], args["title"])), out[:200])

    def test_bad_shelf_and_no_game(self):
        with self.assertRaises(KitError):
            self.search(query="x", shelf="attic")
        with self.assertRaises(KitError) as cm:
            self.search(query="decision", shelf="game")
        self.assertIn("RBXKIT_PROJECT", str(cm.exception))

    def test_count_and_k(self):
        self.assertEqual(len(re.findall(r"^\d+\. ", self.search(query="leaderboard", shelf="guides", k=2), re.M)), 2)
        self.assertEqual(len(re.findall(r"^\d+\. ", self.search(query="leaderboard", shelf="guides", count=4), re.M)), 4)

    def test_read_guide_section_and_listing(self):
        out = rbxvault.run("read", {"shelf": "guides", "title": "roblox-data-stores", "heading": "session locking"})
        self.assertIn("Player data: use session locking", out)
        self.assertNotIn("{{RBXKIT", out)
        listing = rbxvault.run("sections", {"title": "roblox-data-stores"})
        self.assertIn("## Choose the service", listing)
        self.assertRegex(listing, r"\d+\.\d KB")
        with self.assertRaises(KitError) as cm:
            rbxvault.run("read", {"shelf": "guides", "title": "roblox-data-stores", "heading": "zebra crossing"})
        self.assertIn("Choose the service", str(cm.exception))

    def test_team_shelf(self):
        out = rbxvault.run("read", {"shelf": "team", "title": "coder"})
        self.assertIn("YOU ARE THE CODER", out)

    def test_read_cap_lists_sub_headings(self):
        proj = Project(self)
        big = "## Huge part\n\n" + "".join("### Part %d\n\n%s\n\n" % (n, "word " * 400) for n in range(12))
        proj.write("GAME.md", "# G\n\nIntro.\n\n" + big)
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": proj.path}):
            out = rbxvault.run("read", {"shelf": "game", "title": "GAME.md", "heading": "huge part"})
        self.assertLess(len(out.encode("utf-8")), rbxvault.READ_CAP + 1500)
        self.assertIn("only its first 12 KB is shown", out)
        self.assertIn("Part 11", out.rsplit("only its first", 1)[1])


class GameShelfTests(unittest.TestCase):
    def test_game_notes_and_lessons_are_found(self):
        proj = Project(self)
        proj.write("GAME.md", GAME)
        proj.write(os.path.join(".fridae", "lessons.md"), "- 2026-11-02: Lantern flicker VFX needs a 0.2 s fade.\n")
        with open(os.path.join(rbxpaths.WORK, "studio-lessons.md"), "w", encoding="utf-8") as f:
            f.write("- 2026-10-01: Test every menu on a phone emulator.\n")
        self.addCleanup(os.remove, os.path.join(rbxpaths.WORK, "studio-lessons.md"))
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": proj.path}):
            text = rbxvault.run("search", {"query": "shop skins pay to win decision", "shelf": "game"})
            self.assertIn("[game] GAME.md", text)
            self.assertIn("Decision log", text)
            self.assertIn("lantern skins", text)
            text = rbxvault.run("search", {"query": "lantern flicker fade", "shelf": "game"})
            self.assertIn("[game] lessons.md", text)
            text = rbxvault.run("search", {"query": "phone emulator menu", "shelf": "game"})
            self.assertIn("studio-lessons.md", text)
            out = rbxvault.run("read", {"shelf": "game", "title": "GAME.md", "heading": "decision log"})
            self.assertIn("Phones first", out)
            all_text = rbxvault.run("search", {"query": "lantern skins decision log", "k": 8})
            self.assertIn("[game]", all_text)
        with no_project():
            self.assertNotIn("[game]", rbxvault.run("search", {"query": "lantern skins decision log", "k": 8}))

    def test_project_team_override(self):
        proj = Project(self)
        proj.write(os.path.join(".fridae", "team", "experts", "coder.md"), "Project coder: always use zebra modules.\n")
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": proj.path}):
            out = rbxvault.run("read", {"shelf": "team", "title": "coder"})
            self.assertIn("zebra modules", out)
            self.assertIn("zebra", rbxvault.run("search", {"query": "zebra modules", "shelf": "team"}))
        with no_project():
            self.assertNotIn("zebra", rbxvault.run("read", {"shelf": "team", "title": "coder"}))

    def test_game_shelf_never_cached(self):
        proj = Project(self)
        proj.write("GAME.md", GAME)
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": proj.path}):
            rbxvault.run("search", {"query": "lantern", "shelf": "game"})
        with open(rbxvault.cache_path(rbxvault.GUIDES_CACHE_NAME), encoding="utf-8") as f:
            self.assertNotIn("Lantern", f.read())


class CacheTests(unittest.TestCase):
    def test_cache_rebuilds_when_a_file_changes(self):
        tmp = tempfile.mkdtemp(prefix="cache é ")
        self.addCleanup(shutil.rmtree, tmp, True)
        guide = os.path.join(tmp, "g.md")
        with open(guide, "w", encoding="utf-8") as f:
            f.write("# G\n\n## Alpha\n\nfirst text about apples\n")
        files = [("guides", "g", guide)]
        with mock.patch.object(rbxpaths, "WORK", tmp), mock.patch.object(rbxvault, "_kit_files", lambda: list(files)):
            docs = rbxvault.load_sections()
            self.assertEqual([d["heading"] for d in docs], ["Alpha"])
            self.assertTrue(os.path.exists(rbxvault.cache_path(rbxvault.GUIDES_CACHE_NAME)))
            self.assertEqual(rbxvault.load_sections(), docs, "unchanged files come from the cache")
            with open(guide, "w", encoding="utf-8") as f:
                f.write("# G\n\n## Alpha\n\nfirst\n\n## Beta\n\nbananas\n")
            os.utime(guide, (time.time() + 5, time.time() + 5))
            self.assertEqual([d["heading"] for d in rbxvault.load_sections()], ["Alpha", "Beta"])
            other = os.path.join(tmp, "h.md")
            with open(other, "w", encoding="utf-8") as f:
                f.write("## Gamma\n\ncherries\n")
            files.append(("guides", "h", other))
            self.assertIn("Gamma", [d["heading"] for d in rbxvault.load_sections()])


class GuideToolTests(unittest.TestCase):
    def guide(self, **a):
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": ""}):
            res = rbxkit_mcp.t_guide(a)
        return res, res["content"][0]["text"]

    def test_large_guide_comes_as_outline(self):
        res, text = self.guide(topic="unslop")
        self.assertFalse(res["isError"])
        self.assertIn("## Sections of this guide", text)
        self.assertIn("## 2. Color", text)
        self.assertIn("unslop was created by Damir Sokolovsky", text, "the opening (with its credit) is kept")
        self.assertLess(len(text), 8000)
        self.assertIn("Reference files for this guide", text)

    def test_whole_forces_full_text(self):
        _, text = self.guide(topic="unslop", whole=True)
        self.assertIn("Build from one neutral ramp", text)
        self.assertNotIn("## Sections of this guide", text)

    def test_section(self):
        res, text = self.guide(topic="unslop", section="color")
        self.assertFalse(res["isError"])
        self.assertIn("## 2. Color", text)
        self.assertNotIn("## 3. Icons", text)
        res, text = self.guide(topic="unslop", section="no such heading here")
        self.assertTrue(res["isError"])
        self.assertIn("2. Color", text)
        res, text = self.guide(topics=["roblox-luau", "unslop"], section="types")
        self.assertIn("not loaded: unslop", text)
        res, text = self.guide(topic="roblox-trend-game/trend-games-research", section="sources")
        self.assertFalse(res["isError"], text)

    def test_always_whole_and_small_guides(self):
        self.assertEqual(rbxkit_mcp.ALWAYS_WHOLE, ("workflow", "studio", "critic", "producer", "game-bible"))
        for topic in rbxkit_mcp.ALWAYS_WHOLE + ("playtest", "promo"):
            _, text = self.guide(topic=topic)
            self.assertNotIn("## Sections of this guide", text, topic)
            self.assertIn(rbxkit_mcp._one_guide(topic)[:500], text, topic)
        self.assertGreater(len(rbxkit_mcp._one_guide("workflow").encode("utf-8")), rbxkit_mcp.OUTLINE_OVER)

    def test_every_guide_still_loads(self):
        for topic in rbxkit_mcp.GUIDES:
            text = rbxkit_mcp.guide_text(topic)
            self.assertTrue(text, topic)
            if topic not in rbxkit_mcp.ALWAYS_WHOLE:
                self.assertLess(len(text.encode("utf-8")), rbxkit_mcp.OUTLINE_OVER + 6000, topic)

    def test_tool_schema(self):
        tools = {t["name"]: t for t in rbxkit_mcp.handle({"method": "tools/list"})["tools"]}
        g = tools["rbx_guide"]["inputSchema"]["properties"]
        self.assertIn("section", g)
        self.assertIn("whole", g)
        v = tools["rbx_vault"]["inputSchema"]["properties"]
        for k in ("action", "query", "shelf", "k", "count", "title", "heading", "folder", "names"):
            self.assertIn(k, v)
        self.assertEqual(v["shelf"]["enum"], ["all", "vault", "guides", "team", "game"])


class VaultUnchangedTests(unittest.TestCase):
    """The vault on its own ranks exactly as it did before the shelves were added."""

    def test_top_notes_match_the_baseline(self):
        with open(os.path.join(os.path.dirname(__file__), "vault_baseline.json"), encoding="utf-8") as f:
            base = json.load(f)
        idx = rbxvault.load_index()
        for q, titles in base.items():
            got = [d["title"] for _, d in rbxvault.search(q, 5, idx)[0]]
            self.assertEqual(got, titles, q)

    def test_vault_shelf_output_is_the_engine_output(self):
        import contextlib
        import io
        import sys
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            sys.argv = ["vault", "-k", "3", "datastore session lock"]
            rbxvault.main()
        out = rbxvault.run("search", {"query": "datastore session lock", "shelf": "vault"})
        self.assertEqual(out, buf.getvalue().strip() + "\n\nNext: rbx_vault action 'read' with a title and heading "
                                                      "words for one section (not the whole note).")
        out = rbxvault.run("read", {"title": "Session Locking and Profile Libraries", "heading": "lease"})
        self.assertIn("lease", out.lower())
        self.assertIn("Folders:", rbxvault.run("list", {}))
        self.assertIn("deprecated", rbxvault.run("api", {"names": "TeleportService.Teleport"}) + "deprecated")

    def test_no_stdout(self):
        import contextlib
        import io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf), no_project():
            rbxvault.run("search", {"query": "leaderboard"})
            rbxvault.run("read", {"title": "OrderedDataStores and Leaderboards"})
            rbxvault.run("api", {"names": "BadgeService.AwardBadge"})
        self.assertEqual(buf.getvalue(), "")


class RelevanceTests(unittest.TestCase):
    def test_bench_tasks_find_an_expected_guide_in_the_top_five(self):
        with no_project():
            for task, _, expected in TASKS:
                text = rbxvault.run("search", {"query": task, "shelf": "guides", "k": 5})
                tops = [t.split("/")[0] for t in re.findall(r"^\d+\. \[guides\] (\S+)", text, re.M)]
                self.assertTrue(set(tops) & set(expected), "%s: got %s, wanted one of %s" % (task, tops, expected))


if __name__ == "__main__":
    unittest.main()
