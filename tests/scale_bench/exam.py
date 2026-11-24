#!/usr/bin/env python3
"""The knowledge exam: how good is Fridae's knowledge system, and why. Every search mode (keyword, Light, Heavy) at
1x and 10x the vault, the pieces that make each one (ablations), and the answers the real Librarian writes from what
they find, graded by a strong judge.

  .venv/bin/python tests/scale_bench/exam.py --quick          # a small smoke of every stage (about 6 answers)
  .venv/bin/python tests/scale_bench/exam.py                  # the full exam
  .venv/bin/python tests/scale_bench/exam.py --estimate       # print what each stage would take, run nothing
  options: --stages retrieval,ablations,answers,judge,analysis  --cache-dir DIR  --models DIR  --scales 1,10
           --heavy-scales 1,10  --ablation-scales 1  --answer-limit N  --jobs 4  --report FILE.md

Stages (each saves its results as it goes, keyed by the code version and the corpus, so a stopped run resumes):
- retrieval: every question set (paraphrase, exact, old33 from questions.json; typo, negative, multihop, stale, guide
  and scope from exam_questions.json) against keyword, Light and Heavy at each scale. No model calls. Hit@1/3/5, MRR,
  tokens per search (k 3/5/8, through rbx_vault's own search text), latency, build and open time, memory, disk.
- ablations: the same questions with one piece switched off. At query time: dense only, keyword ranking only, bm25
  sparse only and dense+sparse (Heavy), no reranker (Heavy), no [[link]] following. At index time (Light): chunks
  without their context line, and without the note's gist in it. Filters: the scope set with and without game "this".
- answers: about 60 questions with 2-4 key facts each. (a) The app's Librarian (`fridae --librarian-exam answer`: the
  real split, writer and support check over the user's OpenCode plan) from each method's passages, at DeepSeek
  Flash's default effort and (for one method) its lowest. (b) A strong reader (`claude -p --model sonnet`, no tools)
  from the same passages, citing a passage per claim.
- judge: `claude -p --model opus` (no tools) grades the answers 8 at a time against their keys: each key fact
  present, absent or contradicted, each wrong fact asserted or not, abstentions and guide pointers.
- analysis: the scores, why each failed answer failed, and the report (docs/superpowers/reports/).

Services: Heavy uses the local Ollama (mxbai-embed-large, never pulled or changed here) and a Qdrant this run starts
from the app's installed binary on a free port, its storage in the exam cache. The Librarian starts its own OpenCode
server on the app's OpenCode store and sign-in (as the app does) and stops it after. Nothing else is contacted.
"""
import argparse
import contextlib
import datetime
import json
import os
import platform
import random
import resource
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
import urllib.request
from concurrent.futures import ThreadPoolExecutor

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import shim  # noqa: E402  (puts the repo on sys.path)
import corpus  # noqa: E402
import exam_lib as L  # noqa: E402

REPO = shim.REPO
STAGES = ["retrieval", "ablations", "answers", "judge", "analysis"]
VERSION = {"retrieval": 1, "answers": 1, "judge": 1}  # bump when a stage's way of measuring changes
K_MAX = 8
KS = (3, 5, 8)
APP_TIMEOUT = 3.0
OLLAMA_URL = "http://127.0.0.1:11434"
APP_SUPPORT = os.path.expanduser("~/Library/Application Support/Fridae Studios")
QDRANT_BIN = os.path.join(APP_SUPPORT, "rag", "bin", "qdrant")
FRIDAE_BIN = os.path.join(REPO, "app-rs", "target", "debug", "fridae")
RETRIEVAL_SETS = ["paraphrase", "exact", "old33", "typo", "negative", "multihop", "stale", "guide", "scope"]
QUERY_VARIANTS = {"keyword": ["app", "nolinks"],
                  "light": ["app", "dense", "lexical", "nolinks"],
                  "heavy": ["app", "norerank", "dense", "lexical", "sparse", "dense+sparse", "app+sparse", "nolinks"]}
INDEX_VARIANTS = ["noctx", "nogist"]
READER_MODEL = "sonnet"
JUDGE_MODEL = "opus"
JUDGE_BATCH = 8
CLAUDE_TIMEOUT = {"sonnet": 240, "opus": 420}
# unset for the claude calls, so they run as a plain user session on the user's plan (not inside this session)
CLAUDE_ENV_DROP = ("CLAUDECODE", "CLAUDE_CODE_SESSION_ID", "CLAUDE_CODE_MESSAGING_SOCKET", "CLAUDE_CODE_MESSAGING_TOKEN",
                   "CLAUDE_PID", "CLAUDE_AGENT_SDK_VERSION", "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN",
                   "CLAUDE_CODE_ENTRYPOINT")
# what a stage costs until this machine has measured it (rates.json replaces each as soon as it is measured)
DEFAULT_RATES = {"light_embed_cps": 60.0, "heavy_embed_cps": 15.0, "keyword_query_s": 0.15, "light_query_s": 0.08,
                 "heavy_query_s": 0.9, "librarian_answer_s": 4.0, "reader_answer_s": 4.0, "judge_batch_s": 25.0,
                 "chunks_per_est": 1.15}


def say(*a):
    print(*a, file=sys.stderr, flush=True)


def peak_rss_mb():
    r = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return r / (1024 * 1024) if sys.platform == "darwin" else r / 1024


def rss_mb(pid=None):
    try:
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid or os.getpid())], capture_output=True, text=True).stdout
        return int(out.strip()) / 1024
    except (OSError, ValueError):
        return None


def dir_mb(path):
    total = 0
    for root, _, fs in os.walk(path):
        for f in fs:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total / (1024 * 1024)


# ------------------------------------------------------------------ the questions
def load_sets():
    with open(os.path.join(HERE, "questions.json"), encoding="utf-8") as f:
        base = json.load(f)
    with open(os.path.join(HERE, "exam_questions.json"), encoding="utf-8") as f:
        exam = json.load(f)
    return base, exam


def all_questions(base, exam):
    """{id: question} over every set, each with its set and shelves."""
    out = {}
    for s in ("paraphrase", "exact", "old33"):
        for q in base[s]:
            out[q["id"]] = dict(q, set=s, shelves=q.get("shelves") or ["all"])
    for s in ("typo", "negative", "multihop", "stale", "guide", "scope"):
        for i, q in enumerate(exam[s]):
            q = dict(q, set=s, shelves=q.get("shelves") or ["all"])
            if s == "stale":
                q["gold"] = [q["new"]["title"]]
                q["marked"] = corpus.stale_marked(i)
            q.setdefault("gold", [])
            out[q["id"]] = q
    return out


def retrieval_questions(qs, quick):
    out = [q for q in qs.values() if q["set"] in RETRIEVAL_SETS]
    if quick:  # a few of each set
        keep, seen = [], {}
        for q in out:
            seen[q["set"]] = seen.get(q["set"], 0) + 1
            if seen[q["set"]] <= (6 if q["set"] in ("paraphrase", "exact") else 3):
                keep.append(q)
        out = keep
    return out


def answer_entries(exam, qs, limit=None):
    """The answer set: each entry with its question, gold titles and (stale) the old title."""
    out = []
    for a in exam["answers"]:
        q = qs[a["id"]]
        e = dict(a, question=q["q"], gold=q.get("gold") or [], set=q["set"], scope=q["set"] == "scope")
        if q["set"] == "stale":
            e["stale_old"] = q["old"]
        out.append(e)
    if limit:  # a spread of kinds: answers, a negative, a guide, a stale pair
        picked, kinds = [], {}
        order = sorted(out, key=lambda e: (e["expect"] != "answer", e["set"] != "stale"))
        want = {"not_found": 1, "guide": 1}
        for e in out:
            k = e["expect"]
            if k in want and kinds.get(k, 0) < want[k] and len(picked) < limit:
                picked.append(e)
                kinds[k] = kinds.get(k, 0) + 1
        for e in order:
            if len(picked) >= limit:
                break
            if e not in picked and (e["set"] == "stale" and not any(p["set"] == "stale" for p in picked)):
                picked.append(e)
        for e in out:
            if len(picked) >= limit:
                break
            if e not in picked and e["expect"] == "answer":
                picked.append(e)
        out = sorted(picked, key=lambda e: e["id"])
    return out


# ------------------------------------------------------------------ versions
SEARCH_CODE = ["rbxvault.py", "rbxnotes.py", "rbxpaths.py", "rbxupkeep.py"]
ANSWER_CODE = ["app-rs/src/swarm/librarian.rs", "app-rs/src/compact.rs", "app-rs/src/opencode.rs", "app-rs/src/exam.rs"]


def code_commit():
    """The commit measured: HEAD, plus "+dirty" when tracked files differ from it."""
    try:
        head = subprocess.run(["git", "-C", REPO, "rev-parse", "HEAD"], capture_output=True, text=True,
                              timeout=10).stdout.strip()
        dirty = subprocess.run(["git", "-C", REPO, "status", "--porcelain", "--untracked-files=no"],
                               capture_output=True, text=True, timeout=10).stdout.strip()
        branch = subprocess.run(["git", "-C", REPO, "rev-parse", "--abbrev-ref", "HEAD"], capture_output=True,
                                text=True, timeout=10).stdout.strip()
        return {"commit": head + ("+dirty" if dirty else ""), "branch": branch}
    except (OSError, subprocess.SubprocessError):
        return {"commit": "unknown", "branch": ""}


def search_version():
    files = [os.path.join(REPO, f) for f in SEARCH_CODE]
    files += [os.path.join(REPO, "rbxrag", f) for f in os.listdir(os.path.join(REPO, "rbxrag")) if f.endswith(".py")]
    return L.file_hash(files)


def stage_key(stage, corpus_keys):
    q = [os.path.join(HERE, "questions.json"), os.path.join(HERE, "exam_questions.json")]
    extra = "%s|%s|%s" % (stage, VERSION.get(stage, 0), ",".join(sorted(corpus_keys)))
    if stage in ("answers", "judge"):
        extra += "|" + L.file_hash([os.path.join(REPO, f) for f in ANSWER_CODE])
    if stage == "judge":
        extra += "|" + L.file_hash([], L.JUDGE_SYSTEM + L.JUDGE_PROMPT)
    return L.file_hash(q, extra + "|" + search_version())


# ------------------------------------------------------------------ index-time and query-time switches
def apply_index_variant(name):
    """Switch a piece of the chunk text off for this process (index-time ablations; their indexes are separate)."""
    from rbxrag import chunks
    if name == "noctx":  # each chunk starts with its title › heading only: no context line
        def prefix(shelf, title, label, s, labels, gist, count_tokens, limit):
            p = "%s%s%s\n" % (title, chunks.SEP, s["heading"])
            return p if count_tokens(p) <= limit // 4 else "%s%s%s\n" % (title[:80], chunks.SEP, s["heading"][:120])
        chunks._prefix = prefix
    elif name == "nogist":  # the context line without the note's gist
        orig = chunks.context_line
        chunks.context_line = lambda shelf, title, label, section, labels, gist: orig(shelf, title, label, section,
                                                                                       labels, "")
    elif name not in ("base", "", None):
        raise SystemExit("unknown index variant %r" % name)


@contextlib.contextmanager
def query_variant(engine, method, name):
    """The engine answering one way for a while: its plan changed, a ranking blanked or link following off."""
    import rbxvault
    from rbxrag import engine as E
    plan = dict(E.PLANS.get(method, {}))
    blank_dense = nolinks = False
    if name == "app":
        pass
    elif name == "nolinks":
        nolinks = True
    elif name == "dense":
        plan.update(keyword=False, sparse=False, rerank_depth=0)
    elif name == "lexical":  # the keyword search's own ranking, each note's part nearest the query as its passage
        plan.update(sparse=False, rerank_depth=0)
        blank_dense = True
    elif name == "norerank":
        plan.update(rerank_depth=0)
    elif name == "sparse":
        plan.update(keyword=False, sparse=True, rerank_depth=0)
        blank_dense = True
    elif name == "dense+sparse":
        plan.update(keyword=False, sparse=True, rerank_depth=0)
    elif name == "app+sparse":
        plan.update(sparse=True)
    else:
        raise ValueError(name)
    saved = (engine.plan, E.Engine._follow_links, rbxvault._linked_pool, engine.store.__dict__.get("rankings"))
    engine.plan = dict(E.DEFAULT_PLAN, **plan)
    if blank_dense:
        orig = type(engine.store).rankings

        def rankings(dense, sparse, flt, keys=()):
            _, sp, kw = orig(engine.store, dense, sparse, flt, keys)
            return [], sp, kw
        engine.store.rankings = rankings
    if nolinks:
        E.Engine._follow_links = lambda self, q, dv, flt, out, k, shelves: out
        rbxvault._linked_pool = lambda q, pool, *a, **kw: pool
    try:
        yield
    finally:
        engine.plan, E.Engine._follow_links, rbxvault._linked_pool = saved[:3]
        if saved[3] is None:
            engine.store.__dict__.pop("rankings", None)
        else:
            engine.store.rankings = saved[3]


@contextlib.contextmanager
def keyword_variant(name):
    import rbxvault
    saved = rbxvault._linked_pool
    if name == "nolinks":
        rbxvault._linked_pool = lambda q, pool, *a, **kw: pool
    try:
        yield
    finally:
        rbxvault._linked_pool = saved


# ------------------------------------------------------------------ embeddings: cached, Heavy's in parallel
class ExamEmbedCache(shim.EmbedCache):
    """The bench's vector cache (the exam keeps its own file), embedding a batch's misses in `par` parallel pieces
    (Ollama answers several requests at once; fastembed already uses every core)."""

    def __init__(self, inner, path, par=1, piece=64):
        super().__init__(inner, path, read=True)
        self.par, self.piece = par, piece
        if par > 1:
            inner_dense = inner.dense

            def dense(texts):
                texts = list(texts)
                if len(texts) <= piece:
                    return inner_dense(texts)
                parts = [texts[i:i + piece] for i in range(0, len(texts), piece)]
                with ThreadPoolExecutor(max_workers=par) as ex:
                    return [v for part in ex.map(inner_dense, parts) for v in part]
            self._par_dense = dense

    def dense(self, texts):
        np = self._np
        compute = getattr(self, "_par_dense", self._inner.dense)
        return self._cached("d", texts, compute, lambda v: np.asarray(v, dtype=np.float32).tobytes(),
                            lambda b: np.frombuffer(b, dtype=np.float32).tolist())


# ------------------------------------------------------------------ Qdrant, started for Heavy
def free_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


class Qdrant:
    """The app's installed Qdrant binary, run on free ports with its storage in the exam cache and a random API key
    (kept in memory). A pid file lets the next run end one a killed run left behind."""

    def __init__(self, storage):
        self.storage = storage
        self.pidfile = os.path.join(os.path.dirname(storage), "qdrant.pid")
        self.proc = None

    def _end_stale(self):
        try:
            with open(self.pidfile) as f:
                pid = int(f.read().strip())
            cmd = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
            if QDRANT_BIN in cmd:
                os.killpg(pid, signal.SIGTERM)
                time.sleep(1.0)
        except (OSError, ValueError):
            pass

    def start(self):
        self._end_stale()
        os.makedirs(self.storage, exist_ok=True)
        self.key = secrets.token_hex(24)
        self.http, self.grpc = free_port(), free_port()
        env = dict(os.environ, QDRANT__SERVICE__HOST="127.0.0.1", QDRANT__SERVICE__HTTP_PORT=str(self.http),
                   QDRANT__SERVICE__GRPC_PORT=str(self.grpc), QDRANT__SERVICE__API_KEY=self.key,
                   QDRANT__STORAGE__STORAGE_PATH=self.storage,
                   QDRANT__STORAGE__SNAPSHOTS_PATH=os.path.join(self.storage, "snapshots"),
                   QDRANT__TELEMETRY_DISABLED="true", QDRANT__LOG_LEVEL="WARN")
        log = open(os.path.join(os.path.dirname(self.storage), "qdrant.log"), "ab")
        self.proc = subprocess.Popen([QDRANT_BIN], env=env, cwd=os.path.dirname(self.storage), stdout=log,
                                     stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, start_new_session=True)
        with open(self.pidfile, "w") as f:
            f.write(str(self.proc.pid))
        self.url = "http://127.0.0.1:%d" % self.http
        for _ in range(120):
            if self.proc.poll() is not None:
                raise SystemExit("Qdrant stopped at start (see %s)" % log.name)
            try:
                req = urllib.request.Request(self.url + "/readyz", headers={"api-key": self.key})
                with urllib.request.urlopen(req, timeout=2) as r:
                    if r.status == 200:
                        return self
            except OSError:
                pass
            time.sleep(0.5)
        self.stop()
        raise SystemExit("Qdrant did not answer within 60 s")

    def client(self):
        import warnings
        from qdrant_client import QdrantClient
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            return QdrantClient(url=self.url, api_key=self.key, check_compatibility=False, timeout=120)

    def rss_mb(self):
        return rss_mb(self.proc.pid) if self.proc else None

    def stop(self):
        if self.proc and self.proc.poll() is None:
            try:
                os.killpg(self.proc.pid, signal.SIGTERM)
                self.proc.wait(timeout=20)
            except (OSError, subprocess.TimeoutExpired):
                try:
                    os.killpg(self.proc.pid, signal.SIGKILL)
                except OSError:
                    pass
        try:
            os.remove(self.pidfile)
        except OSError:
            pass


# ------------------------------------------------------------------ the worker (one process per index)
class Adapter:
    """What rbxrag.serve.make_server needs: the app's own rbxrag.serve.Service over the already built engine (so a
    search pays the same throttled refresh it pays in the app), timed."""

    def __init__(self, engine, mode="light"):
        from rbxrag.serve import Service
        self.svc = Service(mode, lambda: engine)
        self.svc.engine = engine
        self.svc.ready = True
        self.svc._last_sync = self.svc.clock()  # the worker synced it just now
        self.last = None

    def status(self):
        return self.svc.status()

    def query(self, q, shelves, k, project, filters=None):
        t = time.perf_counter()
        hits = self.svc.query(q, shelves, k, project, filters) if filters else self.svc.query(q, shelves, k, project)
        self.last = (time.perf_counter() - t, hits)
        return hits


class Endpoint:
    """A real /query server (rbxrag.serve.make_server) over the engine on 127.0.0.1, and the endpoint file rbx_vault
    reads (FRIDAE_RAG_FILE). Its token is random, in memory and in a 0600 file in a temporary folder."""

    def close(self):
        self.srv.shutdown()
        self.srv.server_close()


RAG_MARK = "The lines under each result are its matching passage."  # only rag_search's text says this


def make_endpoint(engine, tmp, mode="light"):
    from rbxrag.serve import make_server
    import rbxvault
    ep = Endpoint.__new__(Endpoint)
    ep.adapter = Adapter(engine, mode)
    token = secrets.token_hex(24)
    ep.srv = make_server(ep.adapter, token, 0)
    threading.Thread(target=ep.srv.serve_forever, daemon=True).start()
    ep.file = os.path.join(tmp, "rag-endpoint.json")
    fd = os.open(ep.file, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, "w") as f:
        json.dump({"url": "http://127.0.0.1:%d" % ep.srv.server_address[1], "token": token}, f)
    rbxvault.RAG_TIMEOUT = 600.0
    return ep


@contextlib.contextmanager
def env(**kw):
    old = {k: os.environ.get(k) for k in kw}
    for k, v in kw.items():
        if v is None:
            os.environ.pop(k, None)
        else:
            os.environ[k] = v
    try:
        yield
    finally:
        for k, v in old.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


def engine_shelves(q, project):
    sh = shim.shelves_for(q["shelves"])
    return sh + ["game"] if project and "game" not in sh else sh


def kw_signals(q, shelves):
    """The keyword search's confidence: the best result's share of the query's words (coverage) and the vault's raw
    BM25 score."""
    import rbxvault
    pool, _ = rbxvault._keyword_pool(q, shelves, 5)
    cov = max((s for s, _, _ in [x[:3] for x in pool]), default=0.0)
    bm25 = 0.0
    if "vault" in shelves:
        res, _ = rbxvault.search(q, 1, rbxvault.load_index())
        bm25 = float(res[0][0]) if res else 0.0
    return {"kw_cov": float(cov), "kw_bm25": bm25}


def titles_from_text(text):
    import run as bench
    return bench.titles_from_text(text)


class Backend:
    """Light or Heavy for one index folder: its embedders (cached vectors), its store (Heavy: a Qdrant started
    here), and the engine with the app's plan and keyword ranker."""

    def __init__(self, spec):
        self.spec = spec
        self.method = spec["method"]
        self.index = spec["index"]
        self.qdrant = None

    def __enter__(self):
        from rbxrag.engine import PLANS, Engine, KeywordRanker
        t = time.perf_counter()
        cache = os.path.join(self.spec["cache"], "embed-cache.sqlite")
        if self.method == "light":
            from rbxrag.models import LightEmbedders
            from rbxrag.stores import MemStore
            self.raw = LightEmbedders(self.spec["models"])
            self.emb = ExamEmbedCache(self.raw, cache)
            folder = os.path.join(self.index, "light")
            os.makedirs(folder, exist_ok=True)
            store = MemStore(folder)
        else:
            from rbxrag.models import HeavyEmbedders
            self.qdrant = Qdrant(os.path.join(self.index, "qdrant-storage")).start()
            self.raw = HeavyEmbedders(self.spec["models"], OLLAMA_URL)
            self.emb = ExamEmbedCache(self.raw, cache, par=int(self.spec.get("ollama_par") or 3))
            folder = os.path.join(self.index, "heavy")
            os.makedirs(folder, exist_ok=True)
            store = self.qdrant.client()
        self.load_s = time.perf_counter() - t
        self.engine = Engine(store, self.emb, os.path.join(folder, "manifest.json"), keyword=KeywordRanker(),
                             plan=PLANS[self.method])
        return self

    def __exit__(self, *exc):
        if self.qdrant:
            self.qdrant.stop()

    def disk_mb(self):
        return dir_mb(self.index)


def worker_build(spec, be, game):
    eng = be.engine
    last = [0.0]
    t0 = time.perf_counter()
    name = os.path.basename(spec["index"])

    def progress(step, fraction):
        if fraction - last[0] >= 0.02 or (fraction >= 1.0 and last[0] < 1.0):
            el = time.perf_counter() - t0
            eta = el / fraction * (1 - fraction) if fraction > 0 else 0
            say("  build %s %s %5.1f%%  %6.0f s so far, about %6.0f s to go" % (name, step, 100 * fraction, el, eta))
            last[0] = fraction
    stats = eng.sync(game, progress=progress, full=True)
    build_s = time.perf_counter() - t0
    meta = {"build_s": build_s, "stats": stats, "embed_hits": be.emb.hits, "embed_misses": be.emb.misses,
            "embed_s": be.emb.embed_s, "model_load_s": be.load_s, "peak_rss_build_mb": peak_rss_mb(),
            "qdrant_rss_mb": be.qdrant.rss_mb() if be.qdrant else None, "disk_mb": be.disk_mb(),
            "loadavg_before": spec.get("loadavg"), "loadavg_after": os.getloadavg(),
            "when": datetime.datetime.now().isoformat(timespec="seconds"), "search": search_version(),
            "complete": True}
    L.write_json(os.path.join(spec["index"], "exam-meta.json"), meta)
    return meta


def query_rows(spec, eng, be, game, qs, variants):
    """Each question under each query variant, the engine asked directly for K_MAX results."""
    method = spec["method"]
    rows = {}
    for v in variants:
        if v in ("sparse", "dense+sparse", "app+sparse") and method != "heavy":
            continue
        out = []
        with query_variant(eng, method, v):
            for q in qs:
                project = game if q["set"] == "scope" else None
                shelves = engine_shelves(q, project)
                t = time.perf_counter()
                hits = eng.query(q["q"], shelves, K_MAX, project)
                dt = time.perf_counter() - t
                row = {"id": q["id"], "set": q["set"], "secs": dt,
                       "titles": L.dedupe([h.get("title") for h in hits])[:K_MAX],
                       "linked": [h.get("title") for h in hits if h.get("linked_from")]}
                if q["set"] == "scope":
                    row["titles_this"] = L.dedupe([h.get("title") for h in eng.query(
                        q["q"], shelves, K_MAX, project, {"game": "this"})])[:K_MAX]
                    row["titles_general"] = L.dedupe([h.get("title") for h in eng.query(
                        q["q"], shelves, K_MAX, project, {"game": "general"})])[:K_MAX]
                if v == "app":  # how sure the search is: for the "not found" threshold
                    row.update(kw_signals(q["q"], shelves))
                    dv = [float(x) for x in be.emb.dense_query(q["q"])]
                    flt = eng._filter(shelves, project)
                    keys = [(h["shelf"], h["title"], None) for h in hits[:5]]
                    parts = eng.store.best_parts(dv, flt, keys) if keys else {}
                    row["dense_top"] = max((p[0] for p in parts.values()), default=None)
                    if method == "heavy" and hits:
                        rr = list(be.emb.rerank(q["q"], [h.get("text") or "" for h in hits[:5]], max_tokens=256))
                        row["rerank_top"] = float(max(rr)) if rr else None
                out.append(row)
        rows[v] = out
        say("  %s %s: %d questions" % (os.path.basename(spec["index"]), v, len(out)))
    return rows


def search_text_rows(qs, game, search, ks=KS):
    """rbx_vault's search text at each k: tokens, ranks from the text, whether guide excerpts carry their tag."""
    out = {}
    for k in ks:
        rows = []
        for q in qs:
            project = game if q["set"] == "scope" else ""
            with env(RBXKIT_PROJECT=project):
                t = time.perf_counter()
                text, engine_secs, via = search(q, k)
                e2e = time.perf_counter() - t
            res = L.dedupe([t for _, t in titles_from_text(text)])
            rows.append({"id": q["id"], "set": q["set"], "tokens": L.rough_tokens(text), "chars": len(text or ""),
                         "titles": res[:k], "e2e_secs": e2e, "secs": engine_secs, "via": via,
                         "over_app_timeout": engine_secs is not None and engine_secs > APP_TIMEOUT,
                         "guide_tag": "Excerpt from guide" in (text or "")})
        out[str(k)] = rows
    return out


def update_timing(spec, refresh):
    """One note changes, the index catches up (the refresh a query runs), then it changes back."""
    import run as bench
    note = bench.pick_edit_note(spec["corpus"])
    with bench.Edited(note):
        t = time.perf_counter()
        up = refresh()
        update_s = time.perf_counter() - t
    t = time.perf_counter()
    back = refresh()
    return {"update_s": update_s, "update_stats": up, "restore_s": time.perf_counter() - t, "restore_stats": back}


def worker_query(spec, be, game, qs):
    import rbxvault
    eng = be.engine
    import gc
    gc.collect()
    rss_models = rss_mb()
    t = time.perf_counter()
    open_stats = eng.sync(game)
    open_s = time.perf_counter() - t
    gc.collect()
    rss_open = rss_mb()
    t = time.perf_counter()
    eng.sync(game, quick=True, max_embed=200, blocking=True)
    refresh_s = time.perf_counter() - t
    res = {"method": spec["method"], "variant": spec["ivariant"], "open_s": open_s, "open_stats": open_stats,
           "refresh_nochange_s": refresh_s, "rss_models_mb": rss_models, "rss_open_mb": rss_open,
           "model_load_s": be.load_s, "disk_mb": be.disk_mb()}
    res["rows"] = query_rows(spec, eng, be, game, qs, spec["qvariants"])
    if spec.get("text_pass"):
        tmp = shim.point_at(spec["corpus"])
        ep = make_endpoint(eng, tmp, spec["method"])
        try:
            def search(q, k):
                ep.adapter.last = None
                with env(FRIDAE_RAG_FILE=ep.file):
                    text = rbxvault.run("search", {"query": q["q"], "shelf": "all", "k": k})
                last = ep.adapter.last
                return text, last[0] if last else None, "rag" if RAG_MARK in (text or "") else "keyword"
            res["text"] = search_text_rows(qs, game, search)
            # the same searches with the app's own 3 s timeout: how often an agent silently gets the keyword search
            rbxvault.RAG_TIMEOUT = APP_TIMEOUT
            try:
                res["text_app_timeout"] = search_text_rows(qs, game, search, ks=(5,))["5"]
            finally:
                rbxvault.RAG_TIMEOUT = 600.0
        finally:
            ep.close()
        res.update(update_timing(spec, lambda: eng.sync(game, quick=True, max_embed=200, blocking=True)))
    res.update({"peak_rss_mb": peak_rss_mb(), "qdrant_rss_mb": be.qdrant.rss_mb() if be.qdrant else None,
                "embed_hits": be.emb.hits, "embed_misses": be.emb.misses})
    return res


def worker_keyword(spec, game, qs):
    import rbxvault
    t = time.perf_counter()
    rbxvault.load_index()
    rbxvault.load_sections()
    build_s = time.perf_counter() - t
    rss_built = rss_mb()
    res = {"method": "keyword", "variant": "base", "build_s": build_s, "rss_open_mb": rss_built, "rows": {}}
    for v in spec["qvariants"]:
        rows = []
        with keyword_variant(v):
            for q in qs:
                project = game if q["set"] == "scope" else ""
                shelves = engine_shelves(q, project)
                with env(RBXKIT_PROJECT=project):
                    t = time.perf_counter()
                    text = rbxvault.run("search", {"query": q["q"], "shelf": "all", "k": K_MAX})
                    dt = time.perf_counter() - t
                    row = {"id": q["id"], "set": q["set"], "secs": dt,
                           "titles": L.dedupe([t for _, t in titles_from_text(text)])[:K_MAX]}
                    if q["set"] == "scope":
                        for key, g in (("titles_this", "this"), ("titles_general", "general")):
                            tx = rbxvault.run("search", {"query": q["q"], "shelf": "all", "k": K_MAX, "game": g})
                            row[key] = L.dedupe([t for _, t in titles_from_text(tx)])[:K_MAX]
                    if v == "app":
                        row.update(kw_signals(q["q"], shelves))
                rows.append(row)
        res["rows"][v] = rows
    if spec.get("text_pass"):
        def search(q, k):
            t = time.perf_counter()
            text = rbxvault.run("search", {"query": q["q"], "shelf": "all", "k": k})
            return text, time.perf_counter() - t, "keyword"
        res["text"] = search_text_rows(qs, game, search)
        res.update(update_timing(spec, lambda: rbxvault.run("search", {"query": qs[0]["q"], "shelf": "all", "k": 5})))
    res["peak_rss_mb"] = peak_rss_mb()
    res["disk_mb"] = dir_mb(os.path.join(__import__("rbxpaths").WORK, "cache"))
    return res


def worker_passages(spec, game, ep):
    """The passages rbx lookup (the Librarian's search) gives each part of each answer question: through the index's
    /query (ep) or, without one, the keyword search."""
    import rbxvault
    out = {}
    for item in spec["answers"]:
        project = game if item.get("scope") else ""
        with env(RBXKIT_PROJECT=project, FRIDAE_RAG_FILE=ep.file if ep else None):
            t = time.perf_counter()
            res = rbxvault.lookup(item["parts"], k=spec.get("k", 3))
            dt = time.perf_counter() - t
        out[item["id"]] = {"secs": dt, "parts": [{"question": r.get("query"), "via": r.get("via"),
                                                    "hits": r.get("hits") or [], "error": r.get("error")} for r in res]}
    return out


def worker_main(argv):
    spec = L.read_json(argv[0])
    signal.signal(signal.SIGTERM, lambda *a: sys.exit(143))
    shim.point_at(spec["corpus"])
    apply_index_variant(spec.get("ivariant"))
    game = os.path.join(spec["corpus"], "game")
    qs = spec.get("questions") or []
    if spec["method"] == "keyword":
        res = worker_passages(spec, game, None) if spec["phase"] == "passages" else worker_keyword(spec, game, qs)
    else:
        with Backend(spec) as be:
            if spec["phase"] == "build":
                res = worker_build(spec, be, game)
            elif spec["phase"] == "passages":
                be.engine.sync(game)
                ep = make_endpoint(be.engine, shim.point_at(spec["corpus"]), spec["method"])
                try:
                    res = worker_passages(spec, game, ep)
                finally:
                    ep.close()
            else:
                res = worker_query(spec, be, game, qs)
    L.write_json(spec["out"], res)


# ------------------------------------------------------------------ the orchestrator: helpers
WRITER_VARIANT = {"k5": "default", "rerun": "default"}  # exam setups that run the writer at its default


def lib_k(variant):
    """Passages a part for a Librarian setup: "k5" gets 5, every other 3 (the app's own)."""
    return 5 if variant == "k5" else 3


def passages_name(m, sc, k=3):
    return "passages-%s-%dx%s" % (m, sc, "" if k == 3 else "-k%d" % k)


def reader_name(m, sc, k=3):
    return "reader-%s-%dx%s" % (m, sc, "" if k == 3 else "-k%d" % k)


class Exam:
    def __init__(self, a):
        self.a = a
        self.root = os.path.join(a.cache_dir, "exam")
        os.makedirs(self.root, exist_ok=True)
        self.cache = L.StageCache(os.path.join(self.root, "stages"), code_commit())
        self.work = os.path.join(self.root, "work")
        os.makedirs(self.work, exist_ok=True)
        self.rates_path = os.path.join(self.root, "rates%s.json" % ("-quick" if a.quick else ""))
        self.rates = dict(DEFAULT_RATES)  # the full run's measured rates, then (quick) the quick run's own
        for path in dict.fromkeys([os.path.join(self.root, "rates.json"), self.rates_path]):
            self.rates.update({k: v for k, v in (L.read_json(path, {}) or {}).items() if not k.startswith("_")})
        self.base, self.exam = load_sets()
        self.qs = all_questions(self.base, self.exam)
        self.rq = retrieval_questions(self.qs, a.quick)
        self.entries = answer_entries(self.exam, self.qs, a.answer_limit)
        self.corpora = {}

    # -------------------------------------------------------------- the plan
    def plan(self):
        a = self.a
        top = max(a.scales)
        htop = max(a.heavy_scales) if a.heavy_scales else None
        units = []
        for m in a.methods:
            for s in (a.heavy_scales if m == "heavy" else a.scales):
                units.append((m, s))
        lib = [(m, s, "default") for m, s in units if s == (htop if m == "heavy" else top)]
        lib += [(m, s, "default") for m, s in units if m != "keyword" and s == min(a.scales) and
                (m, s, "default") not in lib]
        if "light" in a.methods:
            lib.append(("light", top, "low"))
            lo = min(a.scales)
            lib += [("light", lo, "k5"), ("light", lo, "rerun")]  # 5 passages a part; the same run again (noise)
        reader = [(m, s, 3) for m, s in units if s == (htop if m == "heavy" else top)]
        if "light" in a.methods:
            reader.append(("light", min(a.scales), 5))
        return {"retrieval": units, "ablation_index": [("light", s, v) for s in a.ablation_scales for v in
                                                       INDEX_VARIANTS] if "light" in a.methods else [],
                "librarian": lib, "reader": reader}

    def corpus_for(self, scale):
        if scale not in self.corpora:
            path, summary = corpus.ensure(self.root, self.base, scale, self.a.quick, log=say, exam=self.exam)
            self.corpora[scale] = (path, summary)
        return self.corpora[scale]

    def derived(self, scale):
        """{distractor title: {"kind", "from", "looks_like"}} of this scale's corpus (corpus.py's manifest)."""
        return L.read_json(os.path.join(self.corpus_for(scale)[0], "distractors.json"), {}) or {}

    def corpus_name(self, scale):
        return os.path.basename(self.corpus_for(scale)[0])

    def est_chunks(self, scale):
        path, summary = self.corpus_for(scale)
        return summary["est_chunks"] * self.rates["chunks_per_est"]

    def index_dir(self, method, scale, ivariant="base"):
        return os.path.join(self.root, "index", "%s-%s-%s-%s" % (method, self.corpus_name(scale), ivariant,
                                                                 search_version()))

    def index_ready(self, method, scale, ivariant="base"):
        if method == "keyword":
            return True
        meta = L.read_json(os.path.join(self.index_dir(method, scale, ivariant), "exam-meta.json"))
        return bool(meta and meta.get("complete"))

    def key(self, stage):
        return stage_key(stage, [self.corpus_name(s) for s in sorted(set(self.a.scales) | set(self.a.heavy_scales))])

    def run_worker(self, spec, timeout=None):
        spec = dict(spec, cache=self.root, models=self.a.models, loadavg=os.getloadavg(),
                    ollama_par=self.a.ollama_par)
        tag = "%s-%s-%s-%s" % (spec["phase"], spec["method"], os.path.basename(spec.get("corpus", "")),
                               spec.get("ivariant", "base"))
        spec_path = os.path.join(self.work, "spec-%s.json" % tag)
        spec["out"] = os.path.join(self.work, "out-%s.json" % tag)
        L.write_json(spec_path, spec)
        t = time.perf_counter()
        p = subprocess.Popen([sys.executable, os.path.abspath(__file__), "_worker", spec_path], start_new_session=True)
        try:
            rc = p.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(p.pid, signal.SIGTERM)
            try:
                p.wait(timeout=30)
            except subprocess.TimeoutExpired:
                os.killpg(p.pid, signal.SIGKILL)
            raise RuntimeError("worker %s took longer than %d s and was stopped" % (tag, timeout))
        except BaseException:
            with contextlib.suppress(OSError):
                os.killpg(p.pid, signal.SIGTERM)
            raise
        if rc != 0:
            raise RuntimeError("worker %s failed (exit %d)" % (tag, rc))
        res = L.read_json(spec["out"])
        res["wall_s"] = time.perf_counter() - t
        return res

    def save_rate(self, name, value):
        if value is None:
            return
        self.rates[name] = value
        cur = L.read_json(self.rates_path, {}) or {}
        cur[name] = value
        L.write_json(self.rates_path, cur)

    # -------------------------------------------------------------- estimates
    def estimate(self, stages):
        p = self.plan()
        r = self.rates
        out = {}
        nq = len(self.rq)
        if "retrieval" in stages:
            s = 0.0
            for m, sc in p["retrieval"]:
                if self.cache.get("retrieval", self.key("retrieval"), "%s-%dx" % (m, sc)):
                    continue
                if m != "keyword" and not self.index_ready(m, sc):
                    s += self.new_chunks(m, sc, p) / r["%s_embed_cps" % m]
                qs = r["%s_query_s" % m]
                s += nq * qs * (2 + len(KS) + 1) + 60  # app (with its signals), the text passes, opening
            out["retrieval"] = s
        if "ablations" in stages:
            s = 0.0
            for m, sc in p["retrieval"]:
                if self.cache.get("ablations", self.key("ablations"), "%s-%dx-query" % (m, sc)):
                    continue
                # Heavy's variants without the reranker cost a tenth of a reranked search
                cost = sum(1.0 if m != "heavy" or v in ("app", "app+sparse", "nolinks") else 0.1
                           for v in QUERY_VARIANTS[m])
                s += nq * r["%s_query_s" % m] * cost + 60
            for m, sc, v in p["ablation_index"]:
                if self.cache.get("ablations", self.key("ablations"), "%s-%dx-%s" % (m, sc, v)):
                    continue
                if not self.index_ready(m, sc, v):
                    s += self.est_chunks(sc) / r["light_embed_cps"]  # new chunk texts: nothing cached
                s += nq * r["light_query_s"] * 2 + 30
            out["ablations"] = s
        if "answers" in stages:
            n = len(self.entries)
            done_l = sum(len(self.cache.records("answers", self.key("answers"), "librarian-%s-%dx-%s" % u))
                         for u in p["librarian"])
            done_r = sum(len(self.cache.records("answers", self.key("answers"), reader_name(*u)))
                         for u in p["reader"])
            s = (n * len(p["librarian"]) - done_l) * r["librarian_answer_s"]
            s += (n * len(p["reader"]) - done_r) * r["reader_answer_s"]
            s += 60 * len(set((m, sc, lib_k(v)) for m, sc, v in p["librarian"]) | set(p["reader"]))  # passages
            out["answers"] = s
        if "judge" in stages:
            n = len(self.entries) * (len(p["librarian"]) + len(p["reader"]))
            out["judge"] = (n / JUDGE_BATCH) / min(4, self.a.jobs) * r["judge_batch_s"] * 1.0
        if "analysis" in stages:
            out["analysis"] = 5.0
        return out

    def new_chunks(self, method, scale, plan):
        """Chunks a build must embed: a bigger corpus holds the smaller one's notes, whose vectors are cached."""
        smaller = [sc for m, sc in plan["retrieval"] if m == method and sc < scale]
        return self.est_chunks(scale) - (self.est_chunks(max(smaller)) if smaller else 0)

    # -------------------------------------------------------------- preflight
    def preflight(self, stages):
        p = self.plan()
        problems = []
        methods = {m for m, _ in p["retrieval"]} if {"retrieval", "ablations", "answers"} & set(stages) else set()
        if methods & {"light", "heavy"}:
            if not self.a.models:
                problems.append("No models folder (--models, FRIDAE_RAG_MODELS or the app's rag/models).")
            else:
                from rbxrag.models import HEAVY, LIGHT, model_present
                need = []
                if "light" in methods:
                    need.append(("dense", LIGHT["dense"]))
                if "heavy" in methods:
                    need += [("sparse", HEAVY["sparse"]), ("rerank", HEAVY["rerank"])]
                for kind, name in need:
                    if not model_present(kind, name, self.a.models):
                        problems.append("The %s model %s is not in %s." % (kind, name, self.a.models))
        if "heavy" in methods:
            try:
                with urllib.request.urlopen(OLLAMA_URL + "/api/tags", timeout=5) as r:
                    tags = json.load(r)
                names = [m.get("name", "") for m in tags.get("models", [])]
                if not any(n.split(":")[0] == "mxbai-embed-large" for n in names):
                    problems.append("Ollama answers but has no mxbai-embed-large (this exam never pulls it).")
            except OSError as e:
                problems.append("Ollama is not answering at %s (%s). Start Ollama first." % (OLLAMA_URL, e))
            if not os.access(QDRANT_BIN, os.X_OK):
                problems.append("No Qdrant binary at %s (the app installs it with Heavy)." % QDRANT_BIN)
            try:
                import qdrant_client  # noqa: F401
            except ImportError:
                problems.append("qdrant_client is not installed in this Python.")
        if "answers" in stages:
            problems += self.preflight_librarian()
        if {"answers", "judge"} & set(stages):
            for model in ([READER_MODEL] if "answers" in stages else []) + ([JUDGE_MODEL] if "judge" in stages else []):
                r = claude_call(model, "Reply with the single word OK.", "Say OK.", self.work, timeout=120)
                if r.get("error") or "ok" not in (r.get("text") or "").casefold():
                    problems.append("claude -p --model %s did not answer: %s" % (model, (r.get("error") or
                                                                                         r.get("text") or "")[:300]))
        return problems

    def ensure_binary(self):
        srcs = [os.path.join(d, f) for d, _, fs in os.walk(os.path.join(REPO, "app-rs", "src")) for f in fs]
        newest = max(os.path.getmtime(s) for s in srcs)
        if not os.path.isfile(FRIDAE_BIN) or os.path.getmtime(FRIDAE_BIN) < newest:
            say("building the exam's Librarian binary from this worktree (cargo build) ...")
            r = subprocess.run(["cargo", "build", "--manifest-path", os.path.join(REPO, "app-rs", "Cargo.toml")],
                               capture_output=True, text=True)
            if r.returncode != 0:
                return "cargo build failed:\n" + r.stderr[-2000:]
        return None

    def librarian_env(self):
        return dict(os.environ, RBXKIT_WORK=self.work)

    def preflight_librarian(self):
        err = self.ensure_binary()
        if err:
            return [err]
        try:
            r = subprocess.run([FRIDAE_BIN, "--librarian-exam", "probe"], capture_output=True, text=True, timeout=150,
                               env=self.librarian_env(), stdin=subprocess.DEVNULL)
        except subprocess.TimeoutExpired:
            subprocess.run([FRIDAE_BIN, "--librarian-exam", "reap"], capture_output=True, env=self.librarian_env())
            return ["OpenCode did not start within 150 s (the Librarian's writer). Is the app's OpenCode signed in?"]
        try:
            v = json.loads(r.stdout.strip().splitlines()[-1])
        except (ValueError, IndexError):
            return ["The Librarian probe gave no answer: %s" % (r.stderr or r.stdout)[-500:]]
        L.write_json(os.path.join(self.root, "opencode-probe.json"), v)
        if not v.get("writers"):
            return ["OpenCode answers but lists none of the Librarian's writer models (signed in to OpenCode Go?)."]
        if v.get("probe", {}).get("error"):
            return ["OpenCode: %s" % v["probe"]["error"]]
        return []

    # -------------------------------------------------------------- stage: retrieval
    def build_index(self, method, scale, ivariant="base"):
        if method == "keyword" or self.index_ready(method, scale, ivariant):
            return L.read_json(os.path.join(self.index_dir(method, scale, ivariant), "exam-meta.json"))
        idx = self.index_dir(method, scale, ivariant)
        shutil.rmtree(idx, ignore_errors=True)
        os.makedirs(idx)
        say("building %s at %dx (%s): about %s" % (method, scale, ivariant, L.fmt_secs(
            self.est_chunks(scale) / self.rates["%s_embed_cps" % method])))
        meta = self.run_worker({"phase": "build", "method": method, "corpus": self.corpus_for(scale)[0],
                                "index": idx, "ivariant": ivariant})
        embedded = (meta.get("stats") or {}).get("embedded") or 0
        if embedded > 1000 and not meta["embed_hits"] and ivariant == "base":  # a cold build: chunks a second
            self.save_rate("%s_embed_cps" % method, embedded / meta["build_s"])
        stats = meta.get("stats") or {}
        summary = self.corpus_for(scale)[1]
        if stats.get("chunks") and summary.get("est_chunks"):
            self.save_rate("chunks_per_est", stats["chunks"] / summary["est_chunks"])
        return meta

    def stage_retrieval(self):
        key = self.key("retrieval")
        for m, sc in self.plan()["retrieval"]:
            name = "%s-%dx" % (m, sc)
            if self.cache.get("retrieval", key, name):
                say("retrieval %s: done before" % name)
                continue
            meta = self.build_index(m, sc)
            say("retrieval %s: %d questions" % (name, len(self.rq)))
            res = self.run_worker({"phase": "query", "method": m, "corpus": self.corpus_for(sc)[0],
                                   "index": self.index_dir(m, sc), "ivariant": "base", "questions": self.rq,
                                   "qvariants": ["app"], "text_pass": True})
            res["build"] = meta
            res["corpus"] = self.corpus_for(sc)[1]
            secs = [r["secs"] for r in res["rows"]["app"]]
            self.save_rate("%s_query_s" % m, L.mean(secs) * (2.5 if m == "heavy" else 1.5))
            self.cache.put("retrieval", key, name, res)

    def stage_ablations(self):
        key = self.key("ablations")
        p = self.plan()
        for m, sc in p["retrieval"]:
            name = "%s-%dx-query" % (m, sc)
            if self.cache.get("ablations", key, name):
                continue
            self.build_index(m, sc)
            say("ablations %s: %s" % (name, ", ".join(QUERY_VARIANTS[m])))
            res = self.run_worker({"phase": "query", "method": m, "corpus": self.corpus_for(sc)[0],
                                   "index": self.index_dir(m, sc), "ivariant": "base", "questions": self.rq,
                                   "qvariants": QUERY_VARIANTS[m]})
            self.cache.put("ablations", key, name, res)
        for m, sc, v in p["ablation_index"]:
            name = "%s-%dx-%s" % (m, sc, v)
            if self.cache.get("ablations", key, name):
                continue
            meta = self.build_index(m, sc, v)
            say("ablations %s" % name)
            res = self.run_worker({"phase": "query", "method": m, "corpus": self.corpus_for(sc)[0],
                                   "index": self.index_dir(m, sc, v), "ivariant": v, "questions": self.rq,
                                   "qvariants": ["app"]})
            res["build"] = meta
            self.cache.put("ablations", key, name, res)

    # -------------------------------------------------------------- stage: answers
    def splits(self, key):
        got = self.cache.get("answers", key, "split")
        if got:
            return got
        if self.ensure_binary():
            raise SystemExit("could not build the Librarian binary")
        qs = [e["question"] for e in self.entries]
        r = subprocess.run([FRIDAE_BIN, "--librarian-exam", "split"], input=json.dumps({"questions": qs}),
                           capture_output=True, text=True, timeout=60, env=self.librarian_env())
        parts = json.loads(r.stdout)["parts"]
        return self.cache.put("answers", key, "split", {e["id"]: p for e, p in zip(self.entries, parts)})

    def passages(self, key, m, sc, split, k=3):
        name = passages_name(m, sc, k)
        got = self.cache.get("answers", key, name)
        if got and all(e["id"] in got for e in self.entries):
            return got
        self.build_index(m, sc)
        items = [{"id": e["id"], "parts": split[e["id"]], "scope": e["scope"]} for e in self.entries]
        res = self.run_worker({"phase": "passages", "method": m, "corpus": self.corpus_for(sc)[0],
                               "index": self.index_dir(m, sc), "ivariant": "base", "answers": items, "k": k})
        return self.cache.put("answers", key, name, res)

    def librarian(self, key, m, sc, variant, passages):
        name = "librarian-%s-%dx-%s" % (m, sc, variant)
        done = self.cache.records("answers", key, name)
        todo = [e for e in self.entries if e["id"] not in done]
        if not todo:
            return
        say("Librarian %s %dx (%s effort): %d answers, %d at a time" % (m, sc, variant, len(todo), self.a.jobs))
        jobs = []
        for e in todo:
            parts = passages[e["id"]]["parts"]
            jobs.append(json.dumps({"id": e["id"], "question": e["question"],
                                    "parts": [{"question": p["question"], "hits": p["hits"]} for p in parts]}))
        cmd = [FRIDAE_BIN, "--librarian-exam", "answer", "--jobs", str(self.a.jobs), "--variant", WRITER_VARIANT.get(variant, variant)]
        limit = (len(todo) // self.a.jobs + 1) * 120 + 180
        t0 = time.perf_counter()
        p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                             env=self.librarian_env(), start_new_session=True)
        err = []
        threading.Thread(target=lambda: err.extend(p.stderr.readlines()), daemon=True).start()
        timer = threading.Timer(limit, lambda: os.killpg(p.pid, signal.SIGTERM))
        timer.start()
        n = 0
        try:
            p.stdin.write("\n".join(jobs) + "\n")
            p.stdin.close()
            for line in p.stdout:
                line = line.strip()
                if not line.startswith("{"):
                    continue
                try:
                    r = json.loads(line)
                except ValueError:
                    continue
                if r.get("id") is None:
                    continue
                r["config"] = {"answerer": "librarian", "method": m, "scale": sc, "variant": variant}
                r["wall_since_start"] = time.perf_counter() - t0
                self.cache.append("answers", key, name, r)
                n += 1
                why = ("" if r.get("format_ok") else " (no writer call: a guide pointer or no passages)"
                       if not r.get("calls") else " (no usable writer answer)" if r.get("writer_raw") is None
                       else " (the writer's reply did not parse)")
                say("  %s %s %.1f s%s" % (name, r["id"], r.get("secs") or 0, why))
            p.wait()
        finally:
            timer.cancel()
            if p.poll() is None:
                os.killpg(p.pid, signal.SIGTERM)
            subprocess.run([FRIDAE_BIN, "--librarian-exam", "reap"], capture_output=True, env=self.librarian_env())
        writers = [x for x in err if x.startswith("{")]
        if writers:
            L.write_json(os.path.join(self.cache.dir("answers", key), "writers-%s.json" % name),
                         {"stderr_first": writers[0].strip()})
        if n:
            self.save_rate("librarian_answer_s", (time.perf_counter() - t0) / n)
        if n < len(todo):
            say("  %s: %d of %d answers came back (the rest are recorded as missing)" % (name, n, len(todo)))

    def reader(self, key, m, sc, k, passages):
        name = reader_name(m, sc, k)
        done = self.cache.records("answers", key, name)
        todo = [e for e in self.entries if e["id"] not in done]
        if not todo:
            return
        say("Sonnet reader %s %dx: %d answers" % (m, sc, len(todo)))
        raw_dir = os.path.join(self.cache.dir("answers", key), "raw")
        os.makedirs(raw_dir, exist_ok=True)
        lock = threading.Lock()
        t0 = time.perf_counter()

        def one(e):
            prompt = reader_prompt(e["question"], passages[e["id"]]["parts"])
            r = claude_call(READER_MODEL, READER_SYSTEM, prompt, self.work, timeout=CLAUDE_TIMEOUT[READER_MODEL])
            if r.get("error"):  # one more try, never more
                r = claude_call(READER_MODEL, READER_SYSTEM, prompt, self.work, timeout=CLAUDE_TIMEOUT[READER_MODEL])
                r["retried"] = True
            L.write_json(os.path.join(raw_dir, "%s-%s.json" % (name, e["id"])), {"prompt": prompt, "out": r})
            rec = {"id": e["id"], "answer": r.get("text"), "secs": r.get("secs"), "usage": r.get("usage"),
                   "error": r.get("error"), "calls": 2 if r.get("retried") else 1, "cost_usd": r.get("cost"),
                   "config": {"answerer": "reader", "method": m, "scale": sc, "variant": READER_MODEL, "k": k}}
            with lock:
                self.cache.append("answers", key, name, rec)
            say("  %s %s %.1f s%s" % (name, e["id"], r.get("secs") or 0, " ERROR" if r.get("error") else ""))
        with ThreadPoolExecutor(max_workers=min(4, self.a.jobs)) as ex:
            list(ex.map(one, todo))
        self.save_rate("reader_answer_s", (time.perf_counter() - t0) / len(todo))

    def stage_answers(self):
        key = self.key("answers")
        p = self.plan()
        split = self.splits(key)
        pas = {}
        for m, sc, v in p["librarian"]:
            pas.setdefault((m, sc, lib_k(v)), None)
        for m, sc, k in p["reader"]:
            pas.setdefault((m, sc, k), None)
        for m, sc, k in pas:
            pas[(m, sc, k)] = self.passages(key, m, sc, split, k)
        for m, sc, v in p["librarian"]:
            self.librarian(key, m, sc, v, pas[(m, sc, lib_k(v))])
        for m, sc, k in p["reader"]:
            self.reader(key, m, sc, k, pas[(m, sc, k)])

    # -------------------------------------------------------------- stage: judge
    def answer_sets(self):
        """[(config name, {id: record})] of every answers unit in the plan."""
        key = self.key("answers")
        p = self.plan()
        out = []
        for m, sc, v in p["librarian"]:
            name = "librarian-%s-%dx-%s" % (m, sc, v)
            out.append((name, self.cache.records("answers", key, name)))
        for m, sc, k in p["reader"]:
            name = reader_name(m, sc, k)
            out.append((name, self.cache.records("answers", key, name)))
        return out

    def stage_judge(self):
        key = self.key("judge")
        raw_dir = os.path.join(self.cache.dir("judge", key), "raw")
        os.makedirs(raw_dir, exist_ok=True)
        done = self.cache.records("judge", key, "verdicts")
        items = []
        by_id = {e["id"]: e for e in self.entries}
        for name, recs in self.answer_sets():
            for qid, r in sorted(recs.items()):
                if qid not in by_id:
                    continue
                k = "%s|%s" % (name, qid)
                if k in done:
                    continue
                e = by_id[qid]
                items.append({"key": k, "question": e["question"], "expect": e["expect"], "facts": e["facts"],
                              "wrong": e["wrong"], "guide": e.get("guide") or "",
                              "answer": r.get("answer") or "(no answer)"})
        if not items:
            say("judge: nothing new to grade")
            return
        random.Random(11).shuffle(items)  # a batch mixes configs, so a config is never graded all in one mood
        batches = [items[i:i + JUDGE_BATCH] for i in range(0, len(items), JUDGE_BATCH)]
        say("judge: %d answers in %d batches (Opus, %d at a time)" % (len(items), len(batches), min(4, self.a.jobs)))
        lock = threading.Lock()
        t0 = time.perf_counter()

        def one(nb):
            n, batch = nb
            prompt = L.JUDGE_PROMPT + L.judge_items_text(batch)
            got, outs = {}, []
            for attempt in range(2):  # a second try for the items the first left ungraded, never more
                left = [it for it in batch if it["key"] not in got]
                if not left:
                    break
                pr = L.JUDGE_PROMPT + L.judge_items_text(left)
                r = claude_call(JUDGE_MODEL, L.JUDGE_SYSTEM, pr, self.work, timeout=CLAUDE_TIMEOUT[JUDGE_MODEL])
                outs.append(r)
                got.update(L.parse_verdicts(r.get("text"), left))
            stamp = "batch-%03d-%s" % (n, secrets.token_hex(3))
            L.write_json(os.path.join(raw_dir, stamp + ".json"), {"prompt": prompt, "outs": outs,
                                                                   "keys": [it["key"] for it in batch]})
            with lock:
                for it in batch:
                    self.cache.append("judge", key, "verdicts", {"id": it["key"], "verdict": got.get(it["key"]),
                                                                 "raw": stamp, "model": JUDGE_MODEL})
            say("  judge batch %d: %d of %d graded" % (n, len(got), len(batch)))
            return outs
        with ThreadPoolExecutor(max_workers=min(4, self.a.jobs)) as ex:
            list(ex.map(one, enumerate(batches)))
        self.save_rate("judge_batch_s", (time.perf_counter() - t0) * min(4, self.a.jobs) / len(batches))

    # -------------------------------------------------------------- stage: analysis and the report
    def stage_analysis(self):
        import exam_report
        text = exam_report.build(self)
        os.makedirs(os.path.dirname(self.a.report), exist_ok=True)
        with open(self.a.report, "w", encoding="utf-8") as f:
            f.write(text)
        say("report: %s" % self.a.report)


# ------------------------------------------------------------------ claude -p (the reader and the judge)
READER_SYSTEM = """You answer a Roblox game developer's question using only the numbered passages from their \
studio's knowledge base. You have no tools.
Rules:
1. Use only what the passages say. Never add facts from your own knowledge.
2. After every sentence that states a fact, cite the passage numbers it comes from, like [2] or [1][3].
3. If the passages do not answer the question, reply exactly: Not in the knowledge base.
4. A passage from the guides shelf is only an excerpt of a step-by-step guide. When the question asks how to do \
something that a guide covers, point to it with a line "Recommended guide: <guide title>" instead of repeating steps.
5. Be brief: at most 150 words."""


def reader_prompt(question, parts):
    lines, n = [], 0
    for p in parts:
        for h in p.get("hits") or []:
            n += 1
            where = "%s: %s" % (h.get("shelf"), h.get("title")) + (" › %s" % h["heading"] if h.get("heading") else "")
            lines.append("[%d] (%s) %s" % (n, where, h.get("text") or ""))
    return "Question: %s\n\nPassages:\n%s\n\nAnswer:" % (question, "\n".join(lines) if lines else "(none)")


def claude_call(model, system, prompt, cwd, timeout=240):
    """One `claude -p` call with no tools, no MCP, no settings, no session saved: {"text", "secs", "usage", "cost",
    "error", "raw"}. Never retried here."""
    exe = shutil.which("claude") or os.path.expanduser("~/.local/bin/claude")
    env = {k: v for k, v in os.environ.items() if k not in CLAUDE_ENV_DROP}
    cmd = [exe, "-p", "--safe-mode", "--model", model, "--tools", "", "--no-session-persistence",
           "--strict-mcp-config", "--disable-slash-commands", "--setting-sources", "", "--output-format", "json",
           "--system-prompt", system]
    d = os.path.join(cwd, "claude")
    os.makedirs(d, exist_ok=True)
    t = time.perf_counter()
    try:
        r = subprocess.run(cmd, input=prompt, capture_output=True, text=True, timeout=timeout, env=env, cwd=d)
    except subprocess.TimeoutExpired:
        return {"error": "timed out after %d s" % timeout, "secs": time.perf_counter() - t}
    except OSError as e:
        return {"error": str(e), "secs": time.perf_counter() - t}
    secs = time.perf_counter() - t
    try:
        v = json.loads(r.stdout)
    except ValueError:
        return {"error": "exit %d: %s" % (r.returncode, (r.stderr or r.stdout)[-400:]), "secs": secs}
    u = v.get("usage") or {}
    usage = {"input": (u.get("input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0) +
             (u.get("cache_read_input_tokens") or 0), "output": u.get("output_tokens") or 0}
    out = {"text": v.get("result") or "", "secs": secs, "usage": usage, "cost": v.get("total_cost_usd"),
           "models": list((v.get("modelUsage") or {}).keys()), "api_ms": v.get("duration_api_ms")}
    if v.get("is_error"):
        out["error"] = (v.get("result") or "error")[:400]
    return out


# ------------------------------------------------------------------ main
def find_models(arg):
    for c in (arg, os.environ.get("FRIDAE_RAG_MODELS"), os.path.join(APP_SUPPORT, "rag", "models")):
        if c and os.path.isdir(c):
            return os.path.abspath(c)
    return None


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "_worker":
        return worker_main(sys.argv[2:])
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("--quick", action="store_true", help="a small smoke: a sampled corpus, a few questions per set")
    p.add_argument("--stages", default=",".join(STAGES))
    p.add_argument("--methods", default="keyword,light,heavy")
    p.add_argument("--scales", default=None, help="default 1,10 (quick: 1,2)")
    p.add_argument("--heavy-scales", default=None, help="default the same as --scales (quick: 1)")
    p.add_argument("--ablation-scales", default="1")
    p.add_argument("--answer-limit", type=int, default=None, help="answer questions (default all; quick 6)")
    p.add_argument("--jobs", type=int, default=4)
    p.add_argument("--ollama-par", type=int, default=3, help="parallel Ollama requests while Heavy embeds")
    p.add_argument("--cache-dir", default=os.environ.get("FRIDAE_SCALE_BENCH_CACHE") or os.path.join(HERE, ".cache"))
    p.add_argument("--models")
    p.add_argument("--report", default=None)
    p.add_argument("--estimate", action="store_true", help="print the estimates and stop")
    p.add_argument("--yes", action="store_true", help="(kept for scripts; the exam never asks)")
    a = p.parse_args()
    a.jobs = max(1, min(4, a.jobs))
    a.methods = [m for m in a.methods.split(",") if m]
    a.scales = [int(x) for x in (a.scales or ("1,2" if a.quick else "1,10")).split(",")]
    a.heavy_scales = [int(x) for x in (a.heavy_scales or ("1" if a.quick else ",".join(map(str, a.scales)))).split(",")]
    a.ablation_scales = [int(x) for x in a.ablation_scales.split(",") if x]
    if a.quick and a.answer_limit is None:
        a.answer_limit = 6
    a.models = find_models(a.models)
    a.cache_dir = os.path.abspath(a.cache_dir)
    if a.report is None:
        a.report = os.path.join(REPO, "docs", "superpowers", "reports", "2026-11-25-knowledge-exam%s.md" %
                                ("-quick" if a.quick else ""))
    stages = [s for s in a.stages.split(",") if s]
    bad = [s for s in stages if s not in STAGES]
    if bad:
        raise SystemExit("unknown stage(s): %s" % ", ".join(bad))
    ex = Exam(a)
    say("knowledge exam%s: stages %s; methods %s; scales %s (Heavy %s); %d retrieval questions, %d answer questions"
        % (" (quick)" if a.quick else "", ",".join(stages), ",".join(a.methods), a.scales, a.heavy_scales,
           len(ex.rq), len(ex.entries)))
    for sc in sorted(set(a.scales) | set(a.heavy_scales)):
        ex.corpus_for(sc)
    est = ex.estimate(stages)
    say("estimated time (from %s rates):" % ("measured" if os.path.isfile(ex.rates_path) else "default"))
    for s in stages:
        say("  %-10s %s" % (s, L.fmt_secs(est.get(s))))
    say("  %-10s %s" % ("total", L.fmt_secs(sum(est.values()))))
    if a.estimate:
        return
    problems = ex.preflight(stages)
    if problems:
        say("cannot start:")
        for pr in problems:
            say("  - " + pr)
        raise SystemExit(2)
    run = dict(code_commit(), started=datetime.datetime.now().isoformat(timespec="seconds"), stages=stages,
               argv=sys.argv[1:], search=search_version(), estimate_s=est)
    say("measuring commit %s (%s)" % (run["commit"], run["branch"]))
    run_path = os.path.join(ex.root, "runs", "%s.json" % run["started"].replace(":", ""))
    L.write_json(run_path, run)
    ex.run_meta = run
    t0 = time.perf_counter()
    for s in stages:
        t = time.perf_counter()
        say("== %s" % s)
        getattr(ex, "stage_" + s)()
        say("== %s took %s" % (s, L.fmt_secs(time.perf_counter() - t)))
        run.setdefault("took_s", {})[s] = time.perf_counter() - t
        L.write_json(run_path, run)
    say("exam done in %s" % L.fmt_secs(time.perf_counter() - t0))


if __name__ == "__main__":
    main()
