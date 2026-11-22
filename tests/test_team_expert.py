"""rbx_team spawn with an expert from the Studio roster: the request carries what the app needs."""
import glob
import json
import os
import shutil
import tempfile
import threading
import unittest
from unittest import mock

import rbxteam
from rbxpaths import KitError


class ExpertSpawnTests(unittest.TestCase):
    TASK = "Build the shop menu in StarterGui.ShopGui: a grid of six items, buy buttons, and a close button. Report what you made."

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="team é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.project = os.path.join(self.tmp, "My Gäme")
        os.makedirs(self.project)
        for p in (mock.patch.object(rbxteam, "REQUESTS", os.path.join(self.tmp, "req")),
                  mock.patch.object(rbxteam, "RESULTS", os.path.join(self.tmp, "res")),
                  mock.patch.object(rbxteam, "PICKUP", 0.6),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": self.project, "RBXKIT_AGENT_ROLE": "",
                                               "RBXKIT_AGENT_ID": "c1"})):
            p.start()
            self.addCleanup(p.stop)

    def requests(self):
        out = []
        for path in glob.glob(os.path.join(rbxteam.REQUESTS, "*.json")):
            with open(path, encoding="utf-8") as f:
                out.append(json.load(f))
        return out

    def test_spawn_writes_expert_fields(self):
        import time
        t = threading.Thread(target=lambda: rbxteam.spawn("worker", "Shop UI", self.TASK, expert="coder"))
        t.start()
        req = None
        for _ in range(40):
            r = self.requests()
            if r:
                req = r[0]
                break
            time.sleep(0.01)
        t.join(5)
        self.assertEqual(req["expert"], "coder")
        self.assertEqual(req["expert_name"], "Coder")
        self.assertTrue(req["expert_text"].strip())
        self.assertIs(req["writes"], True)
        self.assertEqual(req["role"], "worker")
        self.assertEqual(req["name"], "Shop UI")

    def catch(self, expert, role="worker"):
        import time
        t = threading.Thread(target=lambda: rbxteam.spawn(role, "Check", self.TASK, expert=expert))
        t.start()
        req = None
        for _ in range(40):
            r = self.requests()
            if r:
                req = r[0]
                break
            time.sleep(0.01)
        t.join(5)
        return req

    def test_critic_expert_sets_role(self):
        req = self.catch("security", role="worker")  # the roster decides, not the caller
        self.assertEqual(req["role"], "critic")
        self.assertIs(req["writes"], False)

    def test_no_expert_request_is_unchanged(self):
        import time
        t = threading.Thread(target=lambda: rbxteam.spawn("worker", "Shop UI", self.TASK))
        t.start()
        time.sleep(0.2)
        req = self.requests()[0]
        t.join(5)
        self.assertEqual(sorted(req), ["created", "from", "id", "name", "role", "task"])

    def test_unknown_expert_refused(self):
        with self.assertRaises(KitError) as cm:
            rbxteam.spawn("worker", "X", self.TASK, expert="wizard")
        self.assertIn("coder", str(cm.exception))
        self.assertEqual(self.requests(), [])

    def test_lead_refused(self):
        with self.assertRaises(KitError) as cm:
            rbxteam.spawn("worker", "X", self.TASK, expert="lead")
        self.assertIn("Lead", str(cm.exception))
        self.assertEqual(self.requests(), [])

    def test_helper_with_expert_still_refused(self):
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": "worker"}):
            with self.assertRaises(KitError):
                rbxteam.spawn("worker", "X", self.TASK, expert="coder")
        self.assertEqual(self.requests(), [])

    def test_run_passes_expert_and_shows_name(self):
        with mock.patch.object(rbxteam, "spawn", return_value=("w1", {"status": "running", "role": "worker", "name": "Coder: Shop"})) as sp:
            text = rbxteam.run("spawn", {"role": "worker", "name": "Shop", "task": self.TASK, "expert": "coder"})
        self.assertEqual(sp.call_args.kwargs.get("expert"), "coder")
        self.assertIn("Coder: Shop", text)

    def test_spawn_passes_the_ai_and_model_asked_for(self):
        import time
        t = threading.Thread(target=lambda: rbxteam.spawn("worker", "Lava model", self.TASK, expert="world", ai=" Cursor ", model="auto"))
        t.start()
        req = None
        for _ in range(40):
            r = self.requests()
            if r:
                req = r[0]
                break
            time.sleep(0.01)
        t.join(5)
        self.assertEqual((req["ai"], req["model"]), ("cursor", "auto"))

    def test_run_passes_ai_and_shows_where_it_runs(self):
        res = {"status": "running", "role": "worker", "name": "World builder: Lava", "ai": "cursor", "model": "auto",
               "note": "Cursor does not offer the model \"x\", so this expert runs on Cursor's default (auto)."}
        with mock.patch.object(rbxteam, "spawn", return_value=("w1", res)) as sp:
            text = rbxteam.run("spawn", {"role": "worker", "name": "Lava", "task": self.TASK, "expert": "world", "ai": "cursor", "model": "x"})
        self.assertEqual((sp.call_args.kwargs.get("ai"), sp.call_args.kwargs.get("model")), ("cursor", "x"))
        self.assertIn("runs on: cursor (model auto)", text)
        self.assertIn("note: Cursor does not offer", text)
        with mock.patch.object(rbxteam, "spawn", return_value=("w2", {"status": "refused", "reason": "Gemini is not installed; run this expert on Claude (leave ai out) or ask the user."})):
            text = rbxteam.run("spawn", {"role": "worker", "name": "Lava", "task": self.TASK, "ai": "gemini"})
        self.assertIn("REFUSED: Gemini is not installed", text)


class LeadWaitTests(unittest.TestCase):
    """rbx_team wait hands a studio run's Lead back early (live run L4), and refuses experts on an ended run (L2)."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="wait é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.project = os.path.join(self.tmp, "Gäme")
        os.makedirs(os.path.join(self.project, ".fridae"))
        for p in (mock.patch.object(rbxteam, "RESULTS", os.path.join(self.tmp, "res")),
                  mock.patch.object(rbxteam, "REQUESTS", os.path.join(self.tmp, "req")),
                  mock.patch.object(rbxteam, "NUDGES", os.path.join(self.tmp, "nudge")),
                  mock.patch.object(rbxteam, "PICKUP", 0.3),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": self.project, "RBXKIT_AGENT_ROLE": "",
                                               "RBXKIT_AGENT_ID": "lead1"})):
            p.start()
            self.addCleanup(p.stop)
        os.makedirs(rbxteam.RESULTS)
        self.result("w1", "running")

    def result(self, rid, status):
        with open(os.path.join(rbxteam.RESULTS, rid + ".json"), "w", encoding="utf-8") as f:
            json.dump({"id": rid, "status": status, "role": "worker", "name": "Design plan", "text": "ok"}, f)

    def run_file(self, **run):
        with open(os.path.join(self.project, ".fridae", "studio.json"), "w", encoding="utf-8") as f:
            json.dump(dict({"on": True, "lead": "lead1", "stage": "design"}, **run), f)

    def wait(self, timeout=3):
        import time
        t = time.time()
        out = rbxteam.run("wait", {"ids": ["w1"], "timeout": timeout})
        return out, time.time() - t

    def test_no_run_waits_as_before(self):
        out, took = self.wait(1)
        self.assertGreaterEqual(took, 0.9)
        self.assertIn("Still not finished after", out)

    def test_another_chats_run_waits_as_before(self):
        self.run_file(lead="someone-else", paused=True)
        out, took = self.wait(1)
        self.assertGreaterEqual(took, 0.9)
        self.assertNotIn("Returned early", out)

    def test_paused_run_returns_at_once(self):
        self.run_file(paused=True)
        out, took = self.wait()
        self.assertLess(took, 1.5)
        self.assertIn("Returned early", out)
        self.assertIn("paused", out)
        self.assertIn("Still running: w1", out)
        self.assertIn("have not failed", out)

    def test_run_ended_during_the_wait_returns(self):
        import time
        self.run_file()
        threading.Timer(0.6, lambda: self.run_file(on=False, ended=time.time())).start()
        out, took = self.wait()
        self.assertLess(took, 2)
        self.assertIn("ended in Review", out)

    def test_run_ended_before_the_wait_is_no_run(self):
        import time
        self.run_file(on=False, ended=time.time() - 60)
        with mock.patch.object(rbxteam, "LEAD_WAIT", 0.5):
            out, took = self.wait(1)
        self.assertGreaterEqual(took, 0.9, "it waits on its helpers like any chat: no early return, no slices")
        self.assertNotIn("Returned early", out)

    def test_nudge_returns_and_is_taken(self):
        self.run_file()
        os.makedirs(rbxteam.NUDGES)
        with open(os.path.join(rbxteam.NUDGES, "lead1"), "w", encoding="utf-8") as f:
            json.dump({"typed": 1, "wakes": 0}, f)
        out, took = self.wait()
        self.assertLess(took, 1.5)
        self.assertIn("the user sent you a message", out)
        self.assertFalse(os.path.exists(os.path.join(rbxteam.NUDGES, "lead1")), "taken, so the next wait does not return for it")
        with open(os.path.join(rbxteam.NUDGES, "lead1"), "w", encoding="utf-8") as f:
            json.dump({"typed": 0, "wakes": 1}, f)
        out, _ = self.wait()
        self.assertIn("rbx_stage action 'decisions'", out)

    def test_lead_waits_in_slices(self):
        self.run_file()
        with mock.patch.object(rbxteam, "LEAD_WAIT", 1):
            out, took = self.wait(5)
        self.assertLess(took, 2.5)
        self.assertIn("Returned early", out)
        self.assertIn("decisions", out)

    def test_finished_tasks_are_handed_back_normally(self):
        self.run_file(paused=True)
        self.result("w1", "done")
        out, _ = self.wait()
        self.assertIn("All finished", out)
        self.assertNotIn("Returned early", out)

    def test_ended_run_refuses_experts(self):
        import time
        self.run_file(on=False, ended=time.time())
        with self.assertRaises(KitError) as e:
            rbxteam.spawn("worker", "More", ExpertSpawnTests.TASK, expert="coder")
        self.assertIn("This game's studio run was ended.", str(e.exception))
        self.assertEqual(os.listdir(self.tmp).count("req"), 0, "no request was written")
        # a plain helper (no expert) is not part of the run, and an ended run of another chat refuses nothing
        self.run_file(on=False, ended=time.time(), lead="someone-else")
        rid, res = rbxteam.spawn("worker", "More", ExpertSpawnTests.TASK, expert="coder")
        self.assertIsNone(res, "nobody picks it up in a test, but it was asked for")


if __name__ == "__main__":
    unittest.main()
