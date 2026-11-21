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


if __name__ == "__main__":
    unittest.main()
