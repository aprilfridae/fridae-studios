"""The knowledge index: chunks with a dense and a sparse vector each, kept in step with the files incrementally, and
queried by fusing rankings with RRF (the keyword search's, the dense one, and per mode the sparse one and a
reranker's; PLANS). The points live in a store (rbxrag.stores):
Qdrant for Heavy, plain numpy arrays in memory for Light.

The manifest beside the index records, per file, the (mtime_ns, size) it was indexed at, its point ids and its
[[links]], so a sync re-embeds only the files that changed and deletes the points of files that are gone. A changed
format, dimension or model rebuilds the whole index. The manifest is written (whole, with one rename) only after the
points are in. The link graph (which note links to which) is made from it once per sync, never per query: a query
adds up to two passages from notes its first results link to (Engine._follow_links).

Superseded notes (rbxvault.SUPERSEDED_SCALE) are query-time metadata, not part of the index (FORMAT is unchanged):
each sync reads the front matter of the files that changed (supersedes, superseded_by), a query adds the notes
folders' superseded.json, scales a superseded hit's score down and says which note replaces it ("replaced_by").
"""
import json
import os
import tempfile
import threading
from typing import Callable, Protocol

from rbxrag import chunks
from rbxrag.stores import FUSED, PREFETCH, MemStore, QdrantStore  # noqa: F401 (re-exported)

FORMAT = 2  # 2: every chunk carries its context lines and labels, and the manifest each file's [[links]]
KEYWORD_DEPTH = 20  # how many of the keyword search's results join the fusion

# How a query is answered, per mode. Measured on the bench of 33 queries (12 vault baseline, 6 knowledge-bench tasks,
# 15 vague paraphrases; scratchpad ragfuse*.py, the integration report): the keyword search's own ranking fused with
# the dense ranking by RRF (k=60) beat both the keyword search alone and dense + sparse + rerank. The bm25 sparse
# ranking only repeated the keyword search less well, and a reranker that reorders the fused list lost more at the
# top than it gained. Heavy's bigger reranker helps as one more vote in the fusion, over a bounded number of
# candidates (12) cut to a bounded length (256 tokens) so a query stays well inside rbx_vault's 3 s timeout on a
# laptop CPU (about 1.3 s at the 95th percentile on an M2 Pro).
#   keyword: fuse the keyword ranking (when the engine has one); sparse: fuse the sparse ranking; rrf_k: RRF's
#   constant; rerank_depth: how many fused candidates the reranker reads (0: no reranker); rerank_tokens: each cut to
#   this many tokens (0: the model's own limit); rerank: "vote" (its order is one more RRF ranking) or "reorder"
#   (its order replaces the top of the fused list)
DEFAULT_PLAN = {"keyword": True, "sparse": True, "rrf_k": 2, "rerank_depth": FUSED, "rerank_tokens": 0,
                "rerank": "reorder"}
PLANS = {
    "light": {"keyword": True, "sparse": False, "rrf_k": 60, "rerank_depth": 0, "rerank_tokens": 0, "rerank": "vote"},
    "heavy": {"keyword": True, "sparse": False, "rrf_k": 60, "rerank_depth": 12, "rerank_tokens": 256, "rerank": "vote"},
}
BATCH = 1024  # chunks embedded and upserted together (Light sorts each batch by length: far less padding)
CHECKPOINT_BATCHES = 4  # save the manifest (and Light's new rows) every 4,096 embedded chunks
RELEASE_AFTER = 1000  # after embedding more than this many chunks, the embedders may give back their working memory
LINK_CANDIDATES = 24  # linked notes looked at per query at most
LINK_SIM = 0.9  # a linked passage qualifies when it is at least this close to the query, relative to the best direct
LINK_COVER = 0.5  # ... or holds at least half of the query's words


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
    """rbxvault's keyword ranking for the fusion (rbxvault.keyword_ranking), with the vault index and the section
    entries kept loaded as rbxvault.Docs (their BM25F statistics are worked out once, not on every search). They are
    looked at again at most every `every` seconds, in a background thread: a query never waits for the files to be
    walked (at ten times today's notes that walk alone takes seconds), it uses what was loaded last. The first call
    loads synchronously."""

    def __init__(self, every=10.0, clock=None, background=True):
        import time
        self.every = every
        self.clock = clock or time.monotonic
        self.background = background
        self._lock = threading.Lock()
        self._at = None
        self._refreshing = False
        self._idx = self._kit = None
        self._sig = None
        self._docs = {}  # project -> (signature of its own files, rbxvault.Docs)

    def _vault(self):
        import rbxvault
        if not os.path.isdir(rbxvault.KB):
            return None, {"docs": rbxvault.Docs(), "symptoms": []}
        sig = rbxvault.signature(rbxvault.note_files())
        if self._idx is not None and sig == self._sig:
            return sig, self._idx
        idx = rbxvault.load_index()
        return idx.get("sig", sig), dict(idx, docs=rbxvault.Docs(idx["docs"]))

    def _own_files(self, project):
        import rbxvault
        files = [p for _, p in rbxvault.yours_files()]
        if project:
            files += [p for _, p in rbxvault.team_files(os.path.join(project, ".fridae", "team", "experts"))]
            files += [p for _, p in rbxvault.game_files(project)]
        out = []
        for p in files:
            try:
                st = os.stat(p)
                out.append((p, st.st_mtime_ns, st.st_size))
            except OSError:
                pass
        return tuple(out)

    def _sections(self, project, kit, old=None):
        import rbxvault
        sig = (id(kit), self._own_files(project))
        if old is not None and old[0] == sig:
            return old
        return sig, rbxvault.Docs(rbxvault.section_corpus(project or "", kit))

    def _refresh(self):
        """Load everything again where the files changed (the work of a refresh, off the query path)."""
        import rbxvault
        try:
            sig, idx = self._vault()
            kit = rbxvault.load_sections()
            if self._kit is not None and kit == self._kit:
                kit = self._kit  # the same entries: keep the object, so the sections built on it stay valid
            docs = {}
            for project, old in list(self._docs.items())[-4:]:
                docs[project] = self._sections(project, kit, old)
            if self.background and idx is not self._idx:
                self._warm(idx, docs)  # a new index is made ready before searches use it
            with self._lock:
                self._sig, self._idx, self._kit, self._docs = sig, idx, kit, docs
        finally:
            with self._lock:
                self._refreshing = False
                self._at = self.clock()

    @staticmethod
    def _warm(idx, docs):
        import rbxvault
        try:
            idx["docs"].warm(rbxvault.FIELDS)
            for _, d in docs.values():
                for group in (False, True):
                    d.part(("personal", group), lambda x, g=group: (x["shelf"] in rbxvault.PERSONAL) == g).warm(
                        rbxvault.SECTION_FIELDS)
        except Exception:
            pass  # only a head start: a search works the terms out itself

    def warm(self):
        """Load now (when nothing is loaded yet) and work out the common terms in a background thread: the first
        searches then need not."""
        if self._idx is None:
            self("warm", ["vault", "guides", "team", "yours"], 1, "")
        with self._lock:
            idx, docs = self._idx, dict(self._docs)
        if self.background and idx is not None:
            threading.Thread(target=self._warm, args=(idx, docs), name="keyword-warm", daemon=True).start()

    def __call__(self, q, shelves, depth, project):
        import rbxvault
        project = project or ""
        start = False
        with self._lock:
            first = self._idx is None
            if not first and not self._refreshing and self.clock() - self._at >= self.every:
                self._refreshing = start = True
        if first:
            with self._lock:
                self._refreshing = True
            self._refresh()
        elif start:
            if self.background:
                threading.Thread(target=self._refresh, name="keyword-refresh", daemon=True).start()
            else:
                self._refresh()
        with self._lock:
            idx, kit = self._idx, self._kit
            entry = self._docs.get(project)
        docs = None
        if any(s != "vault" for s in shelves):
            if entry is None:
                entry = self._sections(project, kit)
                with self._lock:
                    if kit is self._kit:
                        self._docs[project] = entry
            docs = entry[1]
        return rbxvault.keyword_ranking(q, shelves, depth, idx, docs, project)


def _entry(e):
    """A file's manifest entry: when it was indexed (stat and content hash), its point ids, its project."""
    out = {"mtime_ns": e["mtime_ns"], "size": e["size"], "ids": e["ids"], "project": e.get("project", "")}
    if e.get("sha1"):
        out["sha1"] = e["sha1"]
    if e.get("links"):
        out["links"] = e["links"]
    return out


def _sha1(path):
    import hashlib
    h = hashlib.sha1()
    try:
        with open(path, "rb") as f:
            for block in iter(lambda: f.read(1 << 20), b""):
                h.update(block)
    except OSError:
        return None
    return h.hexdigest()


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
        self._names = {}  # path -> (shelf, title) of every indexed file the syncs listed
        self._graph = None  # (path -> [link targets], target name -> [(shelf, title, path)]), made by each sync
        self._heads = {}  # path -> ((mtime_ns, size), title, project, supersedes, superseded_by, shelf), by each sync
        self._sup = {}  # project -> (supersession entries, known titles) from the heads (the sidecars are added later)

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
        if not quick and not self.ready and hasattr(self.keyword, "warm"):
            try:  # the keyword side loads before the first search, not during it
                self.keyword.warm()
            except Exception:
                pass
        self.ready = True
        if project:
            self.synced_projects.add(project)
        return stats

    def _commit(self, man):
        """Make the store durable, then write the manifest naming the store's generation (written last)."""
        man["generation"] = self.store.commit()
        self._write_manifest({"format": FORMAT, "dim": self.emb.dim, "model": self.model,
                              "generation": man["generation"],
                              "files": {p: _entry(e) for p, e in man["files"].items()}})

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
        new_entries, work = {}, []  # work: the files to embed again, (path, shelf, title, project, stat, sha1)
        restamped = False
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
            digest = _sha1(path)
            if prev and digest and prev.get("sha1") == digest and prev.get("project", "") == proj:
                # touched but not changed (a sync tool, a checkout, an editor's save): nothing to embed
                new_entries[path] = dict(prev, mtime_ns=st.st_mtime_ns, size=st.st_size)
                restamped = True
                continue
            work.append((path, shelf, title, proj, st, digest))
        listed_now = set(new_entries) | {w[0] for w in work}
        metas = {}  # path -> {"links": [...]} for the files chunked in this sync
        # a file is gone when it was indexed for the shared shelves or for this project and is no longer listed
        gone = [p for p, e in old.items() if p not in listed_now and e.get("project", "") in ("", project or "")]
        keep = {p: e for p, e in old.items() if p not in listed_now and p not in gone}  # other projects' files
        pre = None
        if quick:  # a query's refresh: chunk first, so a big change is refused before anything is embedded
            pre, n = {}, 0
            for path, shelf, title, proj, st, _ in work:
                metas[path] = {}
                pre[path] = chunks.chunks_for_file(shelf, title, path, self.emb.count_tokens, project=proj,
                                                   meta=metas[path])
                n += len(pre[path])
                if max_embed is not None and n > max_embed:
                    raise TooBig("more than %d chunks to embed" % max_embed)
        if not work and not gone and not rebuilt:
            # nothing changed: the store stays as it is (and the manifest too, unless files were only touched)
            if restamped:
                self._write_manifest({"format": FORMAT, "dim": self.emb.dim, "model": self.model,
                                      "generation": man.get("generation"),
                                      "files": {p: _entry(e) for p, e in dict(keep, **new_entries).items()}})
            if self._graph is None or not set(new_entries) <= set(self._names):
                self._link_graph(files, dict(keep, **new_entries))
            self.chunks = sum(len(e["ids"]) for e in old.values())
            return {"chunks": self.chunks, "files": len(new_entries), "embedded": 0, "removed": 0}
        # Each file is chunked when its turn comes (a big build never holds every chunk's text at once) and its
        # chunks go to the models in batches. A file is finished once its last chunk is in; its points that the new
        # version no longer has are deleted then.
        work_paths = {w[0] for w in work}
        units = {w[0]: (len(pre[w[0]]) if pre is not None else max(1, w[4].st_size)) for w in work}
        total_units = sum(units.values()) or 1
        state = {"embedded": 0, "flushed": 0, "batches": 0, "done_units": 0}
        batch, ends, stale, finished = [], [], {}, set()  # ends: (path, chunks queued once its last one was)
        sparse_on = bool(self.plan["sparse"] or getattr(self.store, "needs_sparse", True))

        def finish_ready():
            while ends and ends[0][1] <= state["flushed"]:
                path = ends.pop(0)[0]
                if stale.get(path):  # this file's points that its new version no longer has
                    self.store.delete(stale.pop(path))
                finished.add(path)
                state["done_units"] += units[path]

        def flush(n):
            b = batch[:n]
            del batch[:n]
            texts = [c["text"] for c in b]
            dense = self.emb.dense(texts)
            sparse = self.emb.sparse(texts) if sparse_on else [([], [])] * len(b)
            self.store.upsert([(c["id"], dv, sv, c) for c, dv, sv in zip(b, dense, sparse)])
            state["flushed"] += len(b)
            state["embedded"] += len(b)
            finish_ready()
            state["batches"] += 1
            progress("index", min(1.0, state["done_units"] / total_units))
            if (self.checkpoint_batches and state["batches"] % self.checkpoint_batches == 0
                    and len(finished) < len(work_paths)):
                self._checkpoint(man, keep, old, new_entries, gone, work_paths, finished)

        queued = 0
        for path, shelf, title, proj, st, digest in work:
            meta = metas.setdefault(path, {})
            cs = pre.pop(path) if pre is not None else chunks.chunks_for_file(
                shelf, title, path, self.emb.count_tokens, project=proj, meta=meta)
            ids = [c["id"] for c in cs]
            new_entries[path] = {"mtime_ns": st.st_mtime_ns, "size": st.st_size, "ids": ids, "project": proj,
                                 "sha1": digest}
            if meta.get("links"):
                new_entries[path]["links"] = meta["links"]
            if path in old:
                stale[path] = sorted(set(old[path].get("ids", [])) - set(ids))
            batch += cs
            queued += len(cs)
            ends.append((path, queued))
            while len(batch) >= self.batch:
                flush(self.batch)
        if batch:
            flush(len(batch))
        finish_ready()
        rest = [i for ids in stale.values() for i in ids] + [i for p in gone for i in old[p].get("ids", [])]
        if rest:
            self.store.delete(rest)
        man["files"] = dict(keep, **new_entries)
        self._commit(man)
        self._link_graph(files, man["files"])
        if state["embedded"] > RELEASE_AFTER and hasattr(self.emb, "release"):
            self.emb.release()  # a big build's model working memory is not kept for the queries
        self.chunks = sum(len(e["ids"]) for e in man["files"].values())
        progress("index", 1.0)
        return {"chunks": self.chunks, "files": len(new_entries), "embedded": state["embedded"], "removed": len(gone)}

    def _link_graph(self, files, entries):
        """Make the link graph from the manifest's entries (every indexed file's [[links]]) and the files' shelves and
        titles: once per sync, so a query only looks it up."""
        names = dict(self._names)
        names.update({p: v for p, v in files.items()})
        names = {p: v for p, v in names.items() if p in entries}
        links = {p: e["links"] for p, e in entries.items() if e.get("links")}
        targets = {}
        for path, (shelf, title) in names.items():
            base = os.path.splitext(os.path.basename(path))[0]
            for name in {title.casefold(), title.rsplit("/", 1)[-1].casefold(), base.casefold()}:
                targets.setdefault(name, []).append((shelf, title, path))
        self._names = names
        self._graph = (links, targets)
        self._supersession(names, entries)

    def _supersession(self, names, entries):
        """Each indexed file's supersedes / superseded_by, read from its front matter only when it changed."""
        import rbxvault
        heads = {}
        for path, (shelf, title) in names.items():
            e = entries.get(path) or {}
            sig = (e.get("mtime_ns"), e.get("size"))
            old = self._heads.get(path)
            if old and old[0] == sig and old[1] == title and old[5] == shelf and sig[0] not in (None, -1):
                heads[path] = old
                continue
            try:
                with open(path, encoding="utf-8", errors="replace") as f:
                    head = f.read(8192)
            except OSError:
                head = ""
            olds, by = rbxvault.supersession(head)
            heads[path] = (sig, title, e.get("project", ""), olds, by, shelf)
        self._heads = heads
        self._sup = {}

    def _superseded(self, project):
        """{(shelf, old title casefolded): (new title, its shelf)} for a query (rbxvault.supersession_map): the indexed
        files' front matter (the shared ones and this project's) and the notes folders' superseded.json."""
        import rbxnotes
        import rbxvault
        key = project or ""
        cache, heads = self._sup, self._heads  # one snapshot: a sync on another thread replaces both, never changes them
        base = cache.get(key)
        if base is None:
            mine = [h for h in heads.values() if h[2] in ("", key)]
            base = cache[key] = ([(shelf, t, olds, by) for _, t, _, olds, by, shelf in mine if olds or by],
                                 rbxvault.known_titles((shelf, t) for _, t, _, _, _, shelf in mine))
        return rbxvault.supersession_map(base[0], rbxnotes.sidecar_marks(project or None), base[1])

    def _checkpoint(self, man, keep, old, new_entries, gone, work_paths, finished):
        """Save what is fully in: unchanged and finished files as they are, unfinished ones marked to be redone (the
        old ids and any new ones kept, so their points already in are cleaned up next time), gone files still
        listed."""
        files = dict(keep)
        files.update({p: old[p] for p in gone})
        for p, e in new_entries.items():
            if p in work_paths and p not in finished:
                ids = sorted(set(old.get(p, {}).get("ids", [])) | set(e["ids"]))
                files[p] = {"mtime_ns": -1, "size": -1, "ids": ids, "project": e.get("project", "")}
            else:
                files[p] = e
        for p in work_paths:
            if p not in new_entries and p in old:  # not reached yet: its old points are still in
                files[p] = dict(old[p], mtime_ns=-1, size=-1)
        self._commit(dict(man, files=files))

    # ------------------------------------------------------------ query
    def _filter(self, shelves, project, filters=None):
        import rbxvault
        left_out = set(rbxvault.SHELVES) - set(shelves)
        if not project:
            left_out.discard("game")  # game chunks always carry their project, so project == "" leaves them out
        hide = []
        if project:  # the game's own team files replace the kit's experts of the same id
            hide = [t for t, _ in rbxvault.team_files(os.path.join(project, ".fridae", "team", "experts"))]
        # a shelf condition only when it removes something (in Qdrant's local mode every condition costs a pass)
        flt = {"shelves": list(shelves) if left_out else None, "project": project or None, "hide_team": hide}
        for key in ("game", "kind", "source", "since"):
            if (filters or {}).get(key):
                flt[key] = filters[key]
        return flt

    def _follow_links(self, q, dv, flt, out, k, shelves):
        """Up to two passages from notes the first results link to ([[links]]), each the linked note's part nearest
        the query, when it is about as close to the query as the best direct result (LINK_SIM) or holds half of the
        query's words (LINK_COVER). They are marked linked_from (the linking note's title) and placed after the first
        three direct results, never in place of one."""
        import rbxvault
        graph = self._graph
        if graph is None or k <= rbxvault.DIRECT_KEPT or not out or not hasattr(self.store, "best_parts"):
            return out
        links, targets = graph
        have = {(h["shelf"], h["title"]) for h in out}
        keys, src = [], {}
        for h in out[:rbxvault.DIRECT_KEPT]:
            for t in links.get(h["path"], ()):
                for shelf, title, _ in targets.get(t, ()):
                    key = (shelf, title, None)
                    if shelf in shelves and (shelf, title) not in have and key not in src:
                        src[key] = h["title"]
                        keys.append(key)
            if len(keys) >= LINK_CANDIDATES:
                break
        keys = keys[:LINK_CANDIDATES]
        if not keys:
            return out
        direct = [(h["shelf"], h["title"], h["i"]) for h in out[:rbxvault.DIRECT_KEPT]]
        parts = self.store.best_parts(dv, flt, direct + keys)
        sims = [parts[d][0] for d in direct if d in parts]
        bar = LINK_SIM * max(sims) if sims and max(sims) > 0 else None
        want = set(rbxvault.tokens(q))
        found = []
        for key in keys:
            if key not in parts:
                continue
            sim, pay = parts[key]
            words = set(rbxvault.tokens("\n".join(rbxvault.passage(pay.get("text") or "", 4000))))
            cover = len(want & words) / len(want) if want else 0.0
            if cover >= LINK_COVER or (bar is not None and sim >= bar):
                found.append((sim, cover, pay, src[key]))
        found.sort(key=lambda f: (-f[0], -f[1]))
        linked = [dict(pay, score=float(sim), linked_from=frm) for sim, _, pay, frm in found[:rbxvault.LINKED_MAX]]
        return rbxvault.place_linked(out, linked, k) if linked else out

    def query(self, q, shelves, k, project, filters=None):
        """The k best sections for q on the given shelves (a game's chunks only for its own project), as the plan
        says: the keyword search's ranking, the dense ranking and (when on) the sparse ranking fused with RRF, then
        (when on) the reranker over the top candidates, as one more vote or as their new order. Each hit is a chunk's
        payload plus its score; parts of the same section count once. filters (rbxvault.normalize_filters) keep only
        the chunks whose labels pass; up to two hits from linked notes may follow the first three
        (_follow_links)."""
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
        flt = self._filter(shelves, project, filters)
        dense, sparse, kw = self.store.rankings(dv, (list(si), list(sv)), flt, keys)
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
        sup = self._superseded(project)
        for c, sc in zip(fused, scores):
            key = (c["path"], c["i"])
            if key in seen:
                continue
            seen.add(key)
            out.append(dict(c, score=float(sc)))
            if len(out) >= k and not sup:
                break
        if sup:  # a superseded note ranks well below (never dropped): its score scaled, the replacement wins a tie
            import rbxvault
            out = [dict(c, score=float(s)) for s, c in rbxvault.demote(
                out, lambda c: c["score"], lambda c: rbxvault.replaced_by(sup, c), lambda c: c["title"])][:k]
        try:
            out = self._follow_links(q, dv, flt, out, k, shelves)
        except Exception:  # link following only adds: a failure leaves the direct results
            pass
        if sup:
            import rbxvault
            for c in out:
                by = rbxvault.replaced_by(sup, c)
                if by:
                    c["replaced_by"] = by
        return out
