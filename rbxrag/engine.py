"""The knowledge index: a Qdrant collection of chunks with a dense and a sparse vector each, kept in step with the files
incrementally, and queried hybrid (dense + sparse fused with RRF) and then reranked.

The manifest beside the index records, per file, the (mtime_ns, size) it was indexed at and its point ids, so a sync
re-embeds only the files that changed and deletes the points of files that are gone. A changed format, dimension or
model rebuilds the whole collection. The manifest is written (whole, with one rename) only after the points are in.
"""
import json
import os
import tempfile
import threading
import warnings
from typing import Callable, Protocol

from rbxrag import chunks

FORMAT = 1
BATCH = 64
PREFETCH = 40  # candidates from each of the dense and the sparse search
FUSED = 30  # fused candidates the reranker sees


class Embedders(Protocol):
    dim: int
    model: str

    def dense(self, texts: list[str]) -> list[list[float]]: ...
    def dense_query(self, text: str) -> list[float]: ...
    def sparse(self, texts: list[str]) -> list[tuple[list[int], list[float]]]: ...
    def sparse_query(self, text: str) -> tuple[list[int], list[float]]: ...
    def rerank(self, query: str, texts: list[str]) -> list[float]: ...
    def count_tokens(self, text: str) -> int: ...


def _models():
    from qdrant_client import models
    return models


class Engine:
    def __init__(self, client, emb, manifest_path, collection="fridae", corpus=None, project_files=None):
        self.client = client
        self.emb = emb
        self.manifest_path = manifest_path
        self.collection = collection
        self.model = str(getattr(emb, "model", "") or type(emb).__name__)
        self._corpus = corpus or chunks.corpus_files
        self._project_files = project_files or chunks.project_files
        self._db = threading.RLock()  # one client call at a time (the local client is not made for threads)
        self._sync_lock = threading.Lock()  # one sync at a time
        self.ready = False
        self.synced_projects = set()
        self.chunks = 0

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

    # ------------------------------------------------------------ collection
    def _exists(self):
        with self._db:
            return self.client.collection_exists(self.collection)

    def _create(self):
        m = _models()
        with self._db:
            if self.client.collection_exists(self.collection):
                self.client.delete_collection(self.collection)
            self.client.create_collection(
                self.collection,
                vectors_config={"dense": m.VectorParams(size=self.emb.dim, distance=m.Distance.COSINE)},
                sparse_vectors_config={"sparse": m.SparseVectorParams(modifier=m.Modifier.IDF)})
            with warnings.catch_warnings():  # local mode says payload indexes have no effect there; a server uses them
                warnings.simplefilter("ignore")
                for key in ("shelf", "project", "path"):
                    self.client.create_payload_index(self.collection, key, m.PayloadSchemaType.KEYWORD)

    # ------------------------------------------------------------ sync
    def sync(self, project=None, progress: Callable[[str, float], None] = None):
        """Bring the index in step with the files: the shared ones, plus the game's when a project is given. Returns
        {"chunks", "files", "embedded", "removed"} (removed counts files)."""
        with self._sync_lock:
            stats = self._sync(project, progress or (lambda step, fraction: None))
        self.ready = True
        if project:
            self.synced_projects.add(project)
        return stats

    def _sync(self, project, progress):
        m = _models()
        man = self._load_manifest()
        if not self._compatible(man) or not self._exists():
            self._create()
            man = {"format": FORMAT, "dim": self.emb.dim, "model": self.model, "files": {}}
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
        todo, new_entries = [], {}
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
            new_entries[path] = {"mtime_ns": st.st_mtime_ns, "size": st.st_size, "ids": [c["id"] for c in cs],
                                 "project": proj}
        # a file is gone when it was indexed for the shared shelves or for this project and is no longer listed
        gone = [p for p, e in old.items() if p not in new_entries and e.get("project", "") in ("", project or "")]
        stale = []
        for path, e in new_entries.items():
            prev = old.get(path)
            if prev is not None and prev is not e:
                stale += sorted(set(prev.get("ids", [])) - set(e["ids"]))
        for path in gone:
            stale += old[path].get("ids", [])
        total = len(todo)
        for n in range(0, total, BATCH):
            batch = todo[n:n + BATCH]
            texts = [c["text"] for c in batch]
            dense = self.emb.dense(texts)
            sparse = self.emb.sparse(texts)
            points = [m.PointStruct(id=c["id"], payload=c,
                                    vector={"dense": list(map(float, dv)),
                                            "sparse": m.SparseVector(indices=list(map(int, sv[0])),
                                                                     values=list(map(float, sv[1])))})
                      for c, dv, sv in zip(batch, dense, sparse)]
            with self._db:
                self.client.upsert(self.collection, points=points, wait=True)
            progress("index", min(1.0, (n + len(batch)) / total))
        if stale:
            with self._db:
                for n in range(0, len(stale), 512):
                    self.client.delete(self.collection, points_selector=m.PointIdsList(points=stale[n:n + 512]),
                                       wait=True)
        # files of other projects stay as they were
        keep = {p: e for p, e in old.items() if p not in new_entries and p not in gone}
        man["files"] = dict(keep, **new_entries)
        self._write_manifest({"format": FORMAT, "dim": self.emb.dim, "model": self.model,
                              "files": {p: {"mtime_ns": e["mtime_ns"], "size": e["size"], "ids": e["ids"],
                                            "project": e.get("project", "")}
                                        for p, e in man["files"].items()}})
        self.chunks = sum(len(e["ids"]) for e in man["files"].values())
        progress("index", 1.0)
        return {"chunks": self.chunks, "files": len(new_entries), "embedded": total, "removed": len(gone)}

    # ------------------------------------------------------------ query
    def _filter(self, shelves, project):
        m = _models()
        import rbxvault
        left_out = set(rbxvault.SHELVES) - set(shelves)
        if not project:
            left_out.discard("game")  # game chunks always carry their project, so project == "" leaves them out
        # a shelf condition only when it removes something: in Qdrant's local mode every condition costs a pass
        must = [m.FieldCondition(key="shelf", match=m.MatchAny(any=list(shelves)))] if left_out else []
        if project:
            must.append(m.Filter(should=[m.FieldCondition(key="project", match=m.MatchValue(value="")),
                                         m.FieldCondition(key="project", match=m.MatchValue(value=project))]))
        else:
            must.append(m.FieldCondition(key="project", match=m.MatchValue(value="")))
        must_not = []
        if project:  # the game's own team files replace the kit's experts of the same id
            ids = [t for t, _ in rbxvault.team_files(os.path.join(project, ".fridae", "team", "experts"))]
            if ids:
                must_not.append(m.Filter(must=[m.FieldCondition(key="shelf", match=m.MatchValue(value="team")),
                                               m.FieldCondition(key="project", match=m.MatchValue(value="")),
                                               m.FieldCondition(key="title", match=m.MatchAny(any=ids))]))
        return m.Filter(must=must, must_not=must_not or None)

    def query(self, q, shelves, k, project):
        """The k best sections for q on the given shelves (a game's chunks only for its own project): dense and
        sparse candidates fused with RRF, then reranked. Each hit is a chunk's payload plus its rerank score; parts
        of the same section count once."""
        m = _models()
        if not shelves or not q.strip():
            return []
        flt = self._filter(shelves, project)
        dv = [float(x) for x in self.emb.dense_query(q)]
        si, sv = self.emb.sparse_query(q)
        prefetch = [m.Prefetch(query=dv, using="dense", limit=PREFETCH, filter=flt)]
        if len(si):
            prefetch.append(m.Prefetch(query=m.SparseVector(indices=list(map(int, si)), values=list(map(float, sv))),
                                       using="sparse", limit=PREFETCH, filter=flt))
        with self._db:
            res = self.client.query_points(self.collection, prefetch=prefetch, query=m.FusionQuery(fusion=m.Fusion.RRF),
                                           limit=FUSED, with_payload=True)
        cands = [p.payload for p in res.points if p.payload]
        if not cands:
            return []
        scores = list(self.emb.rerank(q, [c["text"] for c in cands]))
        order = sorted(range(len(cands)), key=lambda i: -scores[i])  # stable: ties keep the fused order
        out, seen = [], set()
        for i in order:
            key = (cands[i]["path"], cands[i]["i"])
            if key in seen:
                continue
            seen.add(key)
            out.append(dict(cands[i], score=float(scores[i])))
            if len(out) >= k:
                break
        return out
