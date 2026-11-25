"""The knowledge index's corpus and its token-aware chunks (rbxrag.chunks)."""
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxpaths
import rbxvault
from rbxrag import chunks


def setUpModule():
    global _TMP, _PATCHES
    _TMP = tempfile.mkdtemp(prefix="rag chunks é ")
    _PATCHES = [mock.patch.object(rbxpaths, "WORK", os.path.join(_TMP, "work")),
                mock.patch.object(rbxpaths, "HOME_DIR", os.path.join(_TMP, "home"))]
    for p in _PATCHES:
        p.start()


def tearDownModule():
    for p in _PATCHES:
        p.stop()
    shutil.rmtree(_TMP, True)


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return path


NOTE = """---
tags: [datastore]
---

# Saving Data

Some opening words about saving.

## Session locks

Lock the profile while a server holds it.

## Retries

Retry UpdateAsync with backoff.
"""


class ChunkTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rag é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.count = chunks.make_token_counter(None)

    def test_chunk_prefix_and_ids_stable(self):
        path = write(os.path.join(self.tmp, "Saving Data.md"), NOTE)
        a = chunks.chunks_for_file("yours", "Saving Data", path, self.count)
        b = chunks.chunks_for_file("yours", "Saving Data", path, self.count)
        self.assertTrue(a)
        self.assertEqual([c["id"] for c in a], [c["id"] for c in b])
        self.assertEqual(len({c["id"] for c in a}), len(a))
        for c in a:
            self.assertTrue(c["text"].startswith("%s › %s\n" % (c["title"], c["heading"])), c["text"][:80])
            for key in ("id", "shelf", "title", "heading", "path", "rel", "i", "part", "text", "project"):
                self.assertIn(key, c)
            self.assertEqual(c["project"], "")
        self.assertIn("Session locks", [c["heading"] for c in a])

    def test_long_code_section_split_under_limit(self):
        code = "\n".join("local value_%d = compute(%d) -- a fairly long line of Luau code" % (i, i) for i in range(400))
        code = code[:20000]
        text = "# Dump\n\n## Code\n\n```lua\n" + code + "\n```\n"
        path = write(os.path.join(self.tmp, "Dump.md"), text)
        out = [c for c in chunks.chunks_for_file("yours", "Dump", path, self.count) if c["heading"] == "Code"]
        self.assertGreater(len(out), 1)
        for c in out:
            self.assertLessEqual(self.count(c["text"]), 480)
        self.assertEqual([c["part"] for c in out], list(range(len(out))))
        # one enormous line with no spaces at all is still cut under the limit
        path = write(os.path.join(self.tmp, "Blob.md"), "## Blob\n\n" + "x" * 30000 + "\n")
        out = chunks.chunks_for_file("yours", "Blob", path, self.count)
        self.assertGreater(len(out), 1)
        for c in out:
            self.assertLessEqual(self.count(c["text"]), 480)

    def test_binary_file_is_skipped(self):
        path = os.path.join(self.tmp, "bad.md")
        with open(path, "wb") as f:
            f.write(b"\xff\xfe\x00\x81 not utf-8")
        self.assertEqual(chunks.chunks_for_file("yours", "bad", path, self.count), [])

    def test_corpus_files_scoped(self):
        proj = os.path.join(self.tmp, "My Gäme")
        game = write(os.path.join(proj, "GAME.md"), "# Lantern Run\n\n## Pitch\n\nCarry a lantern.\n")
        note = write(os.path.join(proj, ".fridae", "notes", "x.md"), "A note about the lantern.\n")
        big = write(os.path.join(proj, "src", "big.lua"), "local x = 1\n" * 100)
        mine = write(os.path.join(rbxpaths.HOME_DIR, "notes", "Mine.md"), "My own note.\n")
        files = chunks.corpus_files(proj)
        paths = [p for _, _, p in files]
        self.assertIn(game, paths)
        self.assertIn(note, paths)
        self.assertNotIn(big, paths)
        self.assertIn(("yours", "Mine", mine), files)
        self.assertIn(("game", "GAME.md", game), files)
        shelves = {s for s, _, _ in files}
        self.assertTrue(shelves <= set(rbxvault.SHELVES))
        self.assertIn("guides", shelves)
        none = chunks.corpus_files(None)
        self.assertNotIn("game", {s for s, _, _ in none})
        self.assertNotIn(game, [p for _, _, p in none])

    def test_game_team_override_replaces_kit_expert(self):
        kit = chunks.corpus_files(None)
        team = [(t, p) for s, t, p in kit if s == "team"]
        if not team:
            self.skipTest("no kit team files")
        tid = team[0][0]
        proj = os.path.join(self.tmp, "Game")
        mine = write(os.path.join(proj, ".fridae", "team", "experts", tid + ".md"), "# Mine\n\nOverride.\n")
        files = chunks.corpus_files(proj)
        same = [(s, p) for s, t, p in files if s == "team" and t == tid]
        self.assertEqual(same, [("team", mine)])
        c = chunks.chunks_for_file("team", tid, mine, self.count, project=proj)
        self.assertEqual({x["project"] for x in c}, {proj})

    def test_vault_notes_come_from_knowledge(self):
        kb = os.path.join(self.tmp, "Knowledge", "Roblox")
        path = write(os.path.join(kb, "10 Data", "Saving Data.md"), NOTE)
        with mock.patch.object(rbxvault, "KB", kb):
            self.assertIn(("vault", "Saving Data", path), chunks.corpus_files(None))

    def test_vault_notes_become_sections(self):
        kb = os.path.join(self.tmp, "Roblox")
        path = write(os.path.join(kb, "10 Data", "Saving Data.md"), NOTE)
        with mock.patch.object(rbxvault, "KB", kb):
            files = [f for f in chunks.corpus_files(None) if f[0] == "vault"]
            self.assertEqual(files, [("vault", "Saving Data", path)])
            out = chunks.chunks_for_file("vault", "Saving Data", path, self.count)
        self.assertGreaterEqual(len(out), 2)
        self.assertEqual({c["shelf"] for c in out}, {"vault"})
        secs = rbxvault.parse_sections(NOTE)["sections"]
        for c in out:
            self.assertEqual(secs[c["i"]]["heading"], c["heading"])
        self.assertEqual([c["i"] for c in out], sorted(c["i"] for c in out))
        self.assertEqual(out[0]["rel"], "10 Data/Saving Data.md")
        heads = [c["heading"] for c in out]
        self.assertIn("Session locks", heads)
        self.assertIn("Retries", heads)


class CounterTests(unittest.TestCase):
    def test_fallback_is_pessimistic(self):
        count = chunks.make_token_counter(None)
        self.assertEqual(count("abc"), 1)
        self.assertEqual(count("abcd"), 2)
        self.assertEqual(count("é"), 1)

    def test_tokenizer_json_counts_without_truncation(self):
        try:
            from tokenizers import Tokenizer, models, pre_tokenizers
        except ImportError:
            self.skipTest("tokenizers not installed")
        tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, tmp, True)
        tok = Tokenizer(models.WordLevel({"[UNK]": 0, "a": 1}, unk_token="[UNK]"))
        tok.pre_tokenizer = pre_tokenizers.Whitespace()
        tok.enable_truncation(4)
        path = os.path.join(tmp, "tokenizer.json")
        tok.save(path)
        count = chunks.make_token_counter(path)
        self.assertEqual(count("a a a a a a a a a a"), 10)


if __name__ == "__main__":
    unittest.main()
