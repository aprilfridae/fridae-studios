"""The embedders behind the knowledge index.

Light: fastembed (ONNX, CPU) for everything: dense BAAI/bge-small-en-v1.5, sparse Qdrant/bm25, rerank
Xenova/ms-marco-MiniLM-L-6-v2. Heavy: dense mxbai-embed-large through the local Ollama, sparse Qdrant/bm25 and rerank
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

OLLAMA_URL = "http://127.0.0.1:11434"
LIGHT = {"dense": "BAAI/bge-small-en-v1.5", "dim": 384, "sparse": "Qdrant/bm25",
         "rerank": "Xenova/ms-marco-MiniLM-L-6-v2"}
HEAVY = {"dense": "mxbai-embed-large", "dim": 1024, "sparse": "Qdrant/bm25", "rerank": "BAAI/bge-reranker-base"}
MXBAI_QUERY = "Represent this sentence for searching relevant passages: "


def fastembed_models(mode):
    """[(kind, model name)] of the fastembed models a mode needs on disk."""
    if mode == "light":
        return [("dense", LIGHT["dense"]), ("sparse", LIGHT["sparse"]), ("rerank", LIGHT["rerank"])]
    return [("sparse", HEAVY["sparse"]), ("rerank", HEAVY["rerank"])]


def _quiet_fastembed():
    try:
        from loguru import logger
        logger.disable("fastembed")
    except Exception:
        pass


def load_fastembed(kind, name, cache_dir, local_only=True, lazy=False):
    """One fastembed model object. local_only never touches the network: a model that is not on disk raises."""
    _quiet_fastembed()
    if kind == "dense":
        from fastembed import TextEmbedding as cls
    elif kind == "sparse":
        from fastembed import SparseTextEmbedding as cls
    else:
        from fastembed.rerank.cross_encoder import TextCrossEncoder as cls
    return cls(name, cache_dir=cache_dir, local_files_only=local_only, lazy_load=lazy)


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

    def rerank(self, query, texts):
        texts = list(texts)
        if not texts:
            return []
        return [float(s) for s in self._reranker.rerank(query, texts)]


class LightEmbedders(_FastembedParts):
    dim = LIGHT["dim"]

    def __init__(self, cache_dir=None, dense=None, sparse=None, reranker=None):
        self.model = "light:%s+%s" % (LIGHT["dense"], LIGHT["sparse"])
        self._dense = dense or load_fastembed("dense", LIGHT["dense"], cache_dir)
        self._sparse = sparse or load_fastembed("sparse", LIGHT["sparse"], cache_dir)
        self._reranker = reranker or load_fastembed("rerank", LIGHT["rerank"], cache_dir)
        self.count_tokens = make_token_counter(_tokenizer_json(self._dense))

    def dense(self, texts):
        return [_floats(v) for v in self._dense.embed(list(texts))]

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
        body = json.dumps({"model": self.model, "input": inputs, "truncate": False}).encode()
        req = urllib.request.Request(self.url + "/api/embed", data=body, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as r:
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
