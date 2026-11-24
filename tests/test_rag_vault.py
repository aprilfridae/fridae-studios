"""rbx_vault search through the knowledge index (RAG) service, falling back to the keyword search on any failure."""
import json
import os
import shutil
import socket
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest import mock

import rbxpaths
import rbxvault


def setUpModule():
    global _WORK, _PATCHES
    _WORK = tempfile.mkdtemp(prefix="rag vault é ")
    _PATCHES = [mock.patch.object(rbxpaths, "WORK", _WORK),
                mock.patch.object(rbxpaths, "HOME_DIR", os.path.join(_WORK, "home"))]
    for p in _PATCHES:
        p.start()
    with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "FRIDAE_RAG_URL": "", "FRIDAE_RAG_TOKEN": ""}):
        rbxvault.run("search", {"query": "warm the caches"})  # the timing test measures the fallback, not a cold cache


def tearDownModule():
    for p in _PATCHES:
        p.stop()
    shutil.rmtree(_WORK, True)


class StubRag:
    """A stand-in RAG service on 127.0.0.1: answers /query with the given hits after an optional delay."""

    def __init__(self, tc, hits, delay=0.0, code=200):
        self.seen = []
        stub = self
        self.hits, self.delay, self.code = hits, delay, code

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get("Content-Length") or 0)) or b"{}")
                stub.seen.append({"path": self.path, "auth": self.headers.get("Authorization"), "body": body})
                if stub.delay:
                    time.sleep(stub.delay)
                raw = json.dumps({"hits": stub.hits}).encode()
                try:
                    self.send_response(stub.code)
                    self.send_header("Content-Type", "application/json")
                    self.send_header("Content-Length", str(len(raw)))
                    self.end_headers()
                    self.wfile.write(raw)
                except OSError:
                    pass

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.srv.daemon_threads = True
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        tc.addCleanup(self.srv.server_close)
        tc.addCleanup(self.srv.shutdown)
        self.url = "http://127.0.0.1:%d" % self.srv.server_address[1]


def hit_for(shelf, title, path, heading):
    p = rbxvault.parse_sections(rbxvault.read(path))
    i = next(n for n, s in enumerate(p["sections"]) if s["heading"] == heading)
    return {"id": "x", "shelf": shelf, "title": title, "heading": heading, "path": path, "rel": path, "i": i,
            "part": 0, "text": "%s › %s\n..." % (title, heading), "project": "", "score": 1.0}


def guide_hits():
    out = []
    for title, path in rbxvault.guide_files():
        secs = [s for s in rbxvault.parse_sections(rbxvault.read(path))["sections"] if s["level"] == 2]
        if secs:
            out.append(hit_for("guides", title, path, secs[0]["heading"]))
        if len(out) == 2:
            return out
    return out


def closed_port():
    s = socket.socket()
    s.bind(("127.0.0.1", 0))
    port = s.getsockname()[1]
    s.close()
    return port


def env(url="", token="", project=""):
    return mock.patch.dict(os.environ, {"FRIDAE_RAG_URL": url, "FRIDAE_RAG_TOKEN": token, "RBXKIT_PROJECT": project})


class RagVaultTests(unittest.TestCase):
    def test_search_uses_rag_when_up(self):
        hits = guide_hits()
        self.assertEqual(len(hits), 2)
        stub = StubRag(self, hits)
        with env(stub.url, "tok"):
            out = rbxvault.run("search", {"query": "how do I playtest", "shelf": "all", "k": 4})
        self.assertIn("1. [guides] %s" % hits[0]["title"], out)
        self.assertIn("2. [guides] %s" % hits[1]["title"], out)
        reads = [json.loads(l.split("read: ", 1)[1]) for l in out.splitlines() if l.strip().startswith("read: ")]
        self.assertEqual(reads[0], {"action": "read", "shelf": "guides", "title": hits[0]["title"],
                                    "heading": hits[0]["heading"]})
        self.assertTrue(out.startswith('Results for "how do I playtest" ('))
        self.assertIn("Next: rbx_vault action 'read'", out)
        body = stub.seen[0]["body"]
        self.assertEqual(stub.seen[0]["path"], "/query")
        self.assertEqual(body["q"], "how do I playtest")
        self.assertEqual(body["k"], 4)
        self.assertNotIn("game", body["shelves"])  # no project: no game shelf
        self.assertIn("vault", body["shelves"])
        # the read arguments really read that section
        text = rbxvault.run("read", reads[0])
        self.assertIn(hits[0]["heading"], text)

    def test_vault_only_search_through_rag_keeps_api_and_symptoms(self):
        note = next(p for p in rbxvault.note_files() if "Teleport" in os.path.basename(p))
        title = os.path.basename(note)[:-3]
        secs = [s for s in rbxvault.parse_sections(rbxvault.read(note))["sections"] if s["level"] == 2]
        stub = StubRag(self, [hit_for("vault", title, note, secs[0]["heading"])])
        with env(stub.url, "tok"):
            out = rbxvault.run("search", {"query": "TeleportService.TeleportPartyAsync between places", "shelf": "vault"})
        self.assertIn("1. [vault] %s" % title, out)
        self.assertIn("API status (snapshot):", out)
        self.assertIn("- TeleportService.TeleportPartyAsync: deprecated", out)
        self.assertIn("Symptoms (symptom | cause | fix | note):", out)
        self.assertEqual(stub.seen[0]["body"]["shelves"], ["vault"])
        read = json.loads(next(l for l in out.splitlines() if l.strip().startswith("read: ")).split("read: ", 1)[1])
        self.assertEqual(read["shelf"], "vault")
        self.assertIn(secs[0]["heading"].lower()[:10], rbxvault.run("read", read).lower())

    def test_search_falls_back_when_service_down(self):
        for shelf in ("all", "vault", "guides"):
            a = {"query": "datastore session lock", "shelf": shelf}
            with env():
                want = rbxvault.run("search", a)
            with env("http://127.0.0.1:%d" % closed_port(), "tok"):
                self.assertEqual(rbxvault.run("search", a), want, shelf)

    def test_search_falls_back_on_slow_service(self):
        stub = StubRag(self, guide_hits(), delay=4.0)
        a = {"query": "leaderboard ordered datastore", "shelf": "all"}
        with env():
            t = time.monotonic()
            want = rbxvault.run("search", a)
            keyword = time.monotonic() - t  # the keyword search's own time, which the fallback adds to the wait
        with env(stub.url, "tok"):
            t = time.monotonic()
            got = rbxvault.run("search", a)
            took = time.monotonic() - t
        self.assertEqual(got, want)
        self.assertLess(took, 4.0)  # never waits for the 4 s answer
        self.assertLess(took - keyword, 3.5)

    def test_search_falls_back_on_errors_and_no_hits(self):
        a = {"query": "datastore session lock", "shelf": "all"}
        with env():
            want = rbxvault.run("search", a)
        for stub in (StubRag(self, []), StubRag(self, guide_hits(), code=500)):
            with env(stub.url, "tok"):
                self.assertEqual(rbxvault.run("search", a), want)
        with env("", "tok"), mock.patch("urllib.request.urlopen") as op:
            self.assertEqual(rbxvault.run("search", a), want)
            op.assert_not_called()  # no service configured: no request at all

    def test_token_sent_never_printed(self):
        stub = StubRag(self, guide_hits())
        with env(stub.url, "t0k"):
            out = rbxvault.run("search", {"query": "t0k playtest", "shelf": "guides"})
        self.assertEqual(stub.seen[0]["auth"], "Bearer t0k")
        self.assertNotIn("Bearer", out)
        self.assertNotIn("t0k", out.replace('"t0k playtest"', ""))

    def test_stale_hit_is_skipped(self):
        hits = guide_hits()
        hits[0] = dict(hits[0], heading="A heading that is no longer there", i=999)
        stub = StubRag(self, hits)
        with env(stub.url, "tok"):
            out = rbxvault.run("search", {"query": "playtest", "shelf": "guides"})
        self.assertNotIn("no longer there", out)
        self.assertIn("1. [guides] %s" % hits[1]["title"], out)


if __name__ == "__main__":
    unittest.main()
