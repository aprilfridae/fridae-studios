"""Where the knowledge index keeps its points. The Engine talks to one of two stores through the same small interface:

  reset(dim)              empty store for vectors of this size (a full rebuild)
  open(manifest) -> bool  True when the stored points match the manifest (else the Engine rebuilds)
  upsert(points)          points: [(id, dense, (sparse indices, sparse values), payload)]
  delete(ids)
  commit() -> generation  make everything upserted so far durable and visible; the Engine writes the manifest after it
  rankings(dense, sparse, flt, keys) -> (dense top, sparse top, the keyword keys' parts)   for the engine's fusion
  search(dense, sparse, flt) -> [payload]   the 30 best by RRF of the dense top 40 and the sparse top 40
  count_path(path) -> int
  needs_sparse            True when every point must carry a sparse vector (else the engine sends empty ones when
                          its plan does not rank with them)

flt is {"shelves": [..] or None (no shelf condition), "project": str or None, "hide_team": [expert ids]}: shared
points (project "") always match, a game's points only for that project, and the kit's team experts a game overrides
are hidden for that game.

QdrantStore (Heavy): the Qdrant service. MemStore (Light): numpy arrays in memory, the texts on disk, saved as a
base plus one small delta per commit.
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
    needs_sparse = True  # the collection has a sparse vector for every point

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


DTYPE = "float16"  # Light keeps its vectors as 16-bit floats: half the memory, the same ranking in practice
BLOCK = 16384  # rows turned back into 32-bit floats at a time when a query is scored
COMPACT_DEAD = 0.25  # the base is rewritten when this share of the rows are deleted or replaced ones
MAX_DELTAS = 64  # ... or when this many small commits have piled up on it
_DOC_KEYS = ("shelf", "title", "path", "rel", "project")
_ROW_KEYS = set(_DOC_KEYS) | {"id", "heading", "i", "part", "text"}


class _Snapshot:
    """What a query reads: the first n rows of the store's arrays as they were at a commit. The arrays only ever grow
    past n (a replaced or deleted row is never written over, only marked dead in the next snapshot's own copy of
    `alive`), and the per-document lists are replaced, never changed, so a snapshot needs no lock."""

    def __init__(self, np, st, shelves):
        n = st.n
        self.n = n
        self.dense = st.dense[:n]
        self.alive = st.alive[:n].copy()
        self.doc_of = st.doc_of[:n]
        self.i = st.i_col[:n]
        self.part = st.part_col[:n]
        self.toff, self.tlen = st.toff[:n], st.tlen[:n]
        self.ids, self.headings, self.docs = st.ids, st.headings, st.docs  # lists only appended to
        self.extra = dict(st.extra)
        self.doc_slots = dict(st.doc_slots)
        self.by_title = dict(st.by_title)
        self.texts = st.texts_path
        self._fh = None
        self._fh_lock = threading.Lock()
        self.shelf_codes = {s: i for i, s in enumerate(shelves)}
        self.projects = [""] + sorted({d[4] for d in self.docs} - {""})
        pcode = {p: i for i, p in enumerate(self.projects)}
        doc_shelf = np.array([self.shelf_codes.get(d[0], -1) for d in self.docs] or [0], dtype=np.int8)
        doc_proj = np.array([pcode[d[4]] for d in self.docs] or [0], dtype=np.int32)
        self.shelf = doc_shelf[self.doc_of] if n else np.zeros(0, dtype=np.int8)
        self.proj = doc_proj[self.doc_of] if n else np.zeros(0, dtype=np.int32)
        # sparse vectors (only when the engine stores them) term-major: term t's postings are
        # post_rows/post_vals[starts[k]:ends[k]] where terms[k] == t
        live_sp = [(r, v) for r, v in st.sparse.items() if r < n and self.alive[r]]
        if live_sp:
            lens = np.array([len(v[0]) for _, v in live_sp], dtype=np.int64)
            all_t = np.concatenate([np.asarray(v[0], dtype=np.int64) for _, v in live_sp])
            all_v = np.concatenate([np.asarray(v[1], dtype=np.float32) for _, v in live_sp])
            all_r = np.repeat(np.array([r for r, _ in live_sp], dtype=np.int32), lens)
        else:
            all_t, all_v, all_r = (np.zeros(0, dtype=np.int64), np.zeros(0, dtype=np.float32),
                                   np.zeros(0, dtype=np.int32))
        order = np.argsort(all_t, kind="stable")
        all_t, self.post_rows, self.post_vals = all_t[order], all_r[order], all_v[order]
        self.terms, self.starts, counts = np.unique(all_t, return_index=True, return_counts=True)
        self.ends = self.starts + counts
        self.live = int(self.alive.sum())

    def text(self, r):
        if not self.tlen[r]:
            return ""
        with self._fh_lock:
            try:
                if self._fh is None:
                    self._fh = open(self.texts, "rb")
                self._fh.seek(int(self.toff[r]))
                return self._fh.read(int(self.tlen[r])).decode("utf-8", "replace")
            except OSError:
                return ""

    def payload(self, r):
        d = self.docs[int(self.doc_of[r])]
        p = {"id": self.ids[r], "shelf": d[0], "title": d[1], "heading": self.headings[r], "path": d[2], "rel": d[3],
             "i": int(self.i[r]), "part": int(self.part[r]), "project": d[4], "text": self.text(r)}
        if r in self.extra:
            p.update(self.extra[r])
        return p

    def slots_of(self, shelf, title):
        out = []
        for d in self.by_title.get((shelf, title), ()):
            out += self.doc_slots.get(d, ())
        return out

    def __del__(self):
        try:
            if self._fh is not None:
                self._fh.close()
        except Exception:
            pass


class MemStore:
    """Light's store: every vector in memory in one numpy array (16-bit floats), the rest in small columns, and each
    chunk's text on disk, read only for the hits a query returns. A search is a few vector operations (no server, no
    per-point Python). Writes go into rows past the ones queries read; commit() makes them visible by swapping in a
    new snapshot under a short lock, so searches never wait for a sync.

    On disk (the folder): base-<generation>.npy/.jsonl (the vectors and the rest, written whole), texts-<base>.bin
    (the texts, appended to) and delta-<generation>.npy/.jsonl, one per small commit (its new rows, the ids it
    deleted, and the generation it follows). A commit writes only its delta, so a one-note change costs that note;
    when the deltas pile up (MAX_DELTAS, or more delta rows than base rows) or a quarter of the rows are dead, the
    commit writes a new base instead (amortised: a build writes about twice its size). open() follows the chain of
    deltas from a base to the generation the manifest names; files off that chain are removed by the next commit."""

    needs_sparse = False  # the engine stores sparse vectors here only when its plan ranks with them

    def __init__(self, folder, shelves=None, dtype=DTYPE):
        import numpy as np
        import rbxvault
        self.np = np
        self.folder = folder
        self.shelves = tuple(shelves or rbxvault.SHELVES)
        self.dtype = np.dtype(dtype)
        self.dim = 0
        self._gen = None  # the generation saved or loaded last
        self._chain = []  # [base generation, delta generations...] that _gen is
        self._swap = threading.Lock()
        self._empty(0)
        self._snap = _Snapshot(np, self, self.shelves)

    def close(self):
        if self._tf is not None:
            try:
                self._tf.close()
            except OSError:
                pass
            self._tf = None

    def __del__(self):
        try:
            self.close()
        except Exception:
            pass

    # ------------------------------------------------------------ in memory
    def _empty(self, dim, cap=0):
        np = self.np
        self.dim = dim
        self.n = 0
        self.cap = cap
        self.dense = np.zeros((cap, dim), dtype=self.dtype)
        self.alive = np.zeros(cap, dtype=bool)
        self.doc_of = np.zeros(cap, dtype=np.int32)
        self.i_col = np.zeros(cap, dtype=np.int32)
        self.part_col = np.zeros(cap, dtype=np.int32)
        self.toff = np.zeros(cap, dtype=np.int64)
        self.tlen = np.zeros(cap, dtype=np.int32)
        self.ids, self.headings, self.docs = [], [], []
        self.doc_index, self.doc_slots, self.by_title = {}, {}, {}
        self.extra, self.sparse = {}, {}
        self.slot = {}  # id -> its live row
        self.dead = 0
        self.texts_path = None
        if getattr(self, "_tf", None) is not None:
            self.close()
        self._tf = None  # the texts file, open for appending
        self._tpos = 0
        self._new_rows, self._deleted = [], []  # since the last commit
        self._delta_sizes = []  # rows in each delta on the base

    def _grow(self, need):
        if need <= self.cap:
            return
        np = self.np
        cap = max(need, int(self.cap * 1.5) + 1024)
        for name in ("dense", "alive", "doc_of", "i_col", "part_col", "toff", "tlen"):
            old = getattr(self, name)
            new = np.zeros((cap,) + old.shape[1:], dtype=old.dtype)
            new[:self.n] = old[:self.n]
            setattr(self, name, new)
        self.cap = cap

    def _doc(self, key):
        d = self.doc_index.get(key)
        if d is None:
            d = len(self.docs)
            self.docs.append(key)
            self.doc_index[key] = d
            self.by_title[key[:2]] = self.by_title.get(key[:2], ()) + (d,)
        return d

    def _kill(self, pid):
        r = self.slot.pop(pid, None)
        if r is not None:
            self.alive[r] = False
            self.sparse.pop(r, None)
            self.dead += 1

    def _add(self, pid, vec, sp, payload, toff, tlen):
        """One new row (vec already normalised); its text is already at toff in the texts file."""
        self._kill(pid)
        self._grow(self.n + 1)
        r = self.n
        self.dense[r] = vec
        self.alive[r] = True
        key = tuple(str(payload.get(k) or "") for k in _DOC_KEYS)
        d = self._doc(key)
        self.doc_of[r] = d
        self.i_col[r] = int(payload.get("i") or 0)
        self.part_col[r] = int(payload.get("part") or 0)
        self.toff[r], self.tlen[r] = toff, tlen
        self.ids.append(pid)
        self.headings.append(str(payload.get("heading") or ""))
        extra = {k: v for k, v in payload.items() if k not in _ROW_KEYS}
        if extra:
            self.extra[r] = extra
        if sp is not None and len(sp[0]):
            self.sparse[r] = (self.np.asarray(sp[0], dtype=self.np.int64), self.np.asarray(sp[1], dtype=self.np.float32))
        self.doc_slots[d] = self.doc_slots.get(d, ()) + (r,)
        self.slot[pid] = r
        self.n += 1
        return r

    def _norm(self, v):
        np = self.np
        v = np.asarray(v, dtype=np.float32)
        n = float(np.linalg.norm(v))
        return v / n if n else v

    # ------------------------------------------------------------ files
    def _p(self, name):
        return os.path.join(self.folder, name)

    def _write(self, name, write):
        fd, tmp = tempfile.mkstemp(prefix=".tmp-", dir=self.folder)
        try:
            with os.fdopen(fd, "wb") as f:
                write(f)
            os.replace(tmp, self._p(name))
        except BaseException:
            try:
                os.remove(tmp)
            except OSError:
                pass
            raise

    def _open_texts(self, path, truncate=False):
        self.close()
        self.texts_path = path
        self._tf = open(path, "wb" if truncate else "ab")
        self._tpos = self._tf.tell()

    def _put_text(self, text):
        b = (text or "").encode("utf-8")
        off = self._tpos
        self._tf.write(b)
        self._tpos += len(b)
        return off, len(b)

    def _row_line(self, r):
        line = {"id": self.ids[r], "d": int(self.doc_of[r]), "h": self.headings[r], "i": int(self.i_col[r]),
                "p": int(self.part_col[r]), "t": [int(self.toff[r]), int(self.tlen[r])]}
        if r in self.extra:
            line["x"] = self.extra[r]
        if r in self.sparse:
            line["sp"] = [self.sparse[r][0].tolist(), self.sparse[r][1].tolist()]
        return line

    def _save_rows(self, name, head, rows, docs):
        """<name>.npy (the vectors of rows) and then <name>.jsonl (head, the doc table, one line per row)."""
        np = self.np
        dense = self.dense[rows] if len(rows) else np.zeros((0, self.dim), dtype=self.dtype)
        self._write(name + ".npy", lambda f: np.save(f, dense, allow_pickle=False))

        def lines(f):
            f.write((json.dumps(dict(head, count=len(rows), docs=docs), ensure_ascii=False) + "\n").encode())
            for r in rows:
                f.write((json.dumps(self._row_line(r), ensure_ascii=False) + "\n").encode())
        self._write(name + ".jsonl", lines)

    def _files(self, prefix, suffix):
        try:
            return [f[len(prefix):-len(suffix)] for f in os.listdir(self.folder)
                    if f.startswith(prefix) and f.endswith(suffix)]
        except OSError:
            return []

    def _cleanup(self):
        """Remove every file off the current chain (and what older versions of this store left)."""
        keep = set()
        if self._chain:
            keep |= {"base-%s.npy" % self._chain[0], "base-%s.jsonl" % self._chain[0]}
            keep |= {"delta-%s.%s" % (g, ext) for g in self._chain[1:] for ext in ("npy", "jsonl")}
        if self.texts_path:
            keep.add(os.path.basename(self.texts_path))
        try:
            names = os.listdir(self.folder)
        except OSError:
            return
        for f in names:
            if f in keep:
                continue
            if f.startswith(("base-", "delta-", "texts-", ".tmp-")) or f in ("index.npz", "payloads.jsonl", "meta.json",
                                                                         ".lock"):
                try:
                    os.remove(self._p(f))
                except OSError:
                    pass  # Windows keeps a file a query still reads; the next commit tries again
        import shutil
        shutil.rmtree(self._p("collection"), ignore_errors=True)  # what Qdrant's local mode left here once

    def _compact(self, gen):
        """A new base with only the live rows (texts copied to a new texts file); the deltas are dropped."""
        np = self.np
        live = np.flatnonzero(self.alive[:self.n])
        old_texts = self.texts_path
        dense = self.dense[live].copy()
        cols = {c: getattr(self, c)[live].copy() for c in ("doc_of", "i_col", "part_col", "toff", "tlen")}
        ids = [self.ids[r] for r in live]
        heads = [self.headings[r] for r in live]
        extra = {k: self.extra[r] for k, r in enumerate(live) if r in self.extra}
        sparse = {k: self.sparse[r] for k, r in enumerate(live) if r in self.sparse}
        used = sorted({int(d) for d in cols["doc_of"]})
        remap = {d: k for k, d in enumerate(used)}
        docs = [self.docs[d] for d in used]
        # the texts, in row order, into a new file
        if self._tf is not None:
            self._tf.flush()
        newt = self._p("texts-%s.bin" % gen)
        src = open(old_texts, "rb") if old_texts and os.path.exists(old_texts) else None
        try:
            with open(newt, "wb") as out:
                pos = 0
                for k in range(len(live)):
                    n = int(cols["tlen"][k])
                    b = b""
                    if n and src is not None:
                        src.seek(int(cols["toff"][k]))
                        b = src.read(n)
                    out.write(b)
                    cols["toff"][k], cols["tlen"][k] = pos, len(b)
                    pos += len(b)
        finally:
            if src is not None:
                src.close()
        self._empty(self.dim, cap=max(len(live), 1024))
        n = len(live)
        self.dense[:n] = dense
        self.alive[:n] = True
        self.doc_of[:n] = np.array([remap[int(d)] for d in cols["doc_of"]], dtype=np.int32)
        self.i_col[:n], self.part_col[:n] = cols["i_col"], cols["part_col"]
        self.toff[:n], self.tlen[:n] = cols["toff"], cols["tlen"]
        self.ids, self.headings, self.extra, self.sparse = ids, heads, extra, sparse
        self.n = n
        for key in docs:
            self._doc(key)
        ds = {}
        for r, d in enumerate(self.doc_of[:n].tolist()):
            ds.setdefault(d, []).append(r)
        self.doc_slots = {d: tuple(v) for d, v in ds.items()}
        self.slot = {pid: r for r, pid in enumerate(ids)}
        self._open_texts(newt)
        self._save_rows("base-%s" % gen, {"generation": gen, "dim": self.dim, "dtype": str(self.dtype),
                                          "texts": os.path.basename(newt)}, list(range(n)), [list(d) for d in self.docs])
        self._chain = [gen]

    # ------------------------------------------------------------ writing
    def reset(self, dim):
        with self._swap:
            self._snap = _Snapshot(self.np, _Empty(self.np, dim, self.dtype), self.shelves)
        self.close()
        self._empty(dim)
        self._gen = None
        self._chain = []
        os.makedirs(self.folder, exist_ok=True)
        self.texts_path = None
        self._cleanup()  # everything goes
        gen = uuid.uuid4().hex
        self._open_texts(self._p("texts-%s.bin" % gen), truncate=True)
        self._save_rows("base-%s" % gen, {"generation": gen, "dim": dim, "dtype": str(self.dtype),
                                          "texts": os.path.basename(self.texts_path)}, [], [])
        self._chain = [gen]
        self._gen = gen

    def upsert(self, points):
        for pid, dense, sp, payload in points:
            off, n = self._put_text(payload.get("text", ""))
            if pid in self.slot:
                self._deleted.append(pid)
            vec = self._norm(dense)
            if self.dim and len(vec) != self.dim:
                raise ValueError("a %d-number vector in a %d-number index" % (len(vec), self.dim))
            self._new_rows.append(self._add(pid, vec.astype(self.dtype), sp, payload, off, n))

    def delete(self, ids):
        for pid in ids:
            if pid in self.slot:
                self._deleted.append(pid)
                self._kill(pid)

    def commit(self):
        """Make everything upserted and deleted since the last commit durable (a delta, or a new base when it is
        time) and visible to queries; returns the new generation."""
        if self._tf is None:  # a store that was never reset or opened
            self.reset(self.dim)
        self._cleanup()  # files off the chain the manifest now names
        self._tf.flush()
        gen = uuid.uuid4().hex
        base_rows = self.n - self.dead - len(self._new_rows)
        delta_rows = sum(self._delta_sizes) + len(self._new_rows)
        if (len(self._chain) > MAX_DELTAS or self.dead > COMPACT_DEAD * max(self.n, 1)
                or delta_rows > max(base_rows, 4096)):
            self._compact(gen)
        else:
            rows = [r for r in self._new_rows if self.alive[r]]
            docs = sorted({int(self.doc_of[r]) for r in rows})
            # a delta names its rows' docs by their index in the store's doc table, and carries those entries
            self._save_rows("delta-%s" % gen, {"generation": gen, "parent": self._gen, "delete": self._deleted,
                                               "dim": self.dim}, rows, {str(d): list(self.docs[d]) for d in docs})
            self._chain.append(gen)
            self._delta_sizes.append(len(rows))
        self._new_rows, self._deleted = [], []
        snap = _Snapshot(self.np, self, self.shelves)
        with self._swap:
            self._snap = snap
        self._gen = gen
        return gen

    # ------------------------------------------------------------ loading
    def _head(self, name):
        try:
            with open(self._p(name), encoding="utf-8") as f:
                return json.loads(f.readline())
        except (OSError, ValueError):
            return None

    def open(self, manifest):
        """Load the saved points when a base and its deltas lead to the generation the manifest names; False
        (nothing loaded) otherwise."""
        want = (manifest or {}).get("generation")
        if not want:
            return False
        if want == self._gen:  # already loaded (or just saved)
            return True
        children = {}
        for g in self._files("delta-", ".jsonl"):
            h = self._head("delta-%s.jsonl" % g)
            if h and h.get("generation") == g:
                children.setdefault(h.get("parent"), []).append(g)
        for base in self._files("base-", ".jsonl"):
            chain, cur, seen = [base], base, {base}
            while cur != want:
                nxt = [g for g in children.get(cur, []) if g not in seen]
                if not nxt:
                    break
                # with more than one child (a commit whose manifest was never written) the one leading to want wins
                found = None
                for g in nxt:
                    if self._leads_to(g, want, children):
                        found = g
                        break
                if found is None:
                    break
                cur = found
                chain.append(cur)
                seen.add(cur)
            if cur == want:
                try:
                    if self._load(chain):
                        return True
                except (OSError, ValueError, KeyError, IndexError, TypeError):
                    pass
        return False

    def _leads_to(self, g, want, children, depth=0):
        if g == want:
            return True
        if depth > 10000:
            return False
        return any(self._leads_to(c, want, children, depth + 1) for c in children.get(g, []))

    def _apply(self, name, docmap_from_head):
        """Add the rows of <name>.npy/.jsonl one line at a time (the vectors read through a memory map, so a load
        never holds a second copy of them); returns (head, rows added). Raises on a torn or mismatched pair."""
        np = self.np
        dense = np.load(self._p(name + ".npy"), mmap_mode="r", allow_pickle=False)
        try:
            with open(self._p(name + ".jsonl"), encoding="utf-8") as f:
                head = json.loads(f.readline())
                if head.get("count") != len(dense) or (len(dense) and dense.shape[1] != self.dim):
                    raise ValueError("a torn or mismatched file: %s" % name)
                for pid in head.get("delete") or ():
                    self._kill(pid)
                docmap = docmap_from_head(head)
                k = 0
                for line in f:
                    if not line.strip():
                        continue
                    row = json.loads(line)
                    payload = dict(zip(_DOC_KEYS, docmap[row["d"]]), heading=row["h"], i=row["i"], part=row["p"])
                    payload.update(row.get("x") or {})
                    sp = row.get("sp")
                    self._add(row["id"], dense[k], (sp[0], sp[1]) if sp else None, payload, row["t"][0], row["t"][1])
                    k += 1
                if k != len(dense):
                    raise ValueError("a torn file: %s" % name)
        finally:
            del dense
        return head, k

    def _load(self, chain):
        head = self._head("base-%s.jsonl" % chain[0])
        if not head or not os.path.exists(self._p(head.get("texts") or "-")):
            return False
        texts = self._p(head["texts"])
        self.close()
        self.dtype = self.np.dtype(head.get("dtype") or DTYPE)
        rows = int(head.get("count") or 0) + sum(int((self._head("delta-%s.jsonl" % g) or {}).get("count") or 0)
                                                for g in chain[1:])
        self._empty(int(head["dim"]), cap=rows + 1024)
        self._apply("base-%s" % chain[0], lambda h: {k: tuple(d) for k, d in enumerate(h["docs"])})
        sizes = []
        for g in chain[1:]:
            _, n = self._apply("delta-%s" % g, lambda h: {int(k): tuple(v) for k, v in (h.get("docs") or {}).items()})
            sizes.append(n)
        self.dead = self.n - len(self.slot)
        self._delta_sizes = sizes
        self._open_texts(texts)
        self._chain = list(chain)
        self._gen = chain[-1]
        snap = _Snapshot(self.np, self, self.shelves)
        with self._swap:
            self._snap = snap
        return True

    # ------------------------------------------------------------ reading (lock-free on one snapshot)
    def _mask(self, s, flt):
        np = self.np
        mask = s.alive.copy()
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
                for d in s.by_title.get(("team", t), ()):
                    if s.docs[d][4] == "":
                        rows = list(s.doc_slots.get(d, ()))
                        if rows:
                            mask[rows] = False
        return mask

    def _sims(self, s, q):
        np = self.np
        if s.dense.dtype == np.float32:
            return s.dense @ q
        out = np.empty(s.n, dtype=np.float32)
        for a in range(0, s.n, BLOCK):
            out[a:a + BLOCK] = s.dense[a:a + BLOCK].astype(np.float32) @ q
        return out

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
        s = self._snap
        if not s.n:
            return []
        mask = self._mask(s, flt)
        if not mask.any():
            return []
        q = self._norm(dense)
        dense_top = self._top(self._sims(s, q), mask, PREFETCH) if len(q) == s.dense.shape[1] else []
        sparse_top = self._sparse_top(s, sparse, mask, PREFETCH)
        fused = {}
        for ranking in (dense_top, sparse_top):
            for pos, row in enumerate(ranking):
                fused[row] = fused.get(row, 0.0) + 1.0 / (pos + RRF_K)
        best = sorted(fused, key=lambda r: -fused[r])[:FUSED]
        return [s.payload(r) for r in best]

    def _sparse_top(self, s, sparse, mask, limit):
        np = self.np
        if not len(s.terms):
            return []
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
            idf = np.log((s.live - df + 0.5) / (df + 0.5) + 1.0)
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
        sims = self._sims(s, q) if len(q) == s.dense.shape[1] else None
        dense_top = self._top(sims, mask, limit) if sims is not None else []
        sparse_top = self._sparse_top(s, sparse, mask, limit) if len(sparse[0]) else []
        kw, seen = [], set()
        for key in keys:
            rows = [r for r in s.slots_of(key[0], key[1]) if mask[r] and (key[2] is None or s.i[r] == key[2])]
            if rows:
                best = max(rows, key=lambda r: sims[r]) if sims is not None else rows[0]
                if best not in seen:
                    seen.add(best)
                    kw.append(best)
        return [[s.payload(r) for r in x] for x in (dense_top, sparse_top, kw)]

    def count_path(self, path):
        s = self._snap
        return sum(1 for d, key in enumerate(s.docs) if key[2] == path for r in s.doc_slots.get(d, ()) if s.alive[r])


class _Empty:
    """The fields a _Snapshot reads, for an empty store."""

    def __init__(self, np, dim, dtype):
        self.n = 0
        self.dense = np.zeros((0, dim), dtype=dtype)
        self.alive = np.zeros(0, dtype=bool)
        self.doc_of = self.i_col = self.part_col = self.tlen = np.zeros(0, dtype=np.int32)
        self.toff = np.zeros(0, dtype=np.int64)
        self.ids, self.headings, self.docs = [], [], []
        self.extra, self.doc_slots, self.by_title, self.sparse = {}, {}, {}, {}
        self.texts_path = None
