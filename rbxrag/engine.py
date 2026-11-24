"""The knowledge index: chunks with a dense and a sparse vector each, kept in step with the files incrementally, and
queried by fusing rankings with RRF (the keyword search's, the dense one, and per mode the sparse one and a
reranker's; PLANS). The points live in a store (rbxrag.stores):
Qdrant for Heavy, plain numpy arrays in memory for Light.

The manifest beside the index records, per file, the (mtime_ns, size) it was indexed at and its point ids, so a sync
re-embeds only the files that changed and deletes the points of files that are gone. A changed format, dimension or
model rebuilds the whole index. The manifest is written (whole, with one rename) only after the points are in.
"""
import json
import os
import tempfile
import threading
from typing import Callable, Protocol

from rbxrag import chunks
from rbxrag.stores import FUSED, PREFETCH, MemStore, QdrantStore  # noqa: F401 (re-exported)

FORMAT = 1
KEYWORD_DEPTH = 20  # how many of the keyword search's results join the fusion

# How a query is answered, per mode. Measured on the bench of 33 queries (12 vault baseline, 6 knowledge-bench tasks,
# 15 vague paraphrases; scratchpad ragfuse*.py, the integration report): the keyword search's own ranking fused with
# the dense ranking by RRF (k=60) beat both the keyword search alone and dense + sparse + rerank. The bm25 sparse
# ranking only repeated the keyword search less well, and a reranker that reorders the fused list lost more at the
# top than it gained. Heavy's bigger reranker helps as one more vote in the fusion, over a bounded number of
# candidates cut to a bounded length so a query stays under about 1.5 s on a laptop CPU.
#   keyword: fuse the keyword ranking (when the engine has one); sparse: fuse the sparse ranking; rrf_k: RRF's
#   constant; rerank_depth: how many fused candidates the reranker reads (0: no reranker); rerank_tokens: each cut to
#   this many tokens (0: the model's own limit); rerank: "vote" (its order is one more RRF ranking) or "reorder"
#   (its order replaces the top of the fused list)
DEFAULT_PLAN = {"keyword": True, "sparse": True, "rrf_k": 2, "rerank_depth": FUSED, "rerank_tokens": 0,
                "rerank": "reorder"}
PLANS = {
    "light": {"keyword": True, "sparse": False, "rrf_k": 60, "rerank_depth": 0, "rerank_tokens": 0, "rerank": "vote"},
    "heavy": {"keyword": True, "sparse": False, "rrf_k": 60, "rerank_depth": 15, "rerank_tokens": 192, "rerank": "vote"},
}
BATCH = 256  # chunks embedded and upserted together (Light sorts each batch by length: far less padding)
CHECKPOINT_BATCHES = 8  # save the manifest (and Light's arrays) every 2,048 embedded chunks
RELEASE_AFTER = 1000  # after embedding more than this many chunks, the embedders may give back their working memory


class Embedders(Protocol):
    dim: int
    model: str

    def dense(self, texts: list[str]) -> list[list[float]]: ...
    def dense_query(self, text: str) -> list[float]: ...
    def sparse(self, texts: list[str]) -> list[tuple[list[int], list[float]]]: ...
    def sparse_query(self, text: str) -> tuple[list[int], list[float]]: ...
    def rerank(self, query: str, texts: list[str], max_tokens: int | None = None) -> list[float]: ...
    def count_tokens(self, text: str) -> int: ...


class KeywordRanker:
    """rbxvault's keyword ranking for the fusion (rbxvault.keyword_ranking), with the vault index and the kit's
    section docs kept loaded and looked at again at most every `every` seconds (as a query's refresh looks at the
    files)."""

    def __init__(self, every=10.0, clock=None):
        import time
        self.every = every
        self.clock = clock or time.monotonic
        self._lock = threading.Lock()
        self._at = None
        self._idx = self._kit = None

    def __call__(self, q, shelves, depth, project):
        import rbxvault
        with self._lock:
            now = self.clock()
            if self._at is None or now - self._at >= self.every:
                self._idx = rbxvault.load_index() if os.path.isdir(rbxvault.KB) else {"docs": [], "symptoms": []}
                self._kit = rbxvault.load_sections()
                self._at = now
            idx, kit = self._idx, self._kit
        docs = rbxvault.section_corpus(project or "", kit) if any(s != "vault" for s in shelves) else None
        return rbxvault.keyword_ranking(q, shelves, depth, idx, docs)


def _rrf(rankings, k0):
    """Payloads fused by reciprocal rank (a hit at 0-based position p of a ranking adds 1 / (p + k0)), best first, with
    their fused scores. Points are told apart by their id."""
    score, first = {}, {}
    for ranking in rankings:
        for pos, c in enumerate(ranking):
            key = c.get("id") or (c.get("path"), c.get("i"), c.get("part"))
            score[key] = score.get(key, 0.0) + 1.0 / (pos + k0)
            first.setdefault(key, c)
    order = sorted(score, key=lambda key: -score[key])
    return [first[key] for key in order], [score[key] for key in order]


class NeedsRebuild(Exception):
    """A quick sync found the index unusable (it never rebuilds itself)."""


class TooBig(Exception):
    """A quick sync found more to embed than a query should wait for."""


def _is_store(x):
    return all(hasattr(x, a) for a in ("reset", "open", "commit", "rankings", "count_path"))


class Engine:
    def __init__(self, store, emb, manifest_path, collection="fridae", corpus=None, project_files=None, keyword=None,
                 plan=None):
        """store: a MemStore or QdrantStore; a QdrantClient is wrapped in a QdrantStore. keyword: a callable
        (q, shelves, depth, project) -> [(shelf, title, section index or None)] (KeywordRanker), fused when the plan
        says so. plan: how a query is answered (PLANS; DEFAULT_PLAN when None)."""
        self.keyword = keyword
        self.plan = dict(DEFAULT_PLAN, **(plan or {}))
        self.store = store if _is_store(store) else QdrantStore(store, collection)
        self.emb = emb
        self.manifest_path = manifest_path
        self.model = str(getattr(emb, "model", "") or type(emb).__name__)
        self._corpus = corpus or chunks.corpus_files
        self._project_files = project_files or chunks.project_files
        self._sync_lock = threading.Lock()  # one sync at a time
        self.ready = False
        self.synced_projects = set()
        self.chunks = 0
        self.batch = BATCH
        self.checkpoint_batches = CHECKPOINT_BATCHES  # a killed build resumes from the last checkpoint

    # ------------------------------------------------------------ manifest
    def _load_manifest(self):
        try:
            with open(self.manifest_path, encoding="utf-8") as f:
                man = json.load(f)
        except (OSError, ValueError):
            return None
        if not isinstance(man, dict) or not isinstance(man.get("files"), dict):
            return None
        return man

    def _write_manifest(self, man):
        d = os.path.dirname(self.manifest_path) or "."
        os.makedirs(d, exist_ok=True)
        fd, tmp = tempfile.mkstemp(prefix=".tmp-", suffix=".json", dir=d)
        try:
            with os.fdopen(fd, "w", encoding="utf-8") as f:
                json.dump(man, f)
            os.replace(tmp, self.manifest_path)
        except BaseException:
            try:
                os.remove(tmp)
            except OSError:
                pass
            raise

    def _compatible(self, man):
        return (man is not None and man.get("format") == FORMAT and man.get("dim") == self.emb.dim
                and man.get("model") == self.model)

    # ------------------------------------------------------------ sync
    def sync(self, project=None, progress: Callable[[str, float], None] = None, full=False, quick=False,
             max_embed=None, blocking=True):
        """Bring the index in step with the files: the shared ones, plus the game's when a project is given. Returns
        {"chunks", "files", "embedded", "removed"} (removed counts files), or None when blocking is False and another
        sync is running. full rebuilds everything (decided inside the sync lock). quick (the refresh a query runs)
        never rebuilds: it raises NeedsRebuild instead, and TooBig before embedding more than max_embed chunks."""
        if not self._sync_lock.acquire(blocking=blocking):
            return None
        try:
            stats = self._sync(project, progress or (lambda step, fraction: None), full, quick, max_embed)
        finally:
            self._sync_lock.release()
        self.ready = True
        if project:
            self.synced_projects.add(project)
        return stats

    def _commit(self, man):
        """Make the store durable, then write the manifest naming the store's generation (written last)."""
        man["generation"] = self.store.commit()
        self._write_manifest({"format": FORMAT, "dim": self.emb.dim, "model": self.model,
                              "generation": man["generation"],
                              "files": {p: {"mtime_ns": e["mtime_ns"], "size": e["size"], "ids": e["ids"],
                                            "project": e.get("project", "")}
                                        for p, e in man["files"].items()}})

    def _invalidate(self):
        """Before the store is emptied: a manifest naming no files, so a build stopped half way is never read as a
        complete index (it is resumed instead, from the checkpoints)."""
        man = {"format": FORMAT, "dim": self.emb.dim, "model": self.model, "generation": None, "files": {}}
        self._write_manifest(man)
        return man

    def _sync(self, project, progress, full=False, quick=False, max_embed=None):
        man = self._load_manifest()
        rebuilt = full or not (self._compatible(man) and self.store.open(man))
        if rebuilt and quick:
            raise NeedsRebuild("the index does not match its manifest")
        if rebuilt:
            man = self._invalidate()
            self.store.reset(self.emb.dim)
            self.synced_projects = set()
        old = man["files"]
        mine = self._project_files(project) if project else set()
        # the shared files are always the whole shared set (a game's team override hides a kit expert only in that
        # game's queries, never in the index), plus the game's own files
        listed = list(self._corpus(None))
        if project:
            listed += [f for f in self._corpus(project) if f[2] in mine]
        files = {}
        for shelf, title, path in listed:
            files.setdefault(path, (shelf, title))
        progress("scan", 0.0)
        todo, new_entries, changed = [], {}, []  # changed: [(path, index in todo after its last chunk)]
        for path, (shelf, title) in files.items():
            try:
                st = os.stat(path)
            except OSError:
                continue
            proj = project if path in mine else ""
            prev = old.get(path)
            if (prev and prev.get("mtime_ns") == st.st_mtime_ns and prev.get("size") == st.st_size
                    and prev.get("project", "") == proj):
                new_entries[path] = prev
                continue
            cs = chunks.chunks_for_file(shelf, title, path, self.emb.count_tokens, project=proj)
            todo += cs
            changed.append((path, len(todo)))
            new_entries[path] = {"mtime_ns": st.st_mtime_ns, "size": st.st_size, "ids": [c["id"] for c in cs],
                                 "project": proj}
        # a file is gone when it was indexed for the shared shelves or for this project and is no longer listed
        gone = [p for p, e in old.items() if p not in new_entries and e.get("project", "") in ("", project or "")]
        stale = {p: sorted(set(old[p].get("ids", [])) - set(new_entries[p]["ids"])) for p, _ in changed if p in old}
        if quick and max_embed is not None and len(todo) > max_embed:
            raise TooBig("%d chunks to embed" % len(todo))
        if not todo and not gone and not any(stale.values()) and not rebuilt:
            # nothing changed: the manifest and the store stay as they are
            self.chunks = sum(len(e["ids"]) for e in old.values())
            return {"chunks": self.chunks, "files": len(new_entries), "embedded": 0, "removed": 0}
        keep = {p: e for p, e in old.items() if p not in new_entries and p not in gone}  # other projects' files
        total = len(todo)
        complete = 0  # changed[:complete] are fully upserted
        batches = 0
        for n in range(0, total, self.batch):
            batch = todo[n:n + self.batch]
            texts = [c["text"] for c in batch]
            dense = self.emb.dense(texts)
            sparse = self.emb.sparse(texts)
            self.store.upsert([(c["id"], dv, sv, c) for c, dv, sv in zip(batch, dense, sparse)])
            done = n + len(batch)
            while complete < len(changed) and changed[complete][1] <= done:
                path = changed[complete][0]
                if stale.get(path):  # this file's points that its new version no longer has
                    self.store.delete(stale.pop(path))
                complete += 1
            batches += 1
            progress("index", min(1.0, done / total))
            if self.checkpoint_batches and batches % self.checkpoint_batches == 0 and done < total:
                self._checkpoint(man, keep, old, new_entries, gone, changed, complete)
        rest = [i for ids in stale.values() for i in ids] + [i for p in gone for i in old[p].get("ids", [])]
        if rest:
            self.store.delete(rest)
        man["files"] = dict(keep, **new_entries)
        self._commit(man)
        if total > RELEASE_AFTER and hasattr(self.emb, "release"):
            self.emb.release()  # a big build's model working memory is not kept for the queries
        self.chunks = sum(len(e["ids"]) for e in man["files"].values())
        progress("index", 1.0)
        return {"chunks": self.chunks, "files": len(new_entries), "embedded": total, "removed": len(gone)}

    def _checkpoint(self, man, keep, old, new_entries, gone, changed, complete):
        """Save what is fully in: unchanged and finished files as they are, unfinished ones marked to be redone (their
        ids kept, so any of their points already in are cleaned up next time), gone files still listed."""
        files = dict(keep)
        files.update({p: old[p] for p in gone})
        pending = {p for p, _ in changed[complete:]}
        for p, e in new_entries.items():
            if p in pending:
                ids = sorted(set(old.get(p, {}).get("ids", [])) | set(e["ids"]))
                files[p] = {"mtime_ns": -1, "size": -1, "ids": ids, "project": e.get("project", "")}
            else:
                files[p] = e
        self._commit(dict(man, files=files))

    # ------------------------------------------------------------ query
    def _filter(self, shelves, project):
        import rbxvault
        left_out = set(rbxvault.SHELVES) - set(shelves)
        if not project:
            left_out.discard("game")  # game chunks always carry their project, so project == "" leaves them out
        hide = []
        if project:  # the game's own team files replace the kit's experts of the same id
            hide = [t for t, _ in rbxvault.team_files(os.path.join(project, ".fridae", "team", "experts"))]
        # a shelf condition only when it removes something (in Qdrant's local mode every condition costs a pass)
        return {"shelves": list(shelves) if left_out else None, "project": project or None, "hide_team": hide}

    def query(self, q, shelves, k, project):
        """The k best sections for q on the given shelves (a game's chunks only for its own project), as the plan
        says: the keyword search's ranking, the dense ranking and (when on) the sparse ranking fused with RRF, then
        (when on) the reranker over the top candidates, as one more vote or as their new order. Each hit is a chunk's
        payload plus its score; parts of the same section count once."""
        if not shelves or not q.strip():
            return []
        plan = self.plan
        dv = [float(x) for x in self.emb.dense_query(q)]
        si, sv = self.emb.sparse_query(q) if plan["sparse"] else ([], [])
        keys = []
        if plan["keyword"] and self.keyword is not None:
            try:
                keys = list(self.keyword(q, list(shelves), KEYWORD_DEPTH, project or ""))
            except Exception:  # the keyword side never fails a query: dense alone answers
                keys = []
        dense, sparse, kw = self.store.rankings(dv, (list(si), list(sv)), self._filter(shelves, project), keys)
        lists = [kw, dense] + ([sparse] if plan["sparse"] else [])
        fused, scores = _rrf(lists, plan["rrf_k"])
        fused, scores = fused[:FUSED], scores[:FUSED]
        if not fused:
            return []
        depth = min(int(plan["rerank_depth"] or 0), len(fused))
        if depth:
            top = fused[:depth]
            texts = [c["text"] for c in top]
            n = int(plan["rerank_tokens"] or 0)
            rs = list(self.emb.rerank(q, texts, max_tokens=n) if n else self.emb.rerank(q, texts))
            order = sorted(range(depth), key=lambda i: -rs[i])  # stable: ties keep the fused order
            if plan["rerank"] == "vote":
                fused, scores = _rrf(lists + [[top[i] for i in order]], plan["rrf_k"])
            else:
                fused = [top[i] for i in order] + fused[depth:]
                scores = [rs[i] for i in order] + scores[depth:]
        out, seen = [], set()
        for c, sc in zip(fused, scores):
            key = (c["path"], c["i"])
            if key in seen:
                continue
            seen.add(key)
            out.append(dict(c, score=float(sc)))
            if len(out) >= k:
                break
        return out
