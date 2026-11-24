"""The RAG service: one process (started by the app) that keeps the models and the index open and answers on
127.0.0.1. Every request needs `Authorization: Bearer <token>`; the token comes from a file and is never printed.

  GET  /status   {"mode", "ready", "building", "progress": {"step", "fraction"}, "chunks", "error"}
  POST /query    {"q", "shelves", "k", "project"} -> {"hits": [...]}; 503 until the first full sync has finished
  POST /rebuild  {"started": bool}

On start it binds, prints {"listening": port}, then loads the models and builds or syncs the index in a background
thread. Before a query it re-syncs changed files, at most once every 10 seconds, and indexes a game on its first query.
"""
import hmac
import json
import os
import sys
import threading
import time
import traceback
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import rbxvault
from rbxrag.engine import NeedsRebuild, TooBig

THROTTLE = 10.0  # seconds between the quick (mtime, size) syncs a query runs
BIG_CHANGE = 200  # chunks: a refresh embedding more than this runs in the background
MAX_BODY = 1 << 20


class NotReady(Exception):
    pass


class Service:
    def __init__(self, mode, factory, clock=time.monotonic, throttle=THROTTLE, big=BIG_CHANGE):
        self.mode = mode
        self.factory = factory  # () -> rbxrag.engine.Engine; loads the models and opens the index
        self.clock = clock
        self.throttle = throttle
        self.big = big  # a refresh with more chunks to embed than this runs in the background
        self.engine = None
        self.ready = False
        self.building = False
        self.progress = {"step": "", "fraction": 0.0}
        self.chunks = 0
        self.error = ""
        self.last_stats = None
        self._last_sync = None
        self._state = threading.Lock()
        self._quick = threading.Lock()
        self._bg = False

    # ------------------------------------------------------------ building
    def start(self, full=False):
        """Build (or sync) in the background, not ready until it ends; False when a build is already running."""
        with self._state:
            if self.building:
                return False
            self.building = True
            self.ready = False
        threading.Thread(target=self._build, args=(full,), daemon=True, name="rag-build").start()
        return True

    def _on_progress(self, step, fraction):
        self.progress = {"step": step, "fraction": round(float(fraction), 4)}

    def _build(self, full):
        try:
            if self.engine is None:
                self._on_progress("load", 0.0)
                self.engine = self.factory()
            stats = self.engine.sync(None, self._on_progress, full=full)
            self.last_stats = stats
            self.chunks = stats["chunks"]
            self.error = ""
            self._last_sync = self.clock()
            self.ready = True
        except Exception as e:
            self.error = str(e) or type(e).__name__
            print(json.dumps({"error": self.error}), file=sys.stderr, flush=True)
        finally:
            self.building = False

    def _background(self, project):
        """A big refresh, run while searches keep using the index as it is."""
        with self._state:
            if self._bg:
                return
            self._bg = True

        def run():
            try:
                stats = self.engine.sync(project, self._on_progress)
                self.last_stats = stats
                self.chunks = stats["chunks"]
            except Exception as e:
                self.error = "refresh failed: %s" % (str(e) or type(e).__name__)
            finally:
                self._bg = False
        threading.Thread(target=run, daemon=True, name="rag-refresh").start()

    def status(self):
        return {"mode": self.mode, "ready": self.ready, "building": self.building, "progress": dict(self.progress),
                "chunks": self.chunks, "error": self.error}

    # ------------------------------------------------------------ querying
    def _refresh(self, project):
        """The quick refresh before a search: small changes inline, big ones in the background, never a rebuild."""
        if not self._quick.acquire(blocking=False):  # another query is already refreshing: do not wait for it
            return
        try:
            self._last_sync = self.clock()
            stats = self.engine.sync(project, quick=True, max_embed=self.big, blocking=False)
            if stats is not None:  # None: a build or background refresh holds the index; try again later
                self.last_stats = stats
                self.chunks = stats["chunks"]
        except NeedsRebuild:
            with self._state:
                self.ready = False
            self.start()  # the build notices the mismatch and rebuilds; searches get 503 (keyword) meanwhile
            raise NotReady()
        except TooBig:
            self._background(project)
        except Exception as e:  # a failed refresh never stops a search of what is indexed
            self.error = "refresh failed: %s" % (str(e) or type(e).__name__)
        finally:
            self._quick.release()

    def query(self, q, shelves, k, project):
        if not self.ready or self.engine is None:
            raise NotReady()
        eng = self.engine
        due = self._last_sync is None or self.clock() - self._last_sync >= self.throttle
        if due or (project and project not in eng.synced_projects):  # a game is indexed on its first query
            self._refresh(project)
        return eng.query(q, shelves, k, project)


def _project_arg(p):
    p = str(p or "").strip()
    return os.path.abspath(p) if p and os.path.isdir(p) else None


def make_handler(service, token):
    want = ("Bearer " + token).encode()

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"
        server_version = "fridae-rag"
        timeout = 15  # seconds a connection may sit idle or half-sent

        def log_message(self, fmt, *args):  # nothing is logged (requests carry the token in a header)
            pass

        def _send(self, code, data, close=False):
            body = json.dumps(data, ensure_ascii=False).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            if close:  # an unread or refused request body must not be taken for the next request
                self.send_header("Connection", "close")
                self.close_connection = True
            self.end_headers()
            self.wfile.write(body)

        def _authorized(self):
            got = (self.headers.get("Authorization") or "").encode()
            if hmac.compare_digest(got, want):
                return True
            self._send(401, {"error": "unauthorized"}, close=True)
            return False

        def _body(self):
            try:
                n = int(self.headers.get("Content-Length") or 0)
            except ValueError:
                raise ValueError("bad Content-Length")
            if n < 0 or n > MAX_BODY:
                raise ValueError("bad or too large Content-Length (1 MB at most)")
            raw = self.rfile.read(n) if n else b"{}"
            data = json.loads(raw or b"{}")
            if not isinstance(data, dict):
                raise ValueError("expected a JSON object")
            return data

        def do_GET(self):
            if not self._authorized():
                return
            if self.path.split("?")[0] == "/status":
                return self._send(200, service.status())
            self._send(404, {"error": "not found"})

        def do_POST(self):
            if not self._authorized():
                return
            path = self.path.split("?")[0]
            try:
                data = self._body()
            except (ValueError, UnicodeDecodeError) as e:
                return self._send(400, {"error": str(e)}, close=True)
            if path == "/rebuild":
                return self._send(200, {"started": service.start(full=True)})
            if path != "/query":
                return self._send(404, {"error": "not found"})
            q = str(data.get("q") or "").strip()
            shelves = [s for s in (data.get("shelves") or rbxvault.SHELVES) if s in rbxvault.SHELVES]
            try:
                k = max(1, min(int(data.get("k") or 5), 20))
            except (TypeError, ValueError):
                return self._send(400, {"error": "k must be a whole number"}, close=True)
            if not q:
                return self._send(400, {"error": "q is empty"}, close=True)
            try:
                hits = service.query(q, shelves, k, _project_arg(data.get("project")))
            except NotReady:
                return self._send(503, {"error": "the knowledge index is not ready yet", "status": service.status()})
            except Exception as e:
                traceback.print_exc(file=sys.stderr)
                return self._send(500, {"error": str(e) or type(e).__name__})
            self._send(200, {"hits": hits})

    return Handler


def make_server(service, token, port=0):
    if not token:
        raise ValueError("an empty token is not allowed")
    srv = ThreadingHTTPServer(("127.0.0.1", int(port)), make_handler(service, token))
    srv.daemon_threads = True
    return srv


def read_token(path):
    with open(path, encoding="utf-8") as f:
        tok = f.read().strip()
    if not tok:
        raise ValueError("the token file is empty")
    return tok


def light_factory(data):
    def make():
        from rbxrag.engine import Engine
        from rbxrag.models import LightEmbedders
        from rbxrag.stores import MemStore
        from rbxrag.setup import missing_models, models_dir
        missing = missing_models("light", data)
        if missing:
            raise RuntimeError("The Light models are not downloaded yet (missing: %s). Run the Set up step." %
                               ", ".join(missing))
        emb = LightEmbedders(models_dir(data))
        folder = os.path.join(data, "light")
        os.makedirs(folder, exist_ok=True)
        return Engine(MemStore(folder), emb, os.path.join(folder, "manifest.json"))
    return make


def heavy_factory(data, qdrant_url, qdrant_key, ollama_url=None):
    def make():
        import warnings
        from qdrant_client import QdrantClient
        from rbxrag.engine import Engine
        from rbxrag.models import OLLAMA_URL, HeavyEmbedders
        from rbxrag.setup import missing_models, models_dir
        missing = missing_models("heavy", data)
        if missing:
            raise RuntimeError("The reranker is not downloaded yet (missing: %s). Run step 4 (Start the reranker)." %
                               ", ".join(missing))
        if not qdrant_url:
            raise RuntimeError("Heavy needs the Qdrant service (no --qdrant-url was given).")
        emb = HeavyEmbedders(models_dir(data), ollama_url or OLLAMA_URL)
        with warnings.catch_warnings():  # an API key over plain http is fine on 127.0.0.1
            warnings.simplefilter("ignore")
            client = QdrantClient(url=qdrant_url, api_key=qdrant_key or None, check_compatibility=False, timeout=30)
        for attempt in range(30):  # the app starts Qdrant just before this process: give it a moment to listen
            try:
                client.get_collections()
                break
            except Exception as e:
                if attempt == 29:
                    raise RuntimeError("Qdrant is not answering at %s (%s)." % (qdrant_url, type(e).__name__))
                time.sleep(1.0)
        folder = os.path.join(data, "heavy")
        os.makedirs(folder, exist_ok=True)
        return Engine(client, emb, os.path.join(folder, "manifest.json"))
    return make


def run(mode, data, port, token_file, qdrant_url=None, qdrant_key_file=None, ollama_url=None, out=None):
    """`rag serve`: bind, say so, build in the background, then serve until killed."""
    # set before fastembed (and huggingface_hub) are imported: a query never reaches the network
    os.environ["HF_HUB_OFFLINE"] = "1"
    os.environ["HF_HUB_DISABLE_TELEMETRY"] = "1"
    out = out or (lambda line: print(line, flush=True))
    token = read_token(token_file)
    key = read_token(qdrant_key_file) if qdrant_key_file else None
    data = os.path.abspath(data)
    os.makedirs(data, exist_ok=True)
    factory = light_factory(data) if mode == "light" else heavy_factory(data, qdrant_url, key, ollama_url)
    service = Service(mode, factory)
    srv = make_server(service, token, port)
    out(json.dumps({"listening": srv.server_address[1]}))
    service.start()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()
