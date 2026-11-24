"""The knowledge index's hybrid engine (rbxrag.engine) against Qdrant in memory and stand-in embedders."""
import json
import math
import os
import re
import shutil
import tempfile
import time
import unittest
from unittest import mock
import zlib

from rbxrag import chunks
from rbxrag.engine import PLANS, Engine
from rbxrag.stores import MemStore, QdrantStore

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

    def rerank(self, query, texts, max_tokens=None):
        self.reranked = getattr(self, "reranked", []) + [(len(texts), max_tokens)]
        q = set(words(query))
        return [float(len(q & set(words(t)))) for t in texts]


class ConstantDense(FakeEmbedders):
    """A useless dense model: every text gets the same vector."""

    def _dense(self, text):
        return [1.0] + [0.0] * (self.dim - 1)


def write(path, text, bump=0):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    if bump:
        st = os.stat(path)
        os.utime(path, ns=(st.st_atime_ns, st.st_mtime_ns + bump))
    return path


class EngineCases:
    """Run against both stores (QdrantEngineTests, MemEngineTests below)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="rag engine é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.files = []  # (shelf, title, path)
        self.game = {}  # project -> [(title, path)]
        self.a = write(os.path.join(self.tmp, "kit", "Alpha.md"), "# Alpha\n\n## Coins\n\nPlayers collect shiny coins in the meadow.\n")
        self.b = write(os.path.join(self.tmp, "kit", "Bravo.md"), "# Bravo\n\n## Lanterns\n\nA haunted forest where a lantern keeps ghosts away.\n")
        self.c = write(os.path.join(self.tmp, "kit", "Charlie.md"), "# Charlie\n\n## Rockets\n\nBuild a rocket from scrap metal and fuel.\n")
        self.files = [("guides", "Alpha", self.a), ("vault", "Bravo", self.b), ("yours", "Charlie", self.c)]
        self.manifest = os.path.join(self.tmp, "rag", "manifest.json")
        self.setup_store()

    def corpus(self, project):
        return list(self.files) + [("game", t, p) for t, p in self.game.get(project, [])]

    def project_files(self, project):
        return {p for _, p in self.game.get(project, [])}

    def engine(self, emb=None, keyword=None, plan=None):
        """A new engine on the same manifest and saved index, as after a restart of the service."""
        return Engine(self.new_store(), emb or FakeEmbedders(), self.manifest, corpus=self.corpus,
                      project_files=self.project_files, keyword=keyword, plan=plan)

    def fillers(self, n=60):
        for i in range(n):
            p = write(os.path.join(self.tmp, "kit", "Filler %d.md" % i),
                      "# Filler %d\n\n## Events\n\nConnect to an event and react when the property changes.\n" % i)
            self.files.append(("vault", "Filler %d" % i, p))

    def test_keyword_ranking_joins_the_fusion(self):
        # a useless dense model and no sparse (Light's plan): the keyword search's first result still comes first
        self.fillers()
        target = write(os.path.join(self.tmp, "kit", "Signals.md"),
                       "# Signals\n\n## Watching one property\n\nUse GetPropertyChangedSignal for one property.\n")
        self.files.append(("vault", "Signals", target))
        asked = []

        def keyword(q, shelves, depth, project):
            asked.append((q, shelves, depth, project))
            return [("vault", "Signals", None), ("vault", "Not indexed", None)]
        eng = self.engine(ConstantDense(), keyword, PLANS["light"])
        eng.sync()
        hits = eng.query("GetPropertyChangedSignal", ["vault"], 3, None)
        self.assertEqual(hits[0]["title"], "Signals")
        self.assertEqual(asked, [("GetPropertyChangedSignal", ["vault"], 20, "")])
        self.assertFalse(getattr(eng.emb, "reranked", []), "Light's plan reads no reranker")

    def test_keyword_section_key_is_that_section(self):
        doc = write(os.path.join(self.tmp, "kit", "Delta.md"),
                    "# Delta\n\n## Boats\n\nSail the boats.\n\n## Trains\n\nRide the trains.\n")
        self.files.append(("guides", "Delta", doc))
        eng = self.engine(ConstantDense(), lambda *a: [("guides", "Delta", 1)], PLANS["light"])
        eng.sync()
        hits = eng.query("zzz", ["guides"], 1, None)
        self.assertEqual((hits[0]["title"], hits[0]["heading"]), ("Delta", "Trains"))

    def test_keyword_failure_leaves_dense(self):
        def broken(*a):
            raise OSError("vault unreadable")
        eng = self.engine(None, broken, PLANS["light"])
        eng.sync()
        hits = eng.query("haunted forest lantern ghosts", ["guides", "vault", "yours"], 3, None)
        self.assertEqual(hits[0]["title"], "Bravo")

    def test_keyword_keys_obey_the_filters(self):
        pa, pb = os.path.join(self.tmp, "A"), os.path.join(self.tmp, "B")
        ga = write(os.path.join(pa, "GAME.md"), "# A\n\n## Pitch\n\nA lantern game about haunted forests.\n")
        self.game = {pa: [("GAME.md", ga)]}
        eng = self.engine(None, lambda *a: [("game", "GAME.md", 1), ("game", "GAME.md", None)], PLANS["light"])
        eng.sync()
        eng.sync(pa)
        self.assertNotIn(ga, [h["path"] for h in eng.query("lantern", ["game", "vault"], 5, pb)])
        self.assertIn(ga, [h["path"] for h in eng.query("lantern", ["game", "vault"], 5, pa)])

    def test_heavy_plan_bounds_the_reranker(self):
        self.fillers(40)
        emb = FakeEmbedders()
        eng = self.engine(emb, lambda *a: [], PLANS["heavy"])
        eng.sync()
        hits = eng.query("react when the property changes", ["vault"], 5, None)
        self.assertEqual(len(hits), 5)
        self.assertEqual(emb.reranked, [(PLANS["heavy"]["rerank_depth"], PLANS["heavy"]["rerank_tokens"])])

    def manifest_text(self):
        with open(self.manifest, encoding="utf-8") as f:
            return f.read()

    def count_path(self, eng, path):
        return eng.store.count_path(path)

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
        self.assertEqual(set(man["files"][self.b]), {"mtime_ns", "size", "ids", "project", "sha1"})

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
        emb = ConstantDense()
        eng = self.engine(emb)
        eng.sync()
        # the dense list (40 of 61 equal scores) may well miss it: the sparse list brings it to the reranker
        cands = eng.store.search(emb.dense_query("x"), emb.sparse_query("GetPropertyChangedSignal"),
                                 eng._filter(["vault"], None))
        self.assertIn("Signals", [c["title"] for c in cands])
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

    def test_failed_rebuild_never_looks_complete(self):
        first = self.engine().sync()

        class Other(FakeEmbedders):
            model = "fake-other"

            def dense(self, texts):
                raise RuntimeError("model fell over")
        with self.assertRaises(RuntimeError):
            self.engine(Other()).sync()
        with open(self.manifest, encoding="utf-8") as f:
            man = json.load(f)
        self.assertEqual((man["model"], man["files"]), ("fake-other", {}))  # invalidated before the store was emptied
        # the old model's engine does not take the emptied store for its index: it rebuilds
        self.assertEqual(self.engine().sync()["embedded"], first["chunks"])

    def test_killed_build_resumes(self):
        for i in range(12):
            p = write(os.path.join(self.tmp, "kit", "More %d.md" % i), "# More %d\n\n## Part\n\nWords number %d here.\n" % (i, i))
            self.files.append(("vault", "More %d" % i, p))
        calls = {"n": 0}

        class Dies(FakeEmbedders):
            def dense(self, texts):
                calls["n"] += 1
                if calls["n"] == 4:
                    raise RuntimeError("killed")
                return super().dense(texts)
        eng = self.engine(Dies())
        eng.batch, eng.checkpoint_batches = 2, 1
        with self.assertRaises(RuntimeError):
            eng.sync()
        with open(self.manifest, encoding="utf-8") as f:
            man = json.load(f)
        self.assertTrue(man["files"])  # the checkpoint lists what was fully in
        fresh = self.engine()
        stats = fresh.sync()
        total = sum(len(chunks.chunks_for_file(s, t, p, fresh.emb.count_tokens)) for s, t, p in self.files)
        self.assertEqual(stats["chunks"], total)
        self.assertEqual(stats["embedded"], total - 6)  # three batches of two were in before it died
        self.assertEqual(sum(self.count_path(fresh, p) for _, _, p in self.files), total)
        self.assertEqual(fresh.query("words number 7", ["vault"], 1, None)[0]["title"], "More 7")

    def test_touched_but_unchanged_file_is_not_embedded(self):
        eng = self.engine()
        eng.sync()
        with open(self.b, encoding="utf-8") as f:
            text = f.read()
        write(self.b, text, bump=10 ** 9)  # saved again, same content
        stats = eng.sync()
        self.assertEqual(stats["embedded"], 0)
        with open(self.manifest, encoding="utf-8") as f:
            self.assertEqual(json.load(f)["files"][self.b]["mtime_ns"], os.stat(self.b).st_mtime_ns)
        self.assertEqual(eng.sync()["embedded"], 0)

    def test_nothing_changed_writes_nothing(self):
        eng = self.engine()
        eng.sync()
        before = os.stat(self.manifest)
        stats = eng.sync()
        after = os.stat(self.manifest)
        self.assertEqual((stats["embedded"], stats["removed"]), (0, 0))
        self.assertEqual((before.st_ino, before.st_mtime_ns), (after.st_ino, after.st_mtime_ns))

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


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class QdrantEngineTests(EngineCases, unittest.TestCase):
    def setup_store(self):
        self.client = QdrantClient(":memory:")

    def new_store(self):
        return QdrantStore(self.client)


class MemEngineTests(EngineCases, unittest.TestCase):
    def setup_store(self):
        self.folder = os.path.join(self.tmp, "rag", "light")

    def new_store(self):
        return MemStore(self.folder)

    def test_saved_index_reloads_and_mismatch_rebuilds(self):
        eng = self.engine()
        first = eng.sync()
        again = self.engine()
        self.assertEqual(again.sync()["embedded"], 0)  # loaded from its base and deltas
        self.assertEqual(again.query("haunted forest lantern", ["vault"], 1, None)[0]["title"], "Bravo")
        with open(self.manifest, encoding="utf-8") as f:
            man = json.load(f)
        man["generation"] = "not-the-saved-one"
        with open(self.manifest, "w", encoding="utf-8") as f:
            json.dump(man, f)
        self.assertEqual(self.engine().sync()["embedded"], first["chunks"])
        for f in os.listdir(self.folder):
            if f.startswith("base-") and f.endswith(".jsonl"):
                os.remove(os.path.join(self.folder, f))
        self.assertEqual(self.engine().sync()["embedded"], first["chunks"])

    def store_files(self, prefix):
        return sorted(f for f in os.listdir(self.folder) if f.startswith(prefix))

    def test_one_changed_note_writes_a_delta_and_reloads(self):
        self.fillers(20)
        eng = self.engine(plan=PLANS["light"])
        eng.sync()
        base, deltas = self.store_files("base-"), self.store_files("delta-")
        write(self.b, "# Bravo\n\n## Lanterns\n\nA sunken temple where a lantern shows the way.\n", bump=10 ** 9)
        stats = eng.sync()
        self.assertEqual(stats["embedded"], 1)  # the changed note's one chunk, nothing else
        self.assertEqual(self.store_files("base-"), base)  # the base is not written again
        self.assertEqual(len(self.store_files("delta-")), len(deltas) + 2)  # one delta: .npz and .jsonl
        again = self.engine(plan=PLANS["light"])  # a restart reads the base and the delta
        self.assertEqual(again.sync()["embedded"], 0)
        hit = again.query("sunken temple", ["vault"], 1, None)[0]
        self.assertEqual(hit["title"], "Bravo")
        self.assertIn("sunken temple", hit["text"])  # read back from the texts file
        self.assertEqual(sum(self.count_path(again, p) for _, _, p in self.files), stats["chunks"])

    def test_many_commits_compact_into_a_new_base(self):
        from rbxrag import stores
        eng = self.engine(plan=PLANS["light"])
        eng.sync()
        old = self.store_files("base-")
        for n in range(stores.MAX_DELTAS + 2):
            write(self.b, "# Bravo\n\n## Lanterns\n\nVersion %d of the lantern note.\n" % n, bump=(n + 1) * 10 ** 9)
            eng.sync()
        self.assertNotEqual(self.store_files("base-"), old)
        self.assertLessEqual(len(self.store_files("delta-")), 2 * (stores.MAX_DELTAS + 1))
        self.assertEqual(eng.store.n - eng.store.dead, eng.chunks)
        again = self.engine(plan=PLANS["light"])
        self.assertEqual(again.sync()["embedded"], 0)
        self.assertIn("Version %d" % (stores.MAX_DELTAS + 1), again.query("lantern note", ["vault"], 1, None)[0]["text"])

    def test_commit_without_its_manifest_rolls_back(self):
        eng = self.engine()
        eng.sync()
        before, deltas = self.manifest_text(), self.store_files("delta-")
        write(self.b, "# Bravo\n\n## Lanterns\n\nA sunken temple where a lantern shows the way.\n", bump=10 ** 9)
        eng.sync()
        with open(self.manifest, "w", encoding="utf-8") as f:  # as if it died between the commit and the manifest
            f.write(before)
        again = self.engine()
        self.assertEqual(again.sync()["embedded"], 1)  # back at the manifest's generation, then the change again
        self.assertEqual(again.query("sunken temple", ["vault"], 1, None)[0]["title"], "Bravo")
        # the orphan delta is gone, the redone change is one new delta
        self.assertEqual(len(self.store_files("delta-")), len(deltas) + 2)


    def test_light_plan_stores_no_sparse_vectors(self):
        class Counting(FakeEmbedders):
            sparse_calls = 0

            def sparse(self, texts):
                Counting.sparse_calls += len(texts)
                return super().sparse(texts)
        self.engine(Counting(), plan=PLANS["light"]).sync()
        self.assertEqual(Counting.sparse_calls, 0)
        self.engine(Counting()).sync(full=True)  # the default plan ranks with them
        self.assertGreater(Counting.sparse_calls, 0)


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class StoreEquivalenceTests(unittest.TestCase):
    """MemStore ranks like Qdrant: the same fused candidates for the same points and query."""

    def test_mem_matches_qdrant(self):
        tmp = tempfile.mkdtemp(prefix="rag eq ")
        self.addCleanup(shutil.rmtree, tmp, True)
        topics = ["coins meadow shiny collect", "lantern haunted forest ghosts", "rocket scrap metal fuel",
                  "datastore session lock profile", "sword combat hitbox damage", "shop gamepass purchase robux",
                  "pets eggs hatch rarity", "leaderboard ordered datastore top", "teleport reserved server party",
                  "mobile buttons touch phone ui", "particles vfx beam trail", "npc pathfinding enemy chase",
                  "round timer intermission lobby", "music sound volume audio", "map terrain water level"]
        files = []
        for i, t in enumerate(topics):
            words_ = t.split()
            body = "## %s\n\n%s and more %s with %s.\n\n## Notes %d\n\n%s %s tips.\n" % (
                words_[0].title(), t, words_[1], words_[-1], i, words_[2], words_[0])
            files.append(("vault" if i % 3 else "guides", "Note %d" % i,
                          write(os.path.join(tmp, "Note %d.md" % i), "# Note %d\n\n" % i + body)))
        corpus = lambda project: list(files)
        q_eng = Engine(QdrantStore(QdrantClient(":memory:")), FakeEmbedders(), os.path.join(tmp, "q.json"),
                       corpus=corpus, project_files=lambda p: set())
        # 32-bit vectors here: the stores must rank alike exactly (Light's 16-bit ones may swap near ties)
        m_eng = Engine(MemStore(os.path.join(tmp, "light"), dtype="float32"), FakeEmbedders(), os.path.join(tmp, "m.json"),
                       corpus=corpus, project_files=lambda p: set())
        q_eng.sync()
        m_eng.sync()
        emb = FakeEmbedders()
        for q in ["shiny coins", "haunted lantern", "session lock datastore", "phone touch buttons", "zzz unknown",
                  "leaderboard top datastore", "pets"]:
            for shelves in (["vault", "guides"], ["guides"]):
                flt = q_eng._filter(shelves, None)
                args = ([float(x) for x in emb.dense_query(q)], emb.sparse_query(q), flt)
                # points sharing no word with the query all score 0 and their order among themselves is arbitrary
                # in both stores; the points that match must come out the same, in the same order
                qw = set(words(q))
                a = [p["id"] for p in q_eng.store.search(*args) if qw & set(words(p["text"]))]
                b = [p["id"] for p in m_eng.store.search(*args) if qw & set(words(p["text"]))]
                self.assertEqual(a, b, q)
                self.assertEqual(len(q_eng.store.search(*args)), len(m_eng.store.search(*args)), q)
                ha = [h["id"] for h in q_eng.query(q, shelves, 3, None) if h["score"] > 0]
                hb = [h["id"] for h in m_eng.query(q, shelves, 3, None) if h["score"] > 0]
                self.assertEqual(ha[:1], hb[:1], q)


if __name__ == "__main__":
    unittest.main()


class OllamaHiccupTest(unittest.TestCase):
    """Ollama's own worker can drop a connection mid-build ('Post .../tokenize: EOF'); one hiccup must not end it."""

    def _dense(self, replies):
        from rbxrag import models
        d = models.OllamaDense("http://127.0.0.1:9")
        calls = []

        def post(inputs):
            calls.append(len(inputs))
            r = replies.pop(0) if replies else None
            if isinstance(r, Exception):
                raise r
            return [[1.0, 0.0] for _ in inputs]
        d._post = post
        return d, calls

    def test_a_dropped_connection_is_sent_again(self):
        from rbxrag import models
        d, calls = self._dense([models.OllamaError('Post "http://127.0.0.1:64738/tokenize": EOF')])
        with mock.patch.object(models.time, "sleep"):
            self.assertEqual(len(d.embed(["a", "b"])), 2)
        self.assertEqual(calls, [2, 2])

    def test_a_real_error_is_not_retried(self):
        from rbxrag import models
        d, calls = self._dense([models.OllamaError('model "mxbai-embed-large" not found, try pulling it first')])
        with mock.patch.object(models.time, "sleep"), self.assertRaises(models.OllamaError):
            d.embed(["a"])
        self.assertEqual(calls, [1])

    def test_it_gives_up_after_a_few_tries(self):
        from rbxrag import models
        d, calls = self._dense([models.OllamaError("EOF")] * 10)
        with mock.patch.object(models.time, "sleep"), self.assertRaises(models.OllamaError):
            d.embed(["a"])
        self.assertEqual(len(calls), models.OLLAMA_TRIES)

    def test_big_batches_go_in_small_calls(self):
        from rbxrag import models
        d, calls = self._dense([])
        self.assertEqual(len(d.embed(["x"] * 150)), 150)
        self.assertEqual(calls, [64, 64, 22])
