"""The RAG service, its setup steps and the real embedders' wrappers, all against stand-ins: no model download, no
real Ollama, no real Qdrant server, no network beyond 127.0.0.1."""
import io
import json
import os
import shutil
import socket
import tarfile
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

import numpy as np

from rbxrag import models, serve, setup
from rbxrag.engine import Engine
from rbxrag.stores import MemStore
from tests.test_rag_engine import FakeEmbedders, write

try:
    from qdrant_client import QdrantClient
except ImportError:  # pragma: no cover
    QdrantClient = None

TOKEN = "s3cret-token"


def call(base, path, body=None, token=TOKEN, timeout=5):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json"}
    if token is not None:
        headers["Authorization"] = "Bearer " + token
    req = urllib.request.Request(base + path, data=data, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.load(r)
    except urllib.error.HTTPError as e:
        with e:
            try:
                return e.code, json.load(e)
            except ValueError:
                return e.code, None


class env_file:
    """An endpoint file for rbx_vault, as the app writes it."""

    def __init__(self, base, token=TOKEN):
        self.data = {"url": base, "token": token}

    def __enter__(self):
        fd, self.path = tempfile.mkstemp(prefix="rag-endpoint-", suffix=".json")
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(self.data, f)
        return self.path

    def __exit__(self, *exc):
        os.remove(self.path)


def wait_for(cond, timeout=10.0):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if cond():
            return True
        time.sleep(0.02)
    return cond()


class Kit:
    """A few notes on disk and a corpus function over them."""

    def __init__(self, tc):
        self.tmp = tempfile.mkdtemp(prefix="rag serve é ")
        tc.addCleanup(shutil.rmtree, self.tmp, True)
        self.moon = write(os.path.join(self.tmp, "kit", "Moon.md"), "# Moon\n\n## Rockets\n\nBuild a rocket from scrap.\n")
        self.coins = write(os.path.join(self.tmp, "kit", "Coins.md"), "# Coins\n\n## Meadow\n\nCollect shiny coins.\n")
        self.files = [("yours", "Moon", self.moon), ("guides", "Coins", self.coins)]
        self.gate = None  # an Event the first corpus call waits on

    def corpus(self, project):
        if self.gate is not None:
            self.gate.wait(10)
        return list(self.files)

    def engine(self, client=None, emb=None):
        return Engine(client or QdrantClient(":memory:"), emb or FakeEmbedders(),
                      os.path.join(self.tmp, "rag", "manifest.json"), corpus=self.corpus,
                      project_files=lambda p: set())


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class ServiceTests(unittest.TestCase):
    def start(self, service, token=TOKEN):
        srv = serve.make_server(service, token, 0)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        return "http://127.0.0.1:%d" % srv.server_address[1]

    def test_requires_token(self):
        kit = Kit(self)
        svc = serve.Service("light", kit.engine)
        base = self.start(svc)
        svc.start()
        self.assertEqual(call(base, "/status", token=None)[0], 401)
        self.assertEqual(call(base, "/status", token="wrong")[0], 401)
        self.assertEqual(call(base, "/query", {"q": "coins"}, token=None)[0], 401)
        self.assertEqual(call(base, "/rebuild", {}, token="Bearer " + TOKEN)[0], 401)
        code, st = call(base, "/status")
        self.assertEqual(code, 200)
        self.assertEqual(set(st), {"mode", "ready", "building", "progress", "chunks", "error"})
        self.assertEqual(st["mode"], "light")
        self.assertNotIn(TOKEN, json.dumps(st))
        with self.assertRaises(ValueError):
            serve.make_server(svc, "", 0)

    def test_status_not_ready_while_building(self):
        kit = Kit(self)
        kit.gate = threading.Event()
        svc = serve.Service("light", kit.engine)
        base = self.start(svc)
        svc.start()
        self.assertTrue(wait_for(lambda: svc.building))
        code, st = call(base, "/status")
        self.assertEqual((code, st["ready"], st["building"]), (200, False, True))
        code, body = call(base, "/query", {"q": "rocket", "shelves": ["yours"], "k": 3})
        self.assertEqual(code, 503)
        kit.gate.set()
        self.assertTrue(wait_for(lambda: call(base, "/status")[1]["ready"]))
        code, body = call(base, "/query", {"q": "rocket scrap", "shelves": ["yours", "guides"], "k": 3})
        self.assertEqual(code, 200)
        self.assertEqual(body["hits"][0]["title"], "Moon")
        st = call(base, "/status")[1]
        self.assertGreater(st["chunks"], 0)
        self.assertFalse(st["building"])
        # a rebuild is not served as complete while it runs
        kit.gate = threading.Event()
        self.assertEqual(call(base, "/rebuild", {}), (200, {"started": True}))
        self.assertEqual(call(base, "/query", {"q": "rocket"})[0], 503)
        self.assertEqual(call(base, "/rebuild", {})[1], {"started": False})
        kit.gate.set()
        self.assertTrue(wait_for(lambda: call(base, "/status")[1]["ready"]))

    def test_query_picks_up_edit_after_throttle(self):
        kit = Kit(self)
        clock = [100.0]
        svc = serve.Service("light", kit.engine, clock=lambda: clock[0])
        base = self.start(svc)
        svc.start()
        self.assertTrue(wait_for(lambda: svc.ready))
        q = {"q": "purple moon landing", "shelves": ["yours"], "k": 3}
        self.assertNotIn("purple", json.dumps(call(base, "/query", q)[1]))
        write(kit.moon, "# Moon\n\n## Rockets\n\nBuild a rocket from scrap.\n\n## Landing\n\nLand on the purple moon.\n",
              bump=10 ** 9)
        clock[0] += 5  # inside the 10 s throttle: no re-sync yet
        self.assertNotIn("purple", json.dumps(call(base, "/query", q)[1]))
        clock[0] += 6  # 11 s after the build
        code, body = call(base, "/query", q)
        self.assertEqual(code, 200)
        self.assertEqual(body["hits"][0]["heading"], "Landing")
        self.assertIn("purple", body["hits"][0]["text"])

    def test_bad_requests_refused_and_closed(self):
        kit = Kit(self)
        svc = serve.Service("light", kit.engine)
        base = self.start(svc)
        port = int(base.rsplit(":", 1)[1])
        self.assertEqual(serve.make_handler(svc, TOKEN).timeout, 15)

        def raw(head):
            with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
                s.sendall(head.encode())
                data = b""
                while True:
                    part = s.recv(65536)
                    if not part:
                        break
                    data += part
            return data.decode("utf-8", "replace")  # the server closed the connection, or this would time out
        for length in ("-5", "2000000", "abc"):
            out = raw("POST /query HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer %s\r\nContent-Length: %s\r\n\r\n"
                      % (TOKEN, length))
            self.assertIn(" 400 ", out.split("\r\n")[0], length)
            self.assertIn("Connection: close", out)
        out = raw("POST /query HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\n{}")
        self.assertIn(" 401 ", out.split("\r\n")[0])
        self.assertIn("Connection: close", out)

    def test_quick_sync_never_rebuilds(self):
        kit = Kit(self)
        clock = [100.0]
        svc = serve.Service("light", kit.engine, clock=lambda: clock[0])
        base = self.start(svc)
        svc.start()
        self.assertTrue(wait_for(lambda: svc.ready))
        man_path = os.path.join(kit.tmp, "rag", "manifest.json")
        with open(man_path, encoding="utf-8") as f:
            man = json.load(f)
        man["model"] = "something-else"  # the index no longer matches its manifest
        with open(man_path, "w", encoding="utf-8") as f:
            json.dump(man, f)
        kit.gate = threading.Event()  # hold the rebuild so its not-ready state can be seen
        clock[0] += 11
        self.assertEqual(call(base, "/query", {"q": "rocket"})[0], 503)
        self.assertFalse(call(base, "/status")[1]["ready"])
        kit.gate.set()
        self.assertTrue(wait_for(lambda: svc.ready))
        self.assertEqual(svc.last_stats["embedded"], svc.last_stats["chunks"])  # rebuilt in the background build
        self.assertEqual(call(base, "/query", {"q": "rocket scrap"})[1]["hits"][0]["title"], "Moon")

    def test_big_refresh_runs_in_background(self):
        kit = Kit(self)
        clock = [100.0]
        hold = threading.Event()
        hold.set()

        class Slow(FakeEmbedders):
            def dense(self, texts):
                hold.wait(10)
                return super().dense(texts)
        svc = serve.Service("light", lambda: kit.engine(emb=Slow()), clock=lambda: clock[0], big=1)
        base = self.start(svc)
        svc.start()
        self.assertTrue(wait_for(lambda: svc.ready))
        write(kit.moon, "# Moon\n\n## Rockets\n\nBuild a rocket.\n\n## Landing\n\nLand on the purple moon.\n\n"
                        "## Orbit\n\nCircle the purple moon twice.\n", bump=10 ** 9)
        hold.clear()  # the background embedding waits
        clock[0] += 11
        t = time.monotonic()
        code, body = call(base, "/query", {"q": "purple moon", "shelves": ["yours"], "k": 3})
        self.assertLess(time.monotonic() - t, 2.0)  # answered from the current index, not after the re-embed
        self.assertEqual(code, 200)
        self.assertNotIn("purple", json.dumps(body))
        hold.set()
        self.assertTrue(wait_for(lambda: "purple" in json.dumps(
            call(base, "/query", {"q": "purple moon", "shelves": ["yours"], "k": 3})[1])))

    def test_rebuild_is_full_inside_the_sync(self):
        kit = Kit(self)
        svc = serve.Service("light", kit.engine)
        base = self.start(svc)
        svc.start()
        self.assertTrue(wait_for(lambda: svc.ready))
        man_path = os.path.join(kit.tmp, "rag", "manifest.json")
        self.assertEqual(call(base, "/rebuild", {}), (200, {"started": True}))
        self.assertTrue(wait_for(lambda: svc.ready and not svc.building))
        self.assertEqual(svc.last_stats["embedded"], svc.last_stats["chunks"])
        self.assertTrue(os.path.exists(man_path))

    def test_missing_models_reported_not_ready(self):
        def factory():
            raise RuntimeError("The Light models are not downloaded yet (missing: BAAI/bge-small-en-v1.5).")
        svc = serve.Service("light", factory)
        base = self.start(svc)
        svc.start()
        self.assertTrue(wait_for(lambda: not svc.building and svc.error))
        st = call(base, "/status")[1]
        self.assertFalse(st["ready"])
        self.assertIn("bge-small", st["error"])
        self.assertEqual(call(base, "/query", {"q": "x"})[0], 503)


class FakeDenseModel:
    """Stands in for fastembed's TextEmbedding: numpy vectors from a hashed bag of words."""

    def __init__(self):
        self.fake = FakeEmbedders()

    def embed(self, texts):
        for t in texts:
            yield np.array(self.fake.dense_query(t), dtype=np.float32)

    def query_embed(self, text):
        yield np.array(self.fake.dense_query(text), dtype=np.float32)


class FakeSparseModel:
    class E:
        def __init__(self, pair):
            self.indices = np.array(pair[0], dtype=np.int64)
            self.values = np.array(pair[1], dtype=np.float32)

    def __init__(self):
        self.fake = FakeEmbedders()

    def embed(self, texts):
        for t in texts:
            yield self.E(self.fake.sparse_query(t))

    def query_embed(self, text):
        yield self.E(self.fake.sparse_query(text))


class FakeReranker:
    def rerank(self, query, docs):
        for s in FakeEmbedders().rerank(query, list(docs)):
            yield np.float32(s)


@unittest.skipIf(QdrantClient is None, "qdrant-client not installed")
class NoNetworkTests(unittest.TestCase):
    def test_no_network_in_query(self):
        kit = Kit(self)
        tried = []
        real = socket.socket.connect

        def guarded(sock, address, *a, **kw):  # every TCP/UDP connect, whatever library makes it
            host = address[0] if isinstance(address, tuple) else address
            if host not in ("127.0.0.1", "::1"):
                tried.append(host)
                raise OSError("network blocked in this test: %s" % host)
            return real(sock, address, *a, **kw)

        emb = models.LightEmbedders(dense=FakeDenseModel(), sparse=FakeSparseModel(), reranker=FakeReranker())
        self.assertEqual(emb.dim, 384)
        emb.dim = 64  # the stand-in dense model gives 64 numbers
        store = MemStore(os.path.join(kit.tmp, "rag", "light"))  # Light's own store
        with mock.patch.object(socket.socket, "connect", guarded):
            svc = serve.Service("light", lambda: kit.engine(store, emb))
            srv = serve.make_server(svc, TOKEN, 0)
            threading.Thread(target=srv.serve_forever, daemon=True).start()
            self.addCleanup(srv.server_close)
            self.addCleanup(srv.shutdown)
            base = "http://127.0.0.1:%d" % srv.server_address[1]
            svc.start()
            self.assertTrue(wait_for(lambda: svc.ready), svc.error)
            code, body = call(base, "/query", {"q": "shiny coins meadow", "shelves": ["guides", "yours"], "k": 2})
            with env_file(base) as path, mock.patch.dict(os.environ, {"FRIDAE_RAG_FILE": path, "RBXKIT_PROJECT": ""}):
                import rbxvault
                out = rbxvault.rag_search("shiny coins meadow", ["guides", "yours"], 2)
        self.assertEqual(code, 200)
        self.assertEqual(body["hits"][0]["title"], "Coins")
        self.assertIsInstance(body["hits"][0]["score"], float)
        self.assertIsNotNone(out)  # rbx_vault's call reached the service too, on 127.0.0.1 only
        self.assertEqual(tried, [])


class StubOllama:
    """A local stand-in for Ollama's /api/embed that refuses inputs longer than `limit` words, like a model whose
    context is full, and records every request body."""

    def __init__(self, tc, limit=50):
        self.bodies = []
        self.paths = []
        stub = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _json(self, out, code=200):
                raw = json.dumps(out).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_GET(self):
                stub.paths.append(self.path)
                if self.path == "/api/version":
                    return self._json({"version": "0.0-stub"})
                return self._json({"models": [{"name": "mxbai-embed-large:latest"}]})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                stub.paths.append(self.path)
                if self.path == "/api/pull":
                    raw = b"".join(json.dumps(x).encode() + b"\n" for x in (
                        {"status": "pulling", "total": 10, "completed": 10}, {"status": "success"}))
                    self.send_response(200)
                    self.send_header("Content-Length", str(len(raw)))
                    self.end_headers()
                    return self.wfile.write(raw)
                stub.bodies.append(body)
                if any(len(t.split()) > limit for t in body["input"]):
                    out, code = {"error": "the input length exceeds the context length"}, 400
                else:
                    out, code = {"embeddings": [[float(len(t.split())), 1.0, 0.0, 0.5] for t in body["input"]]}, 200
                raw = json.dumps(out).encode()
                self.send_response(code)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        tc.addCleanup(self.srv.server_close)
        tc.addCleanup(self.srv.shutdown)
        self.url = "http://127.0.0.1:%d" % self.srv.server_address[1]


class HeavyTests(unittest.TestCase):
    def test_loopback_calls_ignore_proxy_settings(self):
        from tests.test_rag_vault import StubProxy
        stub, proxy = StubOllama(self), StubProxy(self)
        with proxy.env():
            emb = models.HeavyEmbedders(cache_dir=None, ollama_url=stub.url, sparse=object(), reranker=object())
            self.assertEqual(len(emb.dense(["a few words"])), 1)
            self.assertEqual(setup.http_json(stub.url + "/api/version"), {"version": "0.0-stub"})
            self.assertTrue(setup.ollama_has_model(stub.url))
            self.assertEqual([e.get("status") for e in setup._pull_stream(stub.url, "mxbai-embed-large")],
                             ["pulling", "success"])
        self.assertEqual(proxy.seen, [])
        self.assertEqual(stub.paths, ["/api/embed", "/api/version", "/api/tags", "/api/pull"])

    def test_heavy_embed_shrinks_on_context_error(self):
        stub = StubOllama(self, limit=50)
        emb = models.HeavyEmbedders(cache_dir=None, ollama_url=stub.url, sparse=object(), reranker=object())
        long_text = " ".join("word%d" % i for i in range(180))
        out = emb.dense([long_text, "a short one"])
        self.assertEqual(len(out), 2)
        self.assertEqual(len(out[0]), 4)
        self.assertGreater(len(stub.bodies), 2)  # refused, then split and halved
        for b in stub.bodies:
            self.assertIs(b["truncate"], False)
            self.assertEqual(b["keep_alive"], "30m")  # the model stays loaded between searches
            self.assertEqual(b["model"], "mxbai-embed-large")
        # every word of the long text was embedded in some accepted request: nothing was dropped
        accepted = [t for b in stub.bodies if all(len(x.split()) <= 50 for x in b["input"]) for t in b["input"]]
        self.assertEqual(sorted(" ".join(accepted).split()), sorted((long_text + " a short one").split()))
        q = emb.dense_query("find lanterns")
        self.assertEqual(len(q), 4)
        self.assertTrue(stub.bodies[-1]["input"][0].startswith(models.MXBAI_QUERY))

    def test_ollama_down_is_a_plain_error(self):
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
        s.close()
        emb = models.HeavyEmbedders(cache_dir=None, ollama_url="http://127.0.0.1:%d" % port, sparse=object(),
                                    reranker=object(), timeout=2)
        with self.assertRaises(models.OllamaError) as cm:
            emb.dense(["x"])
        self.assertIn("Ollama", str(cm.exception))


class SetupTests(unittest.TestCase):
    def setUp(self):
        self.data = tempfile.mkdtemp(prefix="rag setup é ")
        self.addCleanup(shutil.rmtree, self.data, True)

    def test_setup_lines_shape(self):
        have = set()

        def downloader(kind, name, cache_dir):
            self.assertEqual(cache_dir, os.path.join(self.data, "models"))
            have.add(name)

        present = lambda kind, name, cache_dir: name in have
        lines = []
        done = setup.run_cli("models", "light", self.data, out=lines.append, downloader=downloader, present=present)
        rows = [json.loads(l) for l in lines]
        self.assertTrue(all(set(r) == {"step", "fraction"} for r in rows[:-1]))
        self.assertGreaterEqual(len(rows), 7)  # 0 and 1 for each of the three models, then the end
        self.assertEqual(rows[-1], {"ready": True, "missing": [], "error": ""})
        self.assertEqual(done["ready"], True)
        self.assertEqual(have, {models.LIGHT["dense"], models.LIGHT["sparse"], models.LIGHT["rerank"]})
        # idempotent: everything is there, so it ends ready at once
        lines = []
        setup.run_cli("models", "light", self.data, out=lines.append, downloader=downloader, present=present)
        self.assertEqual([json.loads(l) for l in lines], [{"ready": True, "missing": [], "error": ""}])

        def broken(kind, name, cache_dir):
            raise RuntimeError("Could not download %s: no internet" % name)
        lines = []
        setup.run_cli("models", "heavy", self.data, out=lines.append, downloader=broken,
                      present=lambda kind, name, cache_dir: False)
        end = json.loads(lines[-1])
        self.assertFalse(end["ready"])
        self.assertIn("no internet", end["error"])
        self.assertEqual(end["missing"], [models.HEAVY["sparse"], models.HEAVY["rerank"]])

    def test_qdrant_install_checks_sha256_and_version(self):
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode="w:gz") as tar:
            raw = b"#!/bin/sh\necho qdrant\n"
            info = tarfile.TarInfo("qdrant-dir/qdrant")
            info.size = len(raw)
            tar.addfile(info, io.BytesIO(raw))
        blob = buf.getvalue()
        good = __import__("hashlib").sha256(blob).hexdigest()
        urls = []

        def fetch(url, dest, on_fraction=None):
            urls.append(url)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "wb") as f:
                f.write(blob)
            if on_fraction:
                on_fraction(1.0)
        # the pinned checksum does not match this stand-in archive: refused, deleted, nothing installed
        lines = []
        setup.run_cli("qdrant-install", "heavy", self.data, out=lines.append, fetch=fetch, arch="arm64")
        end = json.loads(lines[-1])
        self.assertFalse(end["ready"])
        self.assertIn("checksum", end["error"])
        self.assertEqual(urls, ["https://github.com/qdrant/qdrant/releases/download/v1.20.2/"
                                "qdrant-aarch64-apple-darwin.tar.gz"])
        self.assertFalse(os.listdir(os.path.join(self.data, "downloads")))
        self.assertFalse(os.path.exists(setup.qdrant_binary(self.data)))
        # with the archive's own checksum it installs, records the version, and is not downloaded again
        lines = []
        setup.run_cli("qdrant-install", "heavy", self.data, out=lines.append, fetch=fetch, sha256=good)
        self.assertEqual(json.loads(lines[-1]), {"ready": True, "missing": [], "error": ""})
        self.assertTrue(setup.has_qdrant(self.data))
        with open(setup.qdrant_version_file(self.data), encoding="utf-8") as f:
            self.assertEqual(f.read().strip(), "v1.20.2")
        self.assertFalse(os.listdir(os.path.join(self.data, "downloads")))
        lines = []
        setup.run_cli("qdrant-install", "heavy", self.data, out=lines.append,
                      fetch=lambda *a, **k: self.fail("downloaded again"))
        self.assertEqual(len(lines), 1)
        # a binary of another version counts as not installed
        with open(setup.qdrant_version_file(self.data), "w", encoding="utf-8") as f:
            f.write("v1.0.0\n")
        self.assertFalse(setup.has_qdrant(self.data))
        self.assertFalse(setup.check("heavy", self.data, get=lambda u: None, present=lambda *a: True)["qdrant_binary"])
        self.assertEqual(setup.QDRANT_SHA256["x86_64"],
                         "73a6538ccccb892ec2c6fa3431faf1e73fba7eb2179079adbd0c65ae4c55bf6a")

    def ollama_world(self, codesign_ok=True, complete=True, cli=None, app=None):
        """Stand-ins for the Ollama install: a fake zip download, ditto, codesign, open and the local API."""
        state = {"opened": False, "fetched": 0, "calls": []}
        apps = os.path.join(self.data, "Applications")

        def fetch(url, dest, on_fraction=None):
            state["fetched"] += 1
            self.assertEqual(url, setup.OLLAMA_ZIP)
            os.makedirs(os.path.dirname(dest), exist_ok=True)
            with open(dest, "wb") as f:
                f.write(b"zip")

        def run(cmd, **kw):
            state["calls"].append(cmd[0])
            if cmd[0] == "ditto":
                os.makedirs(os.path.join(cmd[-1], "Ollama.app", "Contents", "MacOS" if complete else "Resources"))
                return mock.Mock(returncode=0, stderr="")
            if cmd[0] == "codesign":
                return mock.Mock(returncode=0 if codesign_ok else 1, stderr="" if codesign_ok else "invalid signature")
            if cmd[0] == "open":
                state["opened"] = True
            return mock.Mock(returncode=0, stderr="")
        hooks = {"fetch": fetch, "run": run, "apps_dir": apps, "wait": 0,
                 "get": lambda url: {"version": "x"} if state["opened"] else None,
                 "which": lambda name: cli if name == "ollama" else "/usr/bin/codesign",
                 "find_app": lambda: app}
        return state, apps, hooks

    def test_ollama_install_verifies_then_moves_whole_app(self):
        state, apps, hooks = self.ollama_world()
        lines = []
        setup.run_cli("ollama-install", "heavy", self.data, out=lines.append, **hooks)
        self.assertEqual(json.loads(lines[-1]), {"ready": True, "missing": [], "error": ""})
        self.assertEqual(state["calls"], ["ditto", "codesign", "open"])
        self.assertTrue(os.path.isdir(os.path.join(apps, "Ollama.app", "Contents", "MacOS")))
        self.assertEqual(os.listdir(apps), ["Ollama.app"])
        self.assertEqual(os.listdir(os.path.join(self.data, "downloads")), [])

    def test_ollama_install_never_leaves_a_bad_app(self):
        for kind in ("codesign", "incomplete"):
            data = tempfile.mkdtemp(prefix="rag ollama ")
            self.addCleanup(shutil.rmtree, data, True)
            self.data = data
            state, apps, hooks = self.ollama_world(codesign_ok=kind != "codesign", complete=kind != "incomplete")
            lines = []
            setup.run_cli("ollama-install", "heavy", data, out=lines.append, **hooks)
            end = json.loads(lines[-1])
            self.assertFalse(end["ready"], kind)
            self.assertIn("signature" if kind == "codesign" else "complete", end["error"])
            self.assertFalse(os.path.exists(os.path.join(apps, "Ollama.app")), kind)
            self.assertFalse(os.path.exists(os.path.join(apps, ".Ollama.app.partial")), kind)
            self.assertEqual(os.listdir(os.path.join(data, "downloads")), [], kind)
            self.assertFalse(state["opened"])

    def test_ollama_install_uses_the_users_own_ollama(self):
        for cli, app in (("/opt/homebrew/bin/ollama", None), (None, "/Applications/Ollama.app")):
            state, apps, hooks = self.ollama_world(cli=cli, app=app)
            lines = []
            setup.run_cli("ollama-install", "heavy", self.data, out=lines.append, **hooks)
            end = json.loads(lines[-1])
            self.assertFalse(end["ready"])
            self.assertIn("Start your Ollama", end["error"])
            self.assertIn("ollama serve", end["error"])
            self.assertEqual((state["fetched"], state["calls"]), (0, []))  # nothing downloaded, installed or opened

    def test_download_short_or_broken_leaves_nothing(self):
        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                self.send_response(200)
                self.send_header("Content-Length", "100000")
                self.end_headers()
                self.wfile.write(b"x" * 10)  # then the connection closes early
        srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)
        dest = os.path.join(self.data, "downloads", "thing.tar.gz")
        with mock.patch.dict(os.environ, {"http_proxy": "", "HTTP_PROXY": ""}):
            with self.assertRaises(setup.SetupError):
                setup.download("http://127.0.0.1:%d/thing" % srv.server_address[1], dest, timeout=5)
        self.assertEqual(os.listdir(os.path.dirname(dest)), [])

    def test_ollama_pull_progress_and_check(self):
        state = {"models": []}

        def get(url):
            if url.endswith("/api/version"):
                return {"version": "0.9"}
            if url.endswith("/api/tags"):
                return {"models": [{"name": n} for n in state["models"]]}

        def pull(url, model):
            yield {"status": "pulling manifest"}
            yield {"status": "downloading", "total": 100, "completed": 50}
            yield {"status": "downloading", "total": 100, "completed": 100}
            state["models"].append(model + ":latest")
            yield {"status": "success"}

        lines = []
        setup.run_cli("ollama-pull", "heavy", self.data, out=lines.append, get=get, pull=pull)
        rows = [json.loads(l) for l in lines]
        self.assertIn({"step": "ollama-pull: mxbai-embed-large", "fraction": 0.5}, rows)
        self.assertEqual(rows[-1], {"ready": True, "missing": [], "error": ""})
        st = setup.check("heavy", self.data, get=get, present=lambda *a: False)
        self.assertEqual(st, {"models": False, "ollama": "running", "ollama_model": True, "qdrant_binary": False})
        down = setup.check("light", self.data, get=lambda url: None, present=lambda *a: True)
        self.assertTrue(down["models"])
        self.assertIn(down["ollama"], ("missing", "stopped"))
        self.assertFalse(down["ollama_model"])
        lines = []
        setup.run_cli("ollama-pull", "heavy", self.data, out=lines.append, get=lambda url: None,
                      pull=lambda *a: self.fail("pulled without Ollama"))
        self.assertFalse(json.loads(lines[-1])["ready"])

    def test_cli_registered(self):
        import rbx
        with mock.patch("sys.argv", ["rbx"]):
            import argparse
            ap = argparse.ArgumentParser()
            sub = ap.add_subparsers(dest="cmd")
            import rbxrag.cli
            rbxrag.cli.register(sub)
            a = ap.parse_args(["rag", "check", "--mode", "light", "--data", self.data, "--json"])
            self.assertTrue(a.json)
        with open(rbx.__file__, encoding="utf-8") as f:
            self.assertIn('"rbxrag.cli": "rag"', f.read())


if __name__ == "__main__":
    unittest.main()
