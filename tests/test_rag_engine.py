"""The knowledge index's hybrid engine (rbxrag.engine) against Qdrant in memory and stand-in embedders."""
import json
import math
import os
import re
import shutil
import tempfile
import time
import unittest
import zlib

from rbxrag import chunks
from rbxrag.engine import Engine

try:
    from qdrant_client import QdrantClient, models
except ImportError:  # pragma: no cover
    QdrantClient = None


def words(text):
    return re.findall(r"[a-z0-9]+", text.lower())


class FakeEmbedders:
    """Dense: a 64-dim hashed bag of words. Sparse: hashed term counts. Rerank: term overlap."""
    dim = 64
    model = "fake-64"

    def __init__(self):
        self.count_tokens = chunks.make_token_counter(None)
        self.dense_calls = 0

    def _dense(self, text):
        v = [0.0] * self.dim
        for w in words(text):
            v[zlib.crc32(w.encode()) % self.dim] += 1.0
        n = math.sqrt(sum(x * x for x in v)) or 1.0
        return [x / n for x in v] if any(v) else [1.0] + [0.0] * (self.dim - 1)

    def dense(self, texts):
        self.dense_calls += len(texts)
        return [self._dense(t) for t in texts]

    def dense_query(self, text):
        return self._dense(text)

    def _sparse(self, text):
        c = {}
        for w in words(text):
            i = zlib.crc32(w.encode()) % 1000003
            c[i] = c.get(i, 0.0) + 1.0
        keys = sorted(c)
        return keys, [c[k] for k in keys]

    def sparse(self, texts):
        return [self._sparse(t) for t in texts]

    def sparse_query(self, text):
        return self._sparse(text)

    def rerank(self, query, texts):
        q = set(words(query))
        return [float(len(q & set(words(t)))) for t in texts]


class ConstantDense(FakeEmbedders):
    """A useless dense model: every text gets the same vector."""

    def _dense(self, text):
        return [1.0] + [0.0] * (self.dim - 1)

    def rerank(self, query, texts):
        return [0.0 for _ in texts]  # no help from the reranker either: the fused order stands


def write(path, text, bump=0):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    if bump:
        st = os.stat(path)
        os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns + bump))
    return path


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class EngineTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rag engine é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.files = []  # (shelf, title, path)
        self.game = {}  # project -> [(title, path)]
        self.a = write(os.path.join(self.tmp, "kit", "Alpha.md"), "# Alpha\n\n## Coins\n\nPlayers collect shiny coins in the meadow.\n")
        self.b = write(os.path.join(self.tmp, "kit", "Bravo.md"), "# Bravo\n\n## Lanterns\n\nA haunted forest where a lantern keeps ghosts away.\n")
        self.c = write(os.path.join(self.tmp, "kit", "Charlie.md"), "# Charlie\n\n## Rockets\n\nBuild a rocket from scrap metal and fuel.\n")
        self.files = [("guides", "Alpha", self.a), ("vault", "Bravo", self.b), ("yours", "Charlie", self.c)]
        self.client = QdrantClient(":memory:")
        self.manifest = os.path.join(self.tmp, "rag", "manifest.json")

    def corpus(self, project):
        return list(self.files) + [("game", t, p) for t, p in self.game.get(project, [])]

    def project_files(self, project):
        return {p for _, p in self.game.get(project, [])}

    def engine(self, emb=None):
        return Engine(self.client, emb or FakeEmbedders(), self.manifest, corpus=self.corpus,
                      project_files=self.project_files)

    def manifest_text(self):
        with open(self.manifest, encoding="utf-8") as f:
            return f.read()

    def count_path(self, eng, path):
        f = models.Filter(must=[models.FieldCondition(key="path", match=models.MatchValue(value=path))])
        return self.client.count(eng.collection, count_filter=f, exact=True).count

    def test_sync_then_query_finds_note(self):
        eng = self.engine()
        self.assertFalse(eng.ready)
        stats = eng.sync()
        self.assertGreater(stats["chunks"], 0)
        self.assertEqual(stats["files"], 3)
        self.assertEqual(stats["embedded"], stats["chunks"])
        self.assertTrue(eng.ready)
        hits = eng.query("haunted forest lantern ghosts", ["guides", "vault", "yours"], 3, None)
        self.assertEqual(hits[0]["title"], "Bravo")
        self.assertEqual(hits[0]["heading"], "Lanterns")
        self.assertIn("score", hits[0])
        with open(self.manifest, encoding="utf-8") as f:
            man = json.load(f)
        self.assertEqual(man["format"], 1)
        self.assertEqual(man["dim"], 64)
        self.assertEqual(man["model"], "fake-64")
        self.assertIn(self.b, man["files"])
        self.assertEqual(set(man["files"][self.b]), {"mtime_ns", "size", "ids", "project"})

    def test_sync_reembeds_only_changed(self):
        eng = self.engine()
        eng.sync()
        self.assertEqual(eng.sync()["embedded"], 0)
        write(self.c, "# Charlie\n\n## Rockets\n\nBuild a rocket from scrap metal and fuel.\n\n## Moon\n\n"
                      "Land the rocket softly on the purple moon.\n", bump=10 ** 9)
        n = len(chunks.chunks_for_file("yours", "Charlie", self.c, eng.emb.count_tokens))
        stats = eng.sync()
        self.assertEqual(stats["embedded"], n)
        hits = eng.query("purple moon landing", ["yours"], 2, None)
        self.assertEqual(hits[0]["heading"], "Moon")
        # a fresh engine on the same manifest and index re-embeds nothing
        again = self.engine()
        self.assertEqual(again.sync()["embedded"], 0)

    def test_removed_file_points_deleted(self):
        eng = self.engine()
        eng.sync()
        self.assertGreater(self.count_path(eng, self.a), 0)
        os.remove(self.a)
        self.files = [f for f in self.files if f[2] != self.a]
        stats = eng.sync()
        self.assertEqual(stats["removed"], 1)
        self.assertEqual(self.count_path(eng, self.a), 0)
        self.assertEqual(stats["files"], 2)
        with open(self.manifest, encoding="utf-8") as f:
            self.assertNotIn(self.a, json.load(f)["files"])

    def test_exact_identifier_found_by_sparse(self):
        for i in range(60):
            p = write(os.path.join(self.tmp, "kit", "Filler %d.md" % i),
                      "# Filler %d\n\n## Events\n\nConnect to an event and react when the property changes.\n" % i)
            self.files.append(("vault", "Filler %d" % i, p))
        target = write(os.path.join(self.tmp, "kit", "Signals.md"),
                       "# Signals\n\n## Watching one property\n\nUse GetPropertyChangedSignal for one property.\n")
        self.files.append(("vault", "Signals", target))
        eng = self.engine(ConstantDense())
        eng.sync()
        hits = eng.query("GetPropertyChangedSignal", ["vault"], 3, None)
        self.assertEqual(hits[0]["title"], "Signals")

    def test_shelf_and_project_filter(self):
        pa, pb = os.path.join(self.tmp, "A"), os.path.join(self.tmp, "B")
        ga = write(os.path.join(pa, "GAME.md"), "# A\n\n## Pitch\n\nA lantern game about haunted forests.\n")
        gb = write(os.path.join(pb, "GAME.md"), "# B\n\n## Pitch\n\nA racing game with go karts.\n")
        self.game = {pa: [("GAME.md", ga)], pb: [("GAME.md", gb)]}
        eng = self.engine()
        eng.sync()
        eng.sync(pa)
        eng.sync(pb)
        hits = eng.query("haunted forest lantern", ["game", "vault"], 5, pb)
        self.assertNotIn(ga, [h["path"] for h in hits])
        hits = eng.query("haunted forest lantern", ["game"], 5, pa)
        self.assertEqual([h["path"] for h in hits], [ga])
        self.assertEqual(hits[0]["project"], pa)
        hits = eng.query("haunted forest lantern", ["game"], 5, None)
        self.assertEqual(hits, [])
        hits = eng.query("rocket coins lantern", ["yours"], 5, None)
        self.assertTrue(hits)
        self.assertEqual({h["shelf"] for h in hits}, {"yours"})
        # syncing one project never drops another project's files
        self.assertGreater(self.count_path(eng, ga), 0)
        self.assertGreater(self.count_path(eng, gb), 0)

    def test_team_override_does_not_drop_kit_expert(self):
        kit = write(os.path.join(self.tmp, "kit", "team", "maya.md"), "# Maya\n\n## Craft\n\nMaya paints glowing mushrooms.\n")
        proj = os.path.join(self.tmp, "P")
        mine = write(os.path.join(proj, ".fridae", "team", "experts", "maya.md"),
                     "# Maya here\n\n## Craft\n\nIn this game Maya paints glowing mushrooms blue.\n")
        self.files.append(("team", "maya", kit))
        base = self.corpus

        def corpus(project):  # like corpus_files: the game's override replaces the kit's expert
            out = base(project)
            return [f for f in out if f[2] != kit] + [("team", "maya", mine)] if project == proj else out
        self.corpus = corpus
        self.project_files = lambda project: {mine} if project == proj else set()
        eng = self.engine()
        eng.sync()
        eng.sync(proj)
        self.assertGreater(self.count_path(eng, kit), 0)  # still there for every other chat
        self.assertGreater(self.count_path(eng, mine), 0)
        eng.sync()
        self.assertEqual(eng.sync(proj)["embedded"], 0)  # no back-and-forth re-embedding
        hits = eng.query("glowing mushrooms", ["team"], 3, proj)
        self.assertEqual([h["path"] for h in hits], [mine])
        hits = eng.query("glowing mushrooms", ["team"], 3, None)
        self.assertEqual([h["path"] for h in hits], [kit])

    def test_model_change_rebuilds(self):
        eng = self.engine()
        first = eng.sync()
        with open(self.manifest, encoding="utf-8") as f:
            man = json.load(f)
        man["model"] = "another-model"
        with open(self.manifest, "w", encoding="utf-8") as f:
            json.dump(man, f)
        stats = self.engine().sync()
        self.assertEqual(stats["embedded"], first["chunks"])
        with open(self.manifest, encoding="utf-8") as f:
            self.assertEqual(json.load(f)["model"], "fake-64")

    def test_progress_and_manifest_only_after_success(self):
        seen = []
        eng = self.engine()
        eng.sync(progress=lambda step, frac: seen.append((step, frac)))
        self.assertTrue(seen)
        self.assertEqual(seen[-1][1], 1.0)
        write(self.a, "# Alpha\n\n## Coins\n\nNew coin text.\n", bump=10 ** 9)
        before = self.manifest_text()

        class Broken(FakeEmbedders):
            def dense(self, texts):
                raise RuntimeError("model fell over")
        broken = self.engine(Broken())
        with self.assertRaises(RuntimeError):
            broken.sync()
        self.assertEqual(self.manifest_text(), before)


if __name__ == "__main__":
    unittest.main()
