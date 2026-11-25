"""The thin layer between the bench and the code it measures. Everything that touches rbxvault or rbxrag goes through
here, so when the engine's internals move only this file needs a change.

What it uses, all public:
- rbxvault: KB, TEAM, guide_files, run("search"), load_index, load_sections, SHELVES, RAG_TIMEOUT
- rbxrag.engine.Engine(store, emb, manifest_path) with .sync(...) and .query(q, shelves, k, project)
- rbxrag.stores.MemStore(folder), rbxrag.models.LightEmbedders(models_dir)
- rbxrag.serve.make_server(service, token): the /query protocol rbx_vault's rag_search speaks
"""
import hashlib
import json
import os
import secrets
import sqlite3
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))  # the worktree whose engine is measured
if REPO not in sys.path:
    sys.path.insert(0, REPO)


# ------------------------------------------------------------------ pointing the tools at a bench corpus
def point_at(corpus_dir):
    """Make rbxvault (and so the engine's corpus) read the bench corpus: vault notes from <corpus>/vault, guides
    from <corpus>/guides (listed in guides.json), team files from <corpus>/team. Work and home folders are temporary,
    so no cache and no user notes from this machine are read."""
    os.environ["RBXKIT_PROJECT"] = ""
    os.environ.pop("FRIDAE_RAG_FILE", None)
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    import rbxpaths
    tmp = tempfile.mkdtemp(prefix="scale-bench-")
    rbxpaths.WORK = os.path.join(tmp, "work")
    rbxpaths.HOME_DIR = os.path.join(tmp, "home")
    os.makedirs(rbxpaths.WORK)
    os.makedirs(rbxpaths.HOME_DIR)
    import rbxvault
    rbxvault.KB = os.path.join(corpus_dir, "vault")
    rbxvault.TEAM = os.path.join(corpus_dir, "team")
    with open(os.path.join(corpus_dir, "guides.json"), encoding="utf-8") as f:
        guides = [(t, os.path.join(corpus_dir, "guides", p)) for t, p in json.load(f)]
    rbxvault.guide_files = lambda: list(guides)
    if hasattr(rbxvault, "_API"):
        rbxvault._API = None
    return tmp


# ------------------------------------------------------------------ keyword search as rbx_vault answers it
def keyword_search(q, shelves, k):
    """The text rbx_vault's search action returns with no knowledge index running."""
    import rbxvault
    os.environ.pop("FRIDAE_RAG_FILE", None)
    if len(shelves) == 1:
        return rbxvault.run("search", {"query": q, "shelf": shelves[0], "k": k})
    if set(shelves) >= {"vault", "guides", "team"}:
        return rbxvault.run("search", {"query": q, "shelf": "all", "k": k})
    return rbxvault.combined_search(q, list(shelves), k)


def keyword_build():
    """Build the keyword indexes from nothing (what the first search after a change pays)."""
    import rbxvault
    rbxvault.load_index()
    rbxvault.load_sections()


def shelves_for(set_shelves):
    """The engine's shelves for a question's shelves (rbx_vault's "all" without a game)."""
    import rbxvault
    if set_shelves == ["all"]:
        return [s for s in rbxvault.SHELVES if s != "game"]
    return list(set_shelves)


# ------------------------------------------------------------------ embedding cache
class EmbedCache:
    """Wraps an Embedders object: dense and sparse vectors of chunk texts are kept in a sqlite file keyed by the
    model and the text, so an engine change that keeps the chunk texts does not pay for embedding again. Queries
    and the reranker are never cached. read=False still writes (a cold, honest timing that fills the cache)."""

    def __init__(self, inner, path, read=True):
        import numpy as np
        self._np = np
        self._inner = inner
        self._read = read
        self._db = sqlite3.connect(path, check_same_thread=False)
        self._db.execute("CREATE TABLE IF NOT EXISTS e (k TEXT PRIMARY KEY, v BLOB)")
        self._lock = threading.Lock()
        self.hits = 0
        self.misses = 0
        self.embed_s = 0.0

    def __getattr__(self, name):  # dim, model, count_tokens, dense_query, sparse_query, rerank, anything new
        return getattr(self._inner, name)

    def _key(self, kind, text):
        model = str(getattr(self._inner, "model", "") or type(self._inner).__name__)
        return hashlib.sha1(("%s|%s|%s" % (model, kind, text)).encode("utf-8", "surrogatepass")).hexdigest()

    def _get(self, keys):
        if not self._read:
            return {}
        out = {}
        with self._lock:
            for n in range(0, len(keys), 500):
                part = keys[n:n + 500]
                rows = self._db.execute("SELECT k, v FROM e WHERE k IN (%s)" % ",".join("?" * len(part)), part)
                out.update(dict(rows.fetchall()))
        return out

    def _put(self, pairs):
        with self._lock:
            self._db.executemany("INSERT OR REPLACE INTO e VALUES (?, ?)", pairs)
            self._db.commit()

    def _cached(self, kind, texts, compute, pack, unpack):
        texts = list(texts)
        keys = [self._key(kind, t) for t in texts]
        have = self._get(keys)
        miss = [i for i, k in enumerate(keys) if k not in have]
        self.hits += len(texts) - len(miss)
        self.misses += len(miss)
        out = [unpack(have[k]) if k in have else None for k in keys]
        if miss:
            t = time.perf_counter()
            fresh = compute([texts[i] for i in miss])
            self.embed_s += time.perf_counter() - t
            self._put([(keys[i], pack(v)) for i, v in zip(miss, fresh)])
            for i, v in zip(miss, fresh):
                out[i] = v
        return out

    def dense(self, texts):
        np = self._np
        return self._cached("d", texts, self._inner.dense, lambda v: np.asarray(v, dtype=np.float32).tobytes(),
                            lambda b: np.frombuffer(b, dtype=np.float32).tolist())

    def sparse(self, texts):
        np = self._np

        def pack(sv):
            idx, val = sv
            return np.asarray([len(idx)], dtype=np.int64).tobytes() + np.asarray(idx, dtype=np.int64).tobytes() + \
                np.asarray(val, dtype=np.float32).tobytes()

        def unpack(b):
            n = int(np.frombuffer(b[:8], dtype=np.int64)[0])
            idx = np.frombuffer(b[8:8 + 8 * n], dtype=np.int64).tolist()
            val = np.frombuffer(b[8 + 8 * n:], dtype=np.float32).tolist()
            return idx, val
        return self._cached("s", texts, self._inner.sparse, pack, unpack)


# ------------------------------------------------------------------ the Light engine
def light_embedders(models_dir):
    from rbxrag.models import LightEmbedders
    return LightEmbedders(models_dir)


def light_engine(data_dir, emb):
    """An Engine over a MemStore in <data>/light, as rbxrag.serve.light_factory makes it (but with our embedders)."""
    from rbxrag.engine import Engine
    from rbxrag.stores import MemStore
    folder = os.path.join(data_dir, "light")
    os.makedirs(folder, exist_ok=True)
    return Engine(MemStore(folder), emb, os.path.join(folder, "manifest.json"))


def engine_sync(engine, **kw):
    return engine.sync(None, **kw)


def engine_query(engine, q, shelves, k):
    return engine.query(q, shelves, k, None)


def quick_refresh(engine):
    """The sync a query runs before searching (serve.Service._refresh): quick, at most 200 chunks inline."""
    return engine.sync(None, quick=True, max_embed=200, blocking=True)


# ------------------------------------------------------------------ rbx_vault's search through the knowledge index
class _Adapter:
    """What serve.make_server needs: query() and status(). It answers with the engine and times each query."""

    def __init__(self, engine, shelves_map=None):
        self.engine = engine
        self.last = None  # (seconds, hits) of the last query

    def status(self):
        return {"mode": "light", "ready": True, "building": False, "progress": {}, "chunks": 0, "error": ""}

    def query(self, q, shelves, k, project):
        t = time.perf_counter()
        hits = engine_query(self.engine, q, shelves, k)
        self.last = (time.perf_counter() - t, hits)
        return hits


class RagEndpoint:
    """A real /query server on 127.0.0.1 (rbxrag.serve.make_server) over the engine, and an endpoint file for
    rbx_vault. The token is random, kept in memory and in a 0600 file in a temporary folder, never printed."""

    def __init__(self, engine, tmp):
        from rbxrag.serve import make_server
        self.adapter = _Adapter(engine)
        self.token = secrets.token_hex(24)
        self.srv = make_server(self.adapter, self.token, 0)
        self.url = "http://127.0.0.1:%d" % self.srv.server_address[1]
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        self.file = os.path.join(tmp, "rag-endpoint.json")
        fd = os.open(self.file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump({"url": self.url, "token": self.token}, f)
        import rbxvault
        if hasattr(rbxvault, "RAG_TIMEOUT"):  # measure Light's own answer; the 3 s fallback is counted separately
            self.app_timeout = rbxvault.RAG_TIMEOUT
            rbxvault.RAG_TIMEOUT = 600.0
        else:
            self.app_timeout = 3.0

    def search(self, q, shelves, k):
        """(rbx_vault's text, engine seconds, engine hits). Shelves as rbx_vault takes them."""
        import rbxvault
        os.environ["FRIDAE_RAG_FILE"] = self.file
        try:
            self.adapter.last = None
            if len(shelves) == 1:
                text = rbxvault.run("search", {"query": q, "shelf": shelves[0], "k": k})
            elif set(shelves) >= {"vault", "guides", "team"}:
                text = rbxvault.run("search", {"query": q, "shelf": "all", "k": k})
            else:
                text = rbxvault.rag_search(q, list(shelves), k)
        finally:
            os.environ.pop("FRIDAE_RAG_FILE", None)
        secs, hits = self.adapter.last or (None, [])
        return text, secs, hits

    def close(self):
        self.srv.shutdown()
        self.srv.server_close()
