"""rbx_stage: the run file, review items with media, decisions and lessons."""
import json
import os
import shutil
import tempfile
import threading
import time
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
        # a finished stage asks the app for a checkpoint in the work folder: never the real one in tests
        self.work = os.path.join(self.tmp, "work é")
        work = mock.patch.object(rbxpaths, "WORK", self.work)
        work.start()
        self.addCleanup(work.stop)

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

    def test_status_shows_experts_at_once_and_gates(self):
        text = self.call("status")
        self.assertIn("Experts at once (max_experts): 2. Gates (the run waits for you to approve): design, launch", text)
        os.makedirs(os.path.join(self.project, ".fridae"), exist_ok=True)
        with open(rbxstage.run_file(self.project), "w", encoding="utf-8") as f:
            json.dump({"on": True, "max_experts": 3, "gates": ["build"]}, f)
        text = self.call("status")
        self.assertIn("Experts at once (max_experts): 3. Gates (the run waits for you to approve): build", text)

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

    def test_stage_done_requests_checkpoint(self):
        requests = os.path.join(self.work, "stage-requests")
        self.call("stage", id="build", status="start")
        self.assertFalse(os.path.isdir(requests) and os.listdir(requests), "starting a stage asks for nothing")
        os.makedirs(os.path.join(self.project, ".fridae"), exist_ok=True)
        run = rbxstage.load_run(self.project)
        run["lead"] = "c-lead"
        rbxstage.save_run(self.project, run)
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ID": "c-other"}):
            self.assertEqual(self.call("stage", id="build", status="done"), "Stage build done.")
        names = os.listdir(requests)
        self.assertEqual(len(names), 1)
        self.assertTrue(names[0].endswith(".json"))
        with open(os.path.join(requests, names[0]), encoding="utf-8") as f:
            req = json.load(f)
        self.assertEqual((req["project"], req["stage"], req["lead"]), (self.project, "build", "c-lead"))
        # a gate that does not pass asks for no checkpoint, and the stage is not done
        self.call("stage", id="design", status="start")
        with self.assertRaises(KitError):
            self.call("stage", id="design", status="done")
        self.assertEqual(len(os.listdir(requests)), 1)
        # with no work folder to write to, the stage is still done
        with mock.patch.object(rbxpaths, "WORK", os.path.join(self.project, "GAME.md", "x")):
            open(os.path.join(self.project, "GAME.md"), "w").close()
            self.call("stage", id="test", status="start")
            self.assertEqual(self.call("stage", id="test", status="done"), "Stage test done.")
        self.assertEqual([s["status"] for s in rbxstage.load_run(self.project)["stages"] if s["id"] == "test"], ["done"])

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

    def test_supersede_keeps_a_decision(self):
        old = self.filed_id(self.call("review", title="v1", summary="s", blocking=True))
        item = self.item(old)
        item["status"] = "approved"
        self.write_item(item)
        text = self.call("review", title="v2", summary="s", replaces=old)
        self.assertEqual(self.item(old)["status"], "approved")
        self.assertIn("already approved", text)
        self.assertIn("v1", self.call("decisions"))

    def test_replaces_unknown_id(self):
        with self.assertRaises(KitError):
            self.call("review", title="v2", summary="s", replaces="deadbeef")

    def test_changes_then_approved_delivers_twice(self):
        rid = self.filed_id(self.call("review", title="Art", summary="s"))
        item = self.item(rid)
        item.update(status="changes", note="bigger")
        self.write_item(item)
        self.assertIn("changes wanted", self.call("decisions"))
        item.update(status="approved", note="")
        self.write_item(item)
        self.assertIn("approved", self.call("decisions"))
        self.assertIn("No new decisions", self.call("decisions"))

    def test_concurrent_decisions_exactly_once(self):
        ids = []
        for n in range(5):
            rid = self.filed_id(self.call("review", title="T%d" % n, summary="s"))
            item = self.item(rid)
            item["status"] = "approved"
            self.write_item(item)
            ids.append(rid)
        out = []
        threads = [threading.Thread(target=lambda: out.append(self.call("decisions"))) for _ in range(4)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        joined = "\n".join(out)
        for rid in ids:
            self.assertEqual(joined.count("[%s]" % rid), 1)

    def test_concurrent_lessons_lose_none(self):
        threads = [threading.Thread(target=self.call, args=("lesson",), kwargs={"text": "lesson %d" % n}) for n in range(20)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        with open(os.path.join(self.project, ".fridae", "lessons.md"), encoding="utf-8") as f:
            text = f.read()
        for n in range(20):
            self.assertIn("lesson %d\n" % n, text)

    def test_gate_refuses_while_another_blocking_item_waits(self):
        self.call("stage", id="design", status="start")
        first = self.filed_id(self.call("review", title="A", summary="s", blocking=True))
        self.call("review", title="B", summary="s", blocking=True)
        item = self.item(first)
        item["status"] = "approved"
        self.write_item(item)
        with self.assertRaises(KitError):
            self.call("stage", id="design", status="done")

    def test_status_shows_change_notes_and_which_were_delivered(self):
        a = self.filed_id(self.call("review", title="Coin art", summary="s"))
        b = self.filed_id(self.call("review", title="Shop menu", summary="s"))
        for rid, note in ((a, "Make it gold"), (b, "Bigger buttons é")):
            item = self.item(rid)
            item.update(status="changes", note=note)
            self.write_item(item)
        text = self.call("status")
        self.assertIn("Make it gold", text)
        self.assertIn("Bigger buttons é", text)
        self.assertNotIn("already handed over", text)
        self.call("decisions")
        # the note stays readable after 'decisions' handed it over, so a lost turn does not lose it
        text = self.call("status")
        line_a = [l for l in text.splitlines() if "[%s]" % a in l][0]
        self.assertIn("Make it gold", line_a)
        self.assertIn("already handed over", line_a)

    def test_gate_names_an_old_item_with_changes_and_says_replaces(self):
        self.call("stage", id="design", status="start")
        old = self.filed_id(self.call("review", title="Design doc", summary="s", blocking=True))
        item = self.item(old)
        item.update(status="changes", note="more levels")
        self.write_item(item)
        with self.assertRaises(KitError) as e:
            self.call("stage", id="design", status="done")
        self.assertIn("[%s]" % old, str(e.exception))
        self.assertIn("replaces=%s" % old, str(e.exception))

    def test_gate_newer_approved_item_supersedes_older_changes(self):
        self.call("stage", id="design", status="start")
        old = self.filed_id(self.call("review", title="Design v1", summary="s", blocking=True))
        item = self.item(old)
        item.update(status="changes", note="more levels")
        self.write_item(item)
        new = self.filed_id(self.call("review", title="Design v2", summary="s", blocking=True))  # replaces forgotten
        item = self.item(new)
        item["status"] = "approved"
        self.write_item(item)
        self.assertIn("done", self.call("stage", id="design", status="done"))
        self.assertEqual(self.item(old)["status"], "superseded")
        self.assertEqual(self.item(old)["superseded_by"], new)

    def test_an_ended_run_does_not_start_again(self):
        self.call("stage", id="idea", status="start")
        run = rbxstage.load_run(self.project)
        run.update(on=False, ended=time.time(), future_key=1)
        rbxstage.save_run(self.project, run)
        with self.assertRaises(KitError) as e:
            self.call("stage", id="design", status="start")
        self.assertIn("ended", str(e.exception))
        run = rbxstage.load_run(self.project)
        self.assertFalse(run["on"])
        self.assertEqual(run["future_key"], 1)
        self.assertIn("Run: off", self.call("status"))

    def test_lock_retries_while_windows_denies_access(self):
        os.makedirs(os.path.join(self.project, ".fridae"))
        real_open, calls = os.open, []

        def flaky(path, flags, *rest):
            if path == rbxstage.lock_file(self.project) and len(calls) < 2:
                calls.append(1)
                raise PermissionError(13, "Access is denied")  # a lock file pending deletion on Windows
            return real_open(path, flags, *rest)

        with mock.patch.object(rbxstage.os, "open", flaky):
            with rbxstage.locked(self.project):
                self.assertTrue(os.path.exists(rbxstage.lock_file(self.project)))
        self.assertEqual(len(calls), 2)
        self.assertFalse(os.path.exists(rbxstage.lock_file(self.project)))

    def test_lock_taken_over_when_stale_and_released(self):
        os.makedirs(os.path.join(self.project, ".fridae"))
        lock = rbxstage.lock_file(self.project)
        with open(lock, "w") as f:
            f.write("1 0")
        old = time.time() - 60
        os.utime(lock, (old, old))
        with rbxstage.locked(self.project):
            self.assertTrue(os.path.exists(lock))
        self.assertFalse(os.path.exists(lock))
        with self.assertRaises(RuntimeError):
            with rbxstage.locked(self.project):
                raise RuntimeError("boom")
        self.assertFalse(os.path.exists(lock))

    def test_mcp_tool(self):
        res = rbxkit_mcp.handle({"method": "tools/call", "params": {"name": "rbx_stage", "arguments": dict(self.a, action="status")}})
        self.assertIn("no run", res["content"][0]["text"].lower())


if __name__ == "__main__":
    unittest.main()
