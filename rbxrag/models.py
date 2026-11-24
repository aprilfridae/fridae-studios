"""The embedders behind the knowledge index.

Light: fastembed (ONNX, CPU): dense BAAI/bge-small-en-v1.5 only. Its plan (engine.PLANS) fuses the keyword search
instead of a sparse ranking and reads no reranker; Qdrant/bm25 and Xenova/ms-marco-MiniLM-L-6-v2 stay named for a
plan that wants them and load only then. Heavy: dense mxbai-embed-large through the local Ollama, sparse Qdrant/bm25 and rerank
BAAI/bge-reranker-base through fastembed, kept loaded in the RAG process.

The fastembed models are read from <data>/models only (local_files_only): they are downloaded once, by the setup step
the user starts, never while answering a query. Importing this module imports no model library; they load on use.
"""
import json
import math
import os
import urllib.error
import urllib.request

from rbxrag.chunks import make_token_counter
from rbxrag.net import LOOPBACK

OLLAMA_URL = "http://127.0.0.1:11434"
LIGHT = {"dense": "BAAI/bge-small-en-v1.5", "dim": 384, "sparse": "Qdrant/bm25",
         "rerank": "Xenova/ms-marco-MiniLM-L-6-v2",
         # the dense model reads each chunk's first 256 tokens (a chunk holds up to 480): the build is CPU-bound by
         # tokens, and this is about a third faster with less working memory (measured; the integration report)
         "dense_tokens": 256}
DENSE_BATCH = 32  # texts per ONNX run (length-sorted, so little padding); bigger ones only grow the working memory
HEAVY = {"dense": "mxbai-embed-large", "dim": 1024, "sparse": "Qdrant/bm25", "rerank": "BAAI/bge-reranker-base"}
KEEP_ALIVE = "30m"
MXBAI_QUERY = "Represent this sentence for searching relevant passages: "


def fastembed_models(mode):
    """[(kind, model name)] of the fastembed models a mode needs on disk. Light's plan ranks with neither a sparse
    model nor a reranker (engine.PLANS), so it downloads the dense model only."""
    if mode == "light":
        return [("dense", LIGHT["dense"])]
    return [("sparse", HEAVY["sparse"]), ("rerank", HEAVY["rerank"])]


def _quiet_fastembed():
    try:
        from loguru import logger
        logger.disable("fastembed")
    except Exception:
        pass


def onnx_threads():
    """Threads for a model run: the performance cores on a Mac with two kinds (on an M2 Pro, 8 threads embed about
    15% faster than onnxruntime's default and 12 three times slower, as the efficiency cores hold the rest back);
    elsewhere None (onnxruntime's default)."""
    try:
        import subprocess
        out = subprocess.run(["sysctl", "-n", "hw.perflevel0.physicalcpu", "hw.perflevel1.physicalcpu"],
                             capture_output=True, text=True, timeout=5).stdout.split()
        perf, eff = int(out[0]), int(out[1])
        return perf if perf > 0 and eff > 0 else None
    except Exception:
        return None


def load_fastembed(kind, name, cache_dir, local_only=True, lazy=False, threads=None):
    """One fastembed model object. local_only never touches the network: a model that is not on disk raises."""
    _quiet_fastembed()
    if kind == "dense":
        from fastembed import TextEmbedding as cls
    elif kind == "sparse":
        from fastembed import SparseTextEmbedding as cls
    else:
        from fastembed.rerank.cross_encoder import TextCrossEncoder as cls
    return cls(name, cache_dir=cache_dir, local_files_only=local_only, lazy_load=lazy, threads=threads)


def model_present(kind, name, cache_dir):
    """True when the model's files are in cache_dir (no network, no ONNX session)."""
    try:
        load_fastembed(kind, name, cache_dir, local_only=True, lazy=True)
        return True
    except Exception:
        return False


def _tokenizer_json(model_obj):
    inner = getattr(model_obj, "model", None)
    d = getattr(inner, "_model_dir", None)
    p = os.path.join(str(d), "tokenizer.json") if d else None
    return p if p and os.path.isfile(p) else None


def make_cutter(tokenizer_json):
    """cut(text, n): the text's beginning that holds n tokens of that tokenizer (whole characters, nothing rewritten);
    without a tokenizer, a pessimistic 3 bytes a token."""
    if tokenizer_json:
        from tokenizers import Tokenizer
        tok = Tokenizer.from_file(tokenizer_json)
        tok.no_truncation()

        def cut(text, n):
            enc = tok.encode(text, add_special_tokens=False)
            return text if len(enc.ids) <= n else text[:enc.offsets[n - 1][1]]
        return cut

    def cut(text, n):
        return text if len(text.encode()) <= 3 * n else text.encode()[:3 * n].decode("utf-8", "ignore")
    return cut


def _floats(v):
    return [float(x) for x in v]


def _sparse_pair(e):
    return [int(i) for i in e.indices], [float(x) for x in e.values]


class _FastembedParts:
    """Sparse and rerank through fastembed (shared by both stacks)."""

    def sparse(self, texts):
        return [_sparse_pair(e) for e in self._sparse.embed(list(texts))]

    def sparse_query(self, text):
        return _sparse_pair(next(iter(self._sparse.query_embed(text))))

    def rerank(self, query, texts, max_tokens=None):
        """The reranker's score for each text; with max_tokens each text is first cut to that many of the reranker's
        own tokens (engine.PLANS bounds Heavy's this way)."""
        texts = list(texts)
        if not texts:
            return []
        if max_tokens:
            cut = self._cutter()
            texts = [cut(t, int(max_tokens)) for t in texts]
        return [float(s) for s in self._reranker.rerank(query, texts)]

    def _cutter(self):
        if getattr(self, "_cut", None) is None:
            self._cut = make_cutter(_tokenizer_json(self._reranker))
        return self._cut


class LightEmbedders(_FastembedParts):
    dim = LIGHT["dim"]

    def __init__(self, cache_dir=None, dense=None, sparse=None, reranker=None):
        self._threads = onnx_threads() if dense is None else None
        self._dense_obj = dense or load_fastembed("dense", LIGHT["dense"], cache_dir, threads=self._threads)
        self._sparse_obj = sparse  # Light's plan uses none (engine.PLANS); loaded only if a plan asks for one
        self._cache_dir = cache_dir
        self._reranker_obj = reranker  # Light's plan uses none (engine.PLANS); loaded only if a plan asks for one
        tj = _tokenizer_json(self._dense_obj)
        self.count_tokens = make_token_counter(tj)
        self.dense_tokens = LIGHT["dense_tokens"]
        self._dense_cut = make_cutter(tj)
        # the version names the index format: "light4" is the store of rbxrag.stores.MemStore with dense vectors only
        self.model = "light4:%s/%d" % (LIGHT["dense"], self.dense_tokens or 512)

    @property
    def _dense(self):
        if self._dense_obj is None:  # given back after a big build (release); loaded again on use
            self._dense_obj = load_fastembed("dense", LIGHT["dense"], self._cache_dir, threads=self._threads)
        return self._dense_obj

    @property
    def _sparse(self):
        if self._sparse_obj is None:
            self._sparse_obj = load_fastembed("sparse", LIGHT["sparse"], self._cache_dir)
        return self._sparse_obj

    def release(self):
        """Give back the dense model's working memory (onnxruntime keeps its peak); the next use loads it again."""
        if self._cache_dir is not None:
            self._dense_obj = None
            import gc
            gc.collect()

    @property
    def _reranker(self):
        if self._reranker_obj is None:
            self._reranker_obj = load_fastembed("rerank", LIGHT["rerank"], self._cache_dir)
        return self._reranker_obj

    def dense(self, texts):
        """Each text's vector, from its first dense_tokens tokens; embedded shortest first (less padding), returned in
        the order given."""
        texts = list(texts)
        if self.dense_tokens:
            texts = [self._dense_cut(t, self.dense_tokens) for t in texts]
        order = sorted(range(len(texts)), key=lambda i: len(texts[i]))
        out = [None] * len(texts)
        for i, v in zip(order, self._dense.embed([texts[i] for i in order], batch_size=DENSE_BATCH)):
            out[i] = _floats(v)
        return out

    def dense_query(self, text):
        return _floats(next(iter(self._dense.query_embed(text))))


class OllamaError(RuntimeError):
    pass


def _is_context_error(msg):
    m = (msg or "").lower()
    return "context" in m and ("length" in m or "exceed" in m or "window" in m) or "too long" in m


class OllamaDense:
    """Dense embeddings from the local Ollama (POST /api/embed with truncate false). An input the model's context
    cannot hold is halved and both halves embedded and averaged, never silently truncated."""

    def __init__(self, url=OLLAMA_URL, model=HEAVY["dense"], timeout=120):
        self.url = url.rstrip("/")
        self.model = model
        self.timeout = timeout

    def _post(self, inputs):
        # keep_alive: Ollama unloads an idle model after 5 minutes, and reloading it would cost a search seconds
        body = json.dumps({"model": self.model, "input": inputs, "truncate": False, "keep_alive": KEEP_ALIVE}).encode()
        req = urllib.request.Request(self.url + "/api/embed", data=body, headers={"Content-Type": "application/json"})
        try:
            with LOOPBACK.open(req, timeout=self.timeout) as r:
                data = json.load(r)
        except urllib.error.HTTPError as e:
            with e:
                try:
                    msg = json.load(e).get("error", "")
                except Exception:
                    msg = str(e)
            raise OllamaError(msg or str(e))
        except (urllib.error.URLError, OSError) as e:
            raise OllamaError("Ollama is not answering at %s (%s). Start Ollama and try again." % (self.url, e))
        embs = data.get("embeddings")
        if not isinstance(embs, list) or len(embs) != len(inputs):
            raise OllamaError(data.get("error") or "Ollama gave no embeddings.")
        return [_floats(v) for v in embs]

    def embed(self, texts):
        texts = list(texts)
        if not texts:
            return []
        try:
            return self._post(texts)
        except OllamaError as e:
            if not _is_context_error(str(e)):
                raise
        if len(texts) > 1:  # find the one that is too long
            return [v for t in texts for v in self.embed([t])]
        return [self._halved(texts[0])]

    def _halved(self, text):
        if len(text) < 2:
            raise OllamaError("Ollama refused even a tiny input as too long for its context.")
        mid = len(text) // 2
        cut = text.rfind("\n", 0, mid)
        if cut < len(text) // 4:
            cut = text.rfind(" ", 0, mid)
        if cut < len(text) // 4:
            cut = mid
        a, b = self.embed([text[:cut]])[0], self.embed([text[cut:]])[0]
        v = [(x + y) / 2 for x, y in zip(a, b)]
        n = math.sqrt(sum(x * x for x in v)) or 1.0
        return [x / n for x in v]


class HeavyEmbedders(_FastembedParts):
    dim = HEAVY["dim"]

    def __init__(self, cache_dir=None, ollama_url=OLLAMA_URL, sparse=None, reranker=None, timeout=120):
        self.model = "heavy:ollama/%s+%s" % (HEAVY["dense"], HEAVY["sparse"])
        self._ollama = OllamaDense(ollama_url, HEAVY["dense"], timeout)
        self._cache_dir = cache_dir
        self._sparse_obj, self._reranker_obj = sparse, reranker
        self.count_tokens = make_token_counter(None)
        if cache_dir is not None and reranker is None:  # loaded now so a missing model shows at start, not mid-query
            self._reranker_obj = load_fastembed("rerank", HEAVY["rerank"], cache_dir)
        if cache_dir is not None and sparse is None:
            self._sparse_obj = load_fastembed("sparse", HEAVY["sparse"], cache_dir)
        if self._reranker_obj is not None:
            # chunks are counted with the reranker's tokenizer: the reranker reads every candidate whole
            self.count_tokens = make_token_counter(_tokenizer_json(self._reranker_obj))

    @property
    def _sparse(self):
        if self._sparse_obj is None:
            self._sparse_obj = load_fastembed("sparse", HEAVY["sparse"], self._cache_dir)
        return self._sparse_obj

    @property
    def _reranker(self):
        if self._reranker_obj is None:
            self._reranker_obj = load_fastembed("rerank", HEAVY["rerank"], self._cache_dir)
        return self._reranker_obj

    def dense(self, texts):
        return self._ollama.embed(texts)

    def dense_query(self, text):
        return self._ollama.embed([MXBAI_QUERY + text])[0]
