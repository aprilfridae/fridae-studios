"""rbx lookup: the swarm Librarian's search, every part of a question at once (rbxvault.lookup)."""
import io
import json
import os
import shutil
import tempfile
import threading
import unittest
from contextlib import redirect_stdout
from unittest import mock

import rbx
import rbxpaths
import rbxvault
from tests.test_rag_vault import StubRag, closed_port, guide_hits

PARTS = ["past lessons on ProximityPrompts interfering with touch throw drags",
         "phone safe-zone rules around the HUD",
         "part and emitter budget rules for ambient VFX"]


def env(url="", token=""):
    """FRIDAE_RAG_FILE pointing at an endpoint file as the app writes it (none without a url), and no game."""
    path = ""
    if url:
        fd, path = tempfile.mkstemp(prefix="rag-endpoint-", suffix=".json", dir=_WORK)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump({"url": url, "token": token}, f)
    return mock.patch.dict(os.environ, {"FRIDAE_RAG_FILE": path, "RBXKIT_PROJECT": ""})


def setUpModule():
    global _WORK, _PATCHES
    _WORK = tempfile.mkdtemp(prefix="librarian lookup ")
    _PATCHES = [mock.patch.object(rbxpaths, "WORK", _WORK),
                mock.patch.object(rbxpaths, "HOME_DIR", os.path.join(_WORK, "home"))]
    for p in _PATCHES:
        p.start()


def tearDownModule():
    for p in _PATCHES:
        p.stop()
    shutil.rmtree(_WORK, True)


class LookupTests(unittest.TestCase):
    def test_each_part_is_searched_once_and_all_at_the_same_time(self):
        seen, lock = [], threading.Lock()
        together = threading.Barrier(len(PARTS), timeout=5)  # breaks unless every part's search runs at once

        def one(q, shelves, k, end, corpora):
            with lock:
                seen.append(q)
            together.wait()
            return {"query": q, "via": "keyword", "hits": [{"shelf": "vault", "title": q[:10], "heading": "", "text": "x"}]}

        with env(), mock.patch.object(rbxvault, "_lookup_one", side_effect=one):
            out = rbxvault.lookup(PARTS + ["  "], k=3)
        self.assertEqual(sorted(seen), sorted(PARTS))
        self.assertEqual([p["query"] for p in out], PARTS)  # in the order asked, blanks dropped
        self.assertEqual(rbxvault.lookup([], 3), [])

    def test_parts_are_capped_and_one_failing_part_keeps_the_others(self):
        def one(q, *a):
            if q == "q1":
                raise OSError("disk gone")
            return {"query": q, "via": "keyword", "hits": []}

        with env(), mock.patch.object(rbxvault, "_lookup_one", side_effect=one):
            out = rbxvault.lookup(["q%d" % i for i in range(9)])
        self.assertEqual(len(out), rbxvault.LOOKUP_MAX)
        self.assertEqual(out[1]["hits"], [])
        self.assertIn("disk gone", out[1]["error"])
        self.assertNotIn("error", out[0])

    def test_passages_come_from_the_knowledge_index_when_it_runs(self):
        hits = guide_hits()
        for h in hits:
            h["text"] = "%s › %s\nThe matching passage for %s." % (h["title"], h["heading"], h["title"])
        stub = StubRag(self, hits)
        with env(stub.url, "tok"):
            out = rbxvault.lookup(PARTS[:2], k=3)
        self.assertEqual(len(stub.seen), 2)  # one call to the index per part
        self.assertEqual(sorted(s["body"]["q"] for s in stub.seen), sorted(PARTS[:2]))
        for part in out:
            self.assertEqual(part["via"], "rag")
            self.assertEqual(part["hits"][0], {"shelf": "guides", "title": hits[0]["title"], "heading": hits[0]["heading"],
                                               "text": "The matching passage for %s." % hits[0]["title"]})

    def test_keyword_search_answers_when_the_index_is_down(self):
        with env("http://127.0.0.1:%d" % closed_port(), "tok"):
            out = rbxvault.lookup(["datastore session lock", "ProximityPrompt touch"], k=3)
        for part in out:
            self.assertEqual(part["via"], "keyword")
            self.assertTrue(part["hits"], part)
            self.assertLessEqual(len(part["hits"]), 3)
            for h in part["hits"]:
                self.assertTrue(h["title"] and h["text"], h)
                self.assertLessEqual(len(h["text"]), rbxvault.LOOKUP_CHARS)
                self.assertNotIn("tags:", h["text"])  # no front matter

    def test_the_command_reads_and_writes_json(self):
        fake = [{"query": "a", "via": "keyword", "hits": [{"title": "Señal"}]}]
        raw = io.BytesIO()
        out = io.TextIOWrapper(raw, encoding="utf-8")
        stdin = io.TextIOWrapper(io.BytesIO(json.dumps({"queries": ["a"], "k": 2}).encode()), encoding="utf-8")
        with mock.patch.object(rbxvault, "lookup", return_value=fake) as lk, mock.patch("sys.stdin", stdin), \
                redirect_stdout(out):
            rbx.main(["lookup"])
            out.flush()
        lk.assert_called_once_with(["a"], 2)
        self.assertEqual(json.loads(raw.getvalue().decode("utf-8")), {"parts": fake})
        with mock.patch("sys.stdin", io.StringIO("nope")), self.assertRaises(SystemExit):
            rbx.main(["lookup"])


if __name__ == "__main__":
    unittest.main()
