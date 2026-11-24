"""rbx_vault search through the knowledge index (RAG) service, falling back to the keyword search on any failure."""
import json
import os
import re
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
    with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": "", "FRIDAE_RAG_FILE": ""}):
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


def env(url="", token="", project="", raw=None):
    """FRIDAE_RAG_FILE pointing at an endpoint file {"url", "token"} (as the app writes it), or unset when no url."""
    path = ""
    if url or raw is not None:
        fd, path = tempfile.mkstemp(prefix="rag-endpoint-", suffix=".json", dir=_WORK)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            f.write(raw if raw is not None else json.dumps({"url": url, "token": token}))
    return mock.patch.dict(os.environ, {"FRIDAE_RAG_FILE": path, "RBXKIT_PROJECT": project})


class StubProxy:
    """An HTTP proxy stand-in that records anything sent to it (nothing should be)."""

    def __init__(self, tc):
        self.seen = []
        stub = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def handle_one(self):
                n = int(self.headers.get("Content-Length") or 0)
                stub.seen.append({"path": self.path, "headers": dict(self.headers),
                                  "body": self.rfile.read(n) if n else b""})
                self.send_response(502)
                self.send_header("Content-Length", "0")
                self.end_headers()

            do_GET = do_POST = do_CONNECT = handle_one

        self.srv = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.srv.daemon_threads = True
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        tc.addCleanup(self.srv.server_close)
        tc.addCleanup(self.srv.shutdown)
        self.url = "http://127.0.0.1:%d" % self.srv.server_address[1]

    def env(self):
        return mock.patch.dict(os.environ, {"http_proxy": self.url, "HTTP_PROXY": self.url, "https_proxy": self.url,
                                            "HTTPS_PROXY": self.url, "no_proxy": "", "NO_PROXY": ""})


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
        with env(), mock.patch.object(rbxvault, "_LOOPBACK") as op:
            self.assertEqual(rbxvault.run("search", a), want)
            op.open.assert_not_called()  # no service configured: no request at all
        stub = StubRag(self, guide_hits())
        for raw in ("not json", json.dumps({"url": stub.url}), json.dumps(["x"]),
                    json.dumps({"url": "http://example.com:80", "token": "tok"})):  # bad file, or not on this Mac
            with env(raw=raw):
                self.assertEqual(rbxvault.run("search", a), want, raw)
        with mock.patch.dict(os.environ, {"FRIDAE_RAG_FILE": os.path.join(_WORK, "missing.json")}):
            self.assertEqual(rbxvault.run("search", a), want)
        self.assertEqual(stub.seen, [])

    def test_endpoint_file_read_on_every_search(self):
        a = {"query": "how do I playtest", "shelf": "guides"}
        first, second = StubRag(self, guide_hits()), StubRag(self, guide_hits())
        with env(first.url, "one"):
            path = os.environ["FRIDAE_RAG_FILE"]
            self.assertIn("[guides]", rbxvault.run("search", a))
            with open(path, "w", encoding="utf-8") as f:  # the service restarted on another port with a new token
                json.dump({"url": second.url, "token": "two"}, f)
            self.assertIn("[guides]", rbxvault.run("search", a))
        self.assertEqual([x["auth"] for x in first.seen], ["Bearer one"])
        self.assertEqual([x["auth"] for x in second.seen], ["Bearer two"])

    def test_loopback_call_ignores_proxy_settings(self):
        proxy = StubProxy(self)
        stub = StubRag(self, guide_hits())
        with env(stub.url, "t0k"), proxy.env():
            out = rbxvault.run("search", {"query": "how do I playtest", "shelf": "guides"})
        self.assertIn("[guides]", out)
        self.assertEqual(len(stub.seen), 1)
        self.assertEqual(proxy.seen, [])  # neither the token nor the query went to the proxy

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


class PassageTests(unittest.TestCase):
    """An answer from the knowledge index shows the matched passage of each hit, within a cap per hit and per
    answer, instead of a snippet that sends the agent off to read the whole note."""

    def test_hits_show_their_passages_within_the_caps(self):
        hits = guide_hits()
        long_text = "Title › Heading\n" + "\n".join("Line %d says something about the zorblat caves." % i for i in range(80))
        hits = [dict(h, text=long_text) for h in hits]
        stub = StubRag(self, hits)
        with env(stub.url, "tok"):
            out = rbxvault.run("search", {"query": "zorblat caves", "shelf": "guides", "k": 2})
        self.assertIn("Line 0 says something about the zorblat caves.", out)
        self.assertNotIn("Title › Heading", out)  # the index's own first line is not part of the passage
        self.assertNotIn("Line 79", out)
        passages = [l for l in out.splitlines() if l.startswith("   Line")]
        self.assertLessEqual(sum(len(l) - 3 for l in passages), 2 * rbxvault.PASSAGE_CHARS)
        self.assertIn('"action": "read"', out)  # still the arguments to read more

    def test_passage_cap(self):
        lines = rbxvault.passage("T › H\n" + "word " * 1000, 200)
        self.assertLessEqual(sum(len(x) for x in lines), 200)
        self.assertTrue(lines[-1].endswith("…"))
        self.assertEqual(rbxvault.passage("T › H\n```lua\nlocal x = 1\n```\n\nPlain words.", 500),
                         ["local x = 1", "Plain words."])


class KeywordRankingTests(unittest.TestCase):
    """rbxvault.keyword_ranking is the keyword search's own order, which the knowledge index fuses."""

    def test_same_order_as_the_search_shows(self):
        with mock.patch.dict(os.environ, {"FRIDAE_RAG_FILE": ""}):
            for q in ("save player coins between sessions", "the zombie keeps walking into the wall"):
                out = rbxvault.run("search", {"query": q, "shelf": "all", "k": 5})
                lines = out.splitlines()
                heads = [l for l in lines if re.match(r"^\d+\. \[", l)]
                reads = [json.loads(l.split("read: ", 1)[1])["title"] for l in lines if l.strip().startswith("read: ")]
                self.assertEqual(len(heads), len(reads))
                # the direct results keep the keyword order; notes found through a link (at most two) come after the
                # first three and take the last places
                shown = [t for h, t in zip(heads, reads) if "(linked from " not in h]
                linked = [i for i, h in enumerate(heads) if "(linked from " in h]
                self.assertLessEqual(len(linked), rbxvault.LINKED_MAX)
                self.assertTrue(all(i >= rbxvault.DIRECT_KEPT for i in linked))
                ranked = rbxvault.keyword_ranking(q, [s for s in rbxvault.SHELVES if s != "game"], 5)
                self.assertEqual([t for _, t, _ in ranked][:len(shown)], shown)
                self.assertTrue(all((i is None) == (shelf == "vault") for shelf, _, i in ranked))
                vault = rbxvault.keyword_ranking(q, ["vault"], 4)
                self.assertEqual([t for _, t, _ in vault], [d["title"] for _, d in rbxvault.search(q, 4)[0]])

    def test_kept_statistics_rank_exactly_as_the_plain_search(self):
        idx = rbxvault.load_index()
        kept = dict(idx, docs=rbxvault.Docs(idx["docs"]))
        docs = rbxvault.section_corpus("")
        kept_docs = rbxvault.Docs(docs)
        shelves = [s for s in rbxvault.SHELVES if s != "game"]
        for q in ("save player coins between sessions", "the zombie keeps walking into the wall", "GetPropertyChangedSignal",
                  "make it feel softer", "price the first three upgrades", "remote event exploit"):
            self.assertEqual(rbxvault.search(q, 10, kept), rbxvault.search(q, 10, idx), q)
            for sh in (shelves, ["guides", "team"], ["vault", "guides"]):
                self.assertEqual(rbxvault.keyword_ranking(q, sh, 20, kept, kept_docs),
                                 rbxvault.keyword_ranking(q, sh, 20, idx, docs), (q, sh))
                self.assertEqual(rbxvault.search_sections(q, sh, 20, kept_docs), rbxvault.search_sections(q, sh, 20, docs))

    def test_the_service_ranker_keeps_its_indexes_and_takes_a_project(self):
        from rbxrag.engine import KeywordRanker
        now = [0.0]
        kr = KeywordRanker(every=10.0, clock=lambda: now[0], background=False)
        with mock.patch.object(rbxvault, "load_index", wraps=rbxvault.load_index) as li, \
                mock.patch.object(rbxvault, "note_files", wraps=rbxvault.note_files) as nf:
            a = kr("datastore session lock", ["vault", "guides"], 5, "")
            walks = nf.call_count
            kr("datastore session lock", ["vault", "guides"], 5, "")
            self.assertEqual((li.call_count, nf.call_count), (1, walks))  # within 10 s: not even a look
            now[0] = 11.0
            kr("datastore session lock", ["vault", "guides"], 5, "")
            self.assertEqual(nf.call_count, walks + 1)  # looked at the files again...
            self.assertEqual(li.call_count, 1)  # ...and kept the loaded index: nothing changed
        self.assertEqual(a, rbxvault.keyword_ranking("datastore session lock", ["vault", "guides"], 5))
        proj = tempfile.mkdtemp(prefix="kw game ")
        self.addCleanup(shutil.rmtree, proj, True)
        with open(os.path.join(proj, "GAME.md"), "w", encoding="utf-8") as f:
            f.write("# Glimmer\n\n## Pitch\n\nCollect glimmerstones in the zorblat caves.\n")
        self.assertEqual(kr("zorblat glimmerstones", ["game"], 3, proj)[0][:2], ("game", "GAME.md"))
        self.assertEqual(kr("zorblat glimmerstones", ["game"], 3, ""), [])


if __name__ == "__main__":
    unittest.main()
