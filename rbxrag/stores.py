"""Where the knowledge index keeps its points. The Engine talks to one of two stores through the same small interface:

  reset(dim)              empty store for vectors of this size (a full rebuild)
  open(manifest) -> bool  True when the stored points match the manifest (else the Engine rebuilds)
  upsert(points)          points: [(id, dense, (sparse indices, sparse values), payload)]
  delete(ids)
  commit() -> generation  make everything upserted so far durable and visible; the Engine writes the manifest after it
  search(dense, sparse, flt) -> [payload]   the 30 best by RRF of the dense top 40 and the sparse top 40
  count_path(path) -> int

flt is {"shelves": [..] or None (no shelf condition), "project": str or None, "hide_team": [expert ids]}: shared
points (project "") always match, a game's points only for that project, and the kit's team experts a game overrides
are hidden for that game.

QdrantStore (Heavy): the Qdrant service. MemStore (Light): plain numpy arrays in memory, saved as plain files.
"""
import json
import math
import os
import tempfile
import threading
import uuid
import warnings

PREFETCH = 40  # candidates from each of the dense and the sparse search
FUSED = 30  # fused candidates the reranker sees
RRF_K = 2  # Qdrant's RRF constant: a hit at 0-based position p scores 1 / (p + 2)


class QdrantStore:
    def __init__(self, client, collection="fridae"):
        self.client = client
        self.collection = collection
        self._db = threading.RLock()  # one client call at a time (the local client is not made for threads)

    @staticmethod
    def _m():
        from qdrant_client import models
        return models

    def open(self, manifest):
        with self._db:
            return self.client.collection_exists(self.collection)

    def reset(self, dim):
        m = self._m()
        with self._db:
            if self.client.collection_exists(self.collection):
                self.client.delete_collection(self.collection)
            self.client.create_collection(
                self.collection,
                vectors_config={"dense": m.VectorParams(size=dim, distance=m.Distance.COSINE)},
                sparse_vectors_config={"sparse": m.SparseVectorParams(modifier=m.Modifier.IDF)})
            with warnings.catch_warnings():  # local mode says payload indexes have no effect there; a server uses them
                warnings.simplefilter("ignore")
                for key in ("shelf", "project", "path", "title"):
                    self.client.create_payload_index(self.collection, key, m.PayloadSchemaType.KEYWORD)

    def upsert(self, points):
        m = self._m()
        structs = [m.PointStruct(id=pid, payload=payload,
                                 vector={"dense": [float(x) for x in dense],
                                         "sparse": m.SparseVector(indices=[int(i) for i in sp[0]],
                                                                  values=[float(v) for v in sp[1]])})
                   for pid, dense, sp, payload in points]
        if structs:
            with self._db:
                self.client.upsert(self.collection, points=structs, wait=True)

    def delete(self, ids):
        m = self._m()
        ids = list(ids)
        with self._db:
            for n in range(0, len(ids), 512):
                self.client.delete(self.collection, points_selector=m.PointIdsList(points=ids[n:n + 512]), wait=True)

    def commit(self):
        return None  # every write already waited for the server

    def _filter(self, flt):
        m = self._m()
        must = []
        if flt.get("shelves") is not None:
            must.append(m.FieldCondition(key="shelf", match=m.MatchAny(any=list(flt["shelves"]))))
        project = flt.get("project")
        if project:
            must.append(m.Filter(should=[m.FieldCondition(key="project", match=m.MatchValue(value="")),
                                         m.FieldCondition(key="project", match=m.MatchValue(value=project))]))
        else:
            must.append(m.FieldCondition(key="project", match=m.MatchValue(value="")))
        must_not = []
        if project and flt.get("hide_team"):
            must_not.append(m.Filter(must=[m.FieldCondition(key="shelf", match=m.MatchValue(value="team")),
                                           m.FieldCondition(key="project", match=m.MatchValue(value="")),
                                           m.FieldCondition(key="title", match=m.MatchAny(any=list(flt["hide_team"])))]))
        return m.Filter(must=must, must_not=must_not or None)

    def search(self, dense, sparse, flt):
        m = self._m()
        f = self._filter(flt)
        prefetch = [m.Prefetch(query=[float(x) for x in dense], using="dense", limit=PREFETCH, filter=f)]
        si, sv = sparse
        if len(si):
            prefetch.append(m.Prefetch(query=m.SparseVector(indices=[int(i) for i in si], values=[float(v) for v in sv]),
                                       using="sparse", limit=PREFETCH, filter=f))
        with self._db:
            res = self.client.query_points(self.collection, prefetch=prefetch, query=m.FusionQuery(fusion=m.Fusion.RRF),
                                           limit=FUSED, with_payload=True)
        return [p.payload for p in res.points if p.payload]

    def rankings(self, dense, sparse, flt, keys=(), limit=PREFETCH):
        """As MemStore.rankings: the dense and sparse rankings and the nearest allowed part of each keyword key."""
        m = self._m()
        f = self._filter(flt)
        dense = [float(x) for x in dense]
        with self._db:
            res = self.client.query_points(self.collection, query=dense, using="dense", query_filter=f, limit=limit,
                                           with_payload=True)
        dense_top = [p.payload for p in res.points if p.payload]
        sparse_top = []
        si, sv = sparse
        if len(si):
            sq = m.SparseVector(indices=[int(i) for i in si], values=[float(v) for v in sv])
            with self._db:
                res = self.client.query_points(self.collection, query=sq, using="sparse", query_filter=f, limit=limit,
                                               with_payload=True)
            sparse_top = [p.payload for p in res.points if p.payload]
        kw = []
        titles = sorted({k[1] for k in keys})
        if titles:
            kf = m.Filter(must=[f, m.FieldCondition(key="title", match=m.MatchAny(any=titles))])
            with self._db:
                pts, _ = self.client.scroll(self.collection, scroll_filter=kf, limit=4096, with_payload=True,
                                            with_vectors=["dense"])
            qn = math.sqrt(sum(x * x for x in dense)) or 1.0
            best = {}  # a note key or a section key -> (similarity, payload)
            for p in pts:
                pay, vec = p.payload or {}, p.vector
                vec = vec.get("dense") if isinstance(vec, dict) else vec
                if not pay or not vec:
                    continue
                vn = math.sqrt(sum(x * x for x in vec)) or 1.0
                sim = sum(a * b for a, b in zip(dense, vec)) / (qn * vn)
                for key in ((pay.get("shelf"), pay.get("title"), None), (pay.get("shelf"), pay.get("title"), pay.get("i"))):
                    if key not in best or sim > best[key][0]:
                        best[key] = (sim, pay)
            seen = set()
            for key in keys:
                hit = best.get(tuple(key))
                if hit and hit[1].get("id") not in seen:
                    seen.add(hit[1].get("id"))
                    kw.append(hit[1])
        return dense_top, sparse_top, kw

    def count_path(self, path):
        m = self._m()
        f = m.Filter(must=[m.FieldCondition(key="path", match=m.MatchValue(value=path))])
        with self._db:
            return self.client.count(self.collection, count_filter=f, exact=True).count


class _Snapshot:
    """What a query reads: built whole by a commit, then swapped in. Never changed after that."""

    def __init__(self, np, dim, rows, shelves):
        self.n = len(rows)
        self.ids = [r[0] for r in rows]
        self.payloads = [r[3] for r in rows]
        self.dense = (np.stack([r[1] for r in rows]).astype(np.float32) if rows
                      else np.zeros((0, dim), dtype=np.float32))
        self.shelf_codes = {s: i for i, s in enumerate(shelves)}
        self.projects = [""] + sorted({p.get("project", "") for p in self.payloads} - {""})
        pcode = {p: i for i, p in enumerate(self.projects)}
        self.shelf = np.array([self.shelf_codes.get(p.get("shelf"), -1) for p in self.payloads], dtype=np.int8)
        self.proj = np.array([pcode[p.get("project", "")] for p in self.payloads], dtype=np.int32)
        self.team = {}  # the kit's team experts: title -> rows
        self.by_note, self.by_section = {}, {}  # (shelf, title) and (shelf, title, i) -> rows, for keyword keys
        for i, p in enumerate(self.payloads):
            if p.get("shelf") == "team" and not p.get("project"):
                self.team.setdefault(p.get("title"), []).append(i)
            self.by_note.setdefault((p.get("shelf"), p.get("title")), []).append(i)
            self.by_section.setdefault((p.get("shelf"), p.get("title"), p.get("i")), []).append(i)
        # sparse vectors term-major: term t's postings are post_rows/post_vals[starts[k]:ends[k]] where terms[k] == t
        if rows:
            lens = np.array([len(r[2][0]) for r in rows], dtype=np.int64)
            all_t = np.concatenate([np.asarray(r[2][0], dtype=np.int64) for r in rows]) if lens.sum() else \
                np.zeros(0, dtype=np.int64)
            all_v = np.concatenate([np.asarray(r[2][1], dtype=np.float32) for r in rows]) if lens.sum() else \
                np.zeros(0, dtype=np.float32)
            all_r = np.repeat(np.arange(self.n, dtype=np.int32), lens)
        else:
            all_t, all_v, all_r = (np.zeros(0, dtype=np.int64), np.zeros(0, dtype=np.float32),
                                   np.zeros(0, dtype=np.int32))
        order = np.argsort(all_t, kind="stable")
        all_t, self.post_rows, self.post_vals = all_t[order], all_r[order], all_v[order]
        self.terms, self.starts, counts = np.unique(all_t, return_index=True, return_counts=True)
        self.ends = self.starts + counts


class MemStore:
    """Light's store: every point in memory as numpy arrays, so a search is a few vector operations (no server, no
    per-point Python). Writes go to a staging table; commit() builds a new snapshot and swaps it in under a short
    lock, so searches never wait for a sync. Saved as <folder>/index.npz and <folder>/payloads.jsonl (each written to
    a temporary name and renamed), tagged with a generation the manifest must repeat."""

    def __init__(self, folder, shelves=None):
        import numpy as np
        import rbxvault
        self.np = np
        self.folder = folder
        self.shelves = tuple(shelves or rbxvault.SHELVES)
        self.dim = 0
        self._gen = None  # the generation saved or loaded last
        self._rows = {}  # id -> (id, dense float32 normalised, (indices int64, values float32), payload)
        self._swap = threading.Lock()
        self._snap = _Snapshot(np, 0, [], self.shelves)

    # ------------------------------------------------------------ writing
    def reset(self, dim):
        self.dim = dim
        self._gen = None
        self._rows = {}
        with self._swap:
            self._snap = _Snapshot(self.np, dim, [], self.shelves)
        os.makedirs(self.folder, exist_ok=True)
        for name in ("index.npz", "payloads.jsonl"):
            try:
                os.remove(os.path.join(self.folder, name))
            except OSError:
                pass
        # what Qdrant's local mode left here before Light moved to this store
        import shutil
        shutil.rmtree(os.path.join(self.folder, "collection"), ignore_errors=True)
        for name in ("meta.json", ".lock"):
            try:
                os.remove(os.path.join(self.folder, name))
            except OSError:
                pass

    def _norm(self, v):
        np = self.np
        v = np.asarray(v, dtype=np.float32)
        n = float(np.linalg.norm(v))
        return v / n if n else v

    def upsert(self, points):
        np = self.np
        for pid, dense, sp, payload in points:
            self._rows[pid] = (pid, self._norm(dense), (np.asarray(sp[0], dtype=np.int64),
                                                        np.asarray(sp[1], dtype=np.float32)), payload)

    def delete(self, ids):
        for pid in ids:
            self._rows.pop(pid, None)

    def commit(self):
        """Swap in a snapshot of everything staged and save it; returns the new generation."""
        snap = _Snapshot(self.np, self.dim, list(self._rows.values()), self.shelves)
        with self._swap:
            self._snap = snap
        gen = uuid.uuid4().hex
        self._save(snap, gen)
        self._gen = gen
        return gen

    def _replace(self, name, write):
        fd, tmp = tempfile.mkstemp(prefix=".tmp-", dir=self.folder)
        try:
            with os.fdopen(fd, "wb") as f:
                write(f)
            os.replace(tmp, os.path.join(self.folder, name))
        except BaseException:
            try:
                os.remove(tmp)
            except OSError:
                pass
            raise

    def _save(self, snap, gen):
        np = self.np
        os.makedirs(self.folder, exist_ok=True)
        rows = [self._rows[i] for i in snap.ids]
        lens = np.array([len(r[2][0]) for r in rows], dtype=np.int64)
        ptr = np.zeros(len(rows) + 1, dtype=np.int64)
        np.cumsum(lens, out=ptr[1:])
        sp_idx = np.concatenate([r[2][0] for r in rows]) if rows else np.zeros(0, dtype=np.int64)
        sp_val = np.concatenate([r[2][1] for r in rows]) if rows else np.zeros(0, dtype=np.float32)

        def arrays(f):
            np.savez(f, dense=snap.dense, sp_ptr=ptr, sp_idx=sp_idx.astype(np.int64),
                     sp_val=sp_val.astype(np.float32), generation=np.array(gen), dim=np.array(self.dim))

        def payloads(f):
            f.write((json.dumps({"generation": gen, "count": len(rows)}) + "\n").encode())
            for r in rows:
                f.write((json.dumps(dict(r[3], id=r[0]), ensure_ascii=False) + "\n").encode())
        self._replace("index.npz", arrays)
        self._replace("payloads.jsonl", payloads)

    # ------------------------------------------------------------ loading
    def open(self, manifest):
        """Load the saved points when their generation is the manifest's; False (nothing loaded) otherwise."""
        np = self.np
        want = (manifest or {}).get("generation")
        if not want:
            return False
        if want == self._gen:  # already loaded (or just saved)
            return True
        try:
            with np.load(os.path.join(self.folder, "index.npz"), allow_pickle=False) as z:
                if str(z["generation"]) != want:
                    return False
                dense, ptr, sp_idx, sp_val = z["dense"], z["sp_ptr"], z["sp_idx"], z["sp_val"]
                dim = int(z["dim"])
            with open(os.path.join(self.folder, "payloads.jsonl"), encoding="utf-8") as f:
                head = json.loads(f.readline())
                if head.get("generation") != want:
                    return False
                payloads = [json.loads(line) for line in f if line.strip()]
            if len(payloads) != head.get("count") or len(payloads) != len(dense) or len(ptr) != len(dense) + 1:
                return False
        except (OSError, ValueError, KeyError):
            return False
        rows = {}
        for k, p in enumerate(payloads):
            pid = p["id"]
            rows[pid] = (pid, dense[k], (sp_idx[ptr[k]:ptr[k + 1]], sp_val[ptr[k]:ptr[k + 1]]), p)
        self.dim = dim
        self._rows = rows
        self._gen = want
        snap = _Snapshot(np, dim, list(rows.values()), self.shelves)
        with self._swap:
            self._snap = snap
        return True

    # ------------------------------------------------------------ reading (lock-free on one snapshot)
    def _mask(self, s, flt):
        np = self.np
        mask = np.ones(s.n, dtype=bool)
        if flt.get("shelves") is not None:
            codes = [s.shelf_codes[x] for x in flt["shelves"] if x in s.shelf_codes]
            mask &= np.isin(s.shelf, np.array(codes, dtype=np.int8))
        project = flt.get("project")
        shared = s.proj == 0
        if project and project in s.projects:
            mask &= shared | (s.proj == s.projects.index(project))
        else:
            mask &= shared
        if project and flt.get("hide_team"):
            for t in flt["hide_team"]:
                rows = s.team.get(t)
                if rows:
                    mask[rows] = False
        return mask

    def _top(self, scores, allowed, k):
        np = self.np
        cand = np.flatnonzero(allowed)
        if not len(cand):
            return []
        sc = scores[cand]
        if len(cand) > k:
            part = np.argpartition(-sc, k - 1)[:k]
            cand, sc = cand[part], sc[part]
        order = np.argsort(-sc, kind="stable")
        return [int(i) for i in cand[order]]

    def search(self, dense, sparse, flt):
        np = self.np
        s = self._snap
        if not s.n:
            return []
        mask = self._mask(s, flt)
        if not mask.any():
            return []
        q = self._norm(dense)
        dense_top = self._top(s.dense @ q, mask, PREFETCH) if len(q) == s.dense.shape[1] else []
        sparse_top = self._sparse_top(s, sparse, mask, PREFETCH)
        fused = {}
        for ranking in (dense_top, sparse_top):
            for pos, row in enumerate(ranking):
                fused[row] = fused.get(row, 0.0) + 1.0 / (pos + RRF_K)
        best = sorted(fused, key=lambda r: -fused[r])[:FUSED]
        return [s.payloads[r] for r in best]

    def _sparse_top(self, s, sparse, mask, limit):
        np = self.np
        acc = np.zeros(s.n, dtype=np.float32)
        touched = np.zeros(s.n, dtype=bool)
        si, sv = sparse
        for t, qv in zip(si, sv):
            k = int(np.searchsorted(s.terms, int(t)))
            if k >= len(s.terms) or s.terms[k] != int(t):
                continue
            a, b = s.starts[k], s.ends[k]
            rows = s.post_rows[a:b]
            df = b - a
            idf = np.log((s.n - df + 0.5) / (df + 0.5) + 1.0)
            np.add.at(acc, rows, np.float32(float(qv) * idf) * s.post_vals[a:b])
            touched[rows] = True
        return self._top(acc, mask & touched, limit) if touched.any() else []

    def rankings(self, dense, sparse, flt, keys=(), limit=PREFETCH):
        """For the engine's own fusion: the dense ranking, the sparse ranking (empty when sparse has no terms) and,
        for each keyword key (shelf, title, section index or None for a whole note) in its order, the allowed part
        of it nearest the query. Three lists of payloads."""
        s = self._snap
        if not s.n:
            return [], [], []
        mask = self._mask(s, flt)
        if not mask.any():
            return [], [], []
        q = self._norm(dense)
        sims = s.dense @ q if len(q) == s.dense.shape[1] else None
        dense_top = self._top(sims, mask, limit) if sims is not None else []
        sparse_top = self._sparse_top(s, sparse, mask, limit) if len(sparse[0]) else []
        kw, seen = [], set()
        for key in keys:
            rows = s.by_note.get(tuple(key[:2])) if key[2] is None else s.by_section.get(tuple(key))
            rows = [r for r in rows or () if mask[r]]
            if rows:
                best = max(rows, key=lambda r: sims[r]) if sims is not None else rows[0]
                if best not in seen:
                    seen.add(best)
                    kw.append(best)
        return [[s.payloads[r] for r in x] for x in (dense_top, sparse_top, kw)]

    def count_path(self, path):
        return sum(1 for p in self._snap.payloads if p.get("path") == path)
