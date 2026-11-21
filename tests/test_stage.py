"""rbx_stage: the run file, review items with media, decisions and lessons."""
import json
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxkit_mcp
import rbxpaths
import rbxstage
from rbxpaths import KitError


class StageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="stage é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.project = os.path.join(self.tmp, "My Gäme")
        os.makedirs(self.project)
        self.a = {"project": self.project}
        env = mock.patch.dict(os.environ, {"RBXKIT_PROJECT": ""})
        env.start()
        self.addCleanup(env.stop)

    def call(self, action, **kw):
        return rbxstage.run(action, dict(self.a, **kw))

    def review_dir(self):
        return os.path.join(self.project, ".fridae", "review")

    def item(self, item_id):
        with open(os.path.join(self.review_dir(), item_id + ".json"), encoding="utf-8") as f:
            return json.load(f)

    def write_item(self, item):
        with open(os.path.join(self.review_dir(), item["id"] + ".json"), "w", encoding="utf-8") as f:
            json.dump(item, f)

    def media(self, name, size=10):
        path = os.path.join(self.tmp, name)
        with open(path, "wb") as f:
            f.write(b"x" * size)
        return path

    def filed_id(self, text):
        return text.split("[", 1)[1].split("]", 1)[0]

    def test_status_on_fresh_project(self):
        text = self.call("status")
        self.assertIn("no run", text.lower())
        for stage in rbxstage.STAGES:
            self.assertIn(stage, text)

    def test_stage_start_and_done(self):
        self.call("stage", id="design", status="start")
        run = rbxstage.load_run(self.project)
        self.assertEqual(run["stage"], "design")
        self.assertEqual([s["status"] for s in run["stages"] if s["id"] == "design"], ["doing"])
        with self.assertRaises(KitError) as ctx:
            self.call("stage", id="design", status="done")
        self.assertIn("approve", str(ctx.exception))
        rid = self.filed_id(self.call("review", title="Plan", summary="s", blocking=True))
        with self.assertRaises(KitError):
            self.call("stage", id="design", status="done")
        item = self.item(rid)
        item["status"] = "approved"
        self.write_item(item)
        self.call("stage", id="design", status="done")
        stage = [s for s in rbxstage.load_run(self.project)["stages"] if s["id"] == "design"][0]
        self.assertEqual(stage["status"], "done")
        self.assertTrue(stage["ended"])

    def test_ungated_stage_done_needs_no_approval(self):
        self.call("stage", id="build", status="start")
        self.call("stage", id="build", status="done")

    def test_unknown_stage(self):
        with self.assertRaises(KitError):
            self.call("stage", id="nope", status="start")

    def test_review_copies_media(self):
        rid = self.filed_id(self.call("review", title="Shop", summary="Looks", media=[self.media("a.png"), self.media("b.mp4")]))
        item = self.item(rid)
        self.assertEqual(item["media"], ["a.png", "b.mp4"])
        self.assertEqual(item["status"], "waiting")
        for name in item["media"]:
            self.assertTrue(os.path.isfile(os.path.join(self.review_dir(), rid, name)))

    def test_review_bad_media(self):
        good = [self.media("g%d.png" % i) for i in range(9)]
        missing = os.path.join(self.tmp, "gone.png")
        exe = self.media("run.exe")
        text = self.call("review", title="T", summary="S", media=[missing, exe] + good)
        rid = self.filed_id(text)
        item = self.item(rid)
        self.assertEqual(len(item["media"]), 8)
        self.assertIn("gone.png", text)
        self.assertIn("run.exe", text)
        self.assertIn("g8.png", text)  # the 9th good file is over the limit
        self.assertEqual(sorted(os.listdir(os.path.join(self.review_dir(), rid))), sorted(item["media"]))
        self.assertNotIn("run.exe", item["media"])

    def test_review_too_big(self):
        path = self.media("big.mp4")
        real = os.stat
        class S:  # pretend the file is over 200 MB without writing it
            st_size = 201 * 1024 * 1024
            st_mode = real(path).st_mode
        with mock.patch("rbxstage.os.stat", lambda p, *a, **k: S if p == path else real(p, *a, **k)):
            text = self.call("review", title="T", summary="S", media=[path])
        self.assertIn("big.mp4", text)
        self.assertEqual(self.item(self.filed_id(text))["media"], [])

    def test_review_replaces(self):
        old = self.filed_id(self.call("review", title="v1", summary="s"))
        new = self.filed_id(self.call("review", title="v2", summary="s", replaces=old))
        self.assertEqual(self.item(old)["status"], "superseded")
        self.assertEqual(self.item(new)["replaces"], old)
        with self.assertRaises(KitError):
            self.call("review", title="v3", summary="s", replaces="../x")

    def test_decisions_once(self):
        rid = self.filed_id(self.call("review", title="Coin art", summary="s"))
        self.assertIn("No new decisions", self.call("decisions"))
        item = self.item(rid)
        item.update(status="changes", note="Make it gold")
        self.write_item(item)
        first = self.call("decisions")
        self.assertIn("Coin art", first)
        self.assertIn("Make it gold", first)
        self.assertIn("No new decisions", self.call("decisions"))

    def test_status_lists_open_items_and_decisions(self):
        rid = self.filed_id(self.call("review", title="Open one", summary="s"))
        self.assertIn("Open one", self.call("status"))
        item = self.item(rid)
        item["status"] = "approved"
        self.write_item(item)
        self.assertIn("Open one", self.call("status"))  # shown as an unread decision
        self.call("decisions")
        self.assertNotIn("Open one", self.call("status"))

    def test_lesson_scopes(self):
        work = os.path.join(self.tmp, "work")
        os.makedirs(work)
        with mock.patch.object(rbxpaths, "WORK", work):
            self.call("lesson", text="Keep menus small", scope="game")
            self.call("lesson", text="Test on phones", scope="all")
        with open(os.path.join(self.project, ".fridae", "lessons.md"), encoding="utf-8") as f:
            self.assertIn("Keep menus small", f.read())
        with open(os.path.join(work, "studio-lessons.md"), encoding="utf-8") as f:
            self.assertIn("Test on phones", f.read())

    def test_no_project(self):
        with self.assertRaises(KitError):
            rbxstage.run("status", {})

    def test_no_temp_files_left(self):
        self.call("review", title="T", summary="S")
        self.call("stage", id="idea", status="start")
        names = os.listdir(os.path.join(self.project, ".fridae"))
        self.assertFalse([n for n in names if n.endswith(".tmp")])

    def test_mcp_tool(self):
        res = rbxkit_mcp.handle({"method": "tools/call", "params": {"name": "rbx_stage", "arguments": dict(self.a, action="status")}})
        self.assertIn("no run", res["content"][0]["text"].lower())


if __name__ == "__main__":
    unittest.main()
