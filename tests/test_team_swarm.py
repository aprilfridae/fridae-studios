"""rbx_team's swarm actions: posting through the app, reading the board, waiting for a mention, the Studio turn,
and rbx_vault notes updated rather than duplicated (the Librarian's filing)."""
import json
import os
import shutil
import tempfile
import threading
import time
import unittest
from unittest import mock

import rbxteam


class SwarmBoardTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="swarm é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        swarm = os.path.join(self.tmp, "swarm")
        os.makedirs(os.path.join(swarm, "r1"))
        self.board = os.path.join(swarm, "r1", "board.jsonl")
        index = {"c1": {"run": "r1", "name": "shop-ui", "role": "coder", "lead": "L", "board": "/elsewhere", "is_lead": False},
                 "L": {"run": "r1", "name": "lead", "role": "lead", "lead": "L", "board": "", "is_lead": True}}
        with open(os.path.join(swarm, "members.json"), "w", encoding="utf-8") as f:
            json.dump(index, f)
        for p in (mock.patch.object(rbxteam, "REQUESTS", os.path.join(self.tmp, "req")),
                  mock.patch.object(rbxteam, "RESULTS", os.path.join(self.tmp, "res")),
                  mock.patch.object(rbxteam, "SWARM", swarm),
                  mock.patch.object(rbxteam, "INDEX", os.path.join(swarm, "members.json")),
                  mock.patch.object(rbxteam, "PICKUP", 2),
                  mock.patch.dict(os.environ, {"RBXKIT_AGENT_ID": "c1", "RBXKIT_AGENT_ROLE": "worker"})):
            p.start()
            self.addCleanup(p.stop)

    def add(self, **post):
        with open(self.board, "a", encoding="utf-8") as f:
            f.write(json.dumps(post) + "\n")

    def app(self, answer):
        """A stand-in for the app: answers the first request, and keeps it."""
        seen = []

        def run():
            for _ in range(100):
                for n in os.listdir(rbxteam.REQUESTS) if os.path.isdir(rbxteam.REQUESTS) else []:
                    if n.endswith(".json"):
                        path = os.path.join(rbxteam.REQUESTS, n)
                        with open(path, encoding="utf-8") as f:
                            req = json.load(f)
                        os.remove(path)
                        seen.append(req)
                        os.makedirs(rbxteam.RESULTS, exist_ok=True)
                        with open(os.path.join(rbxteam.RESULTS, n), "w", encoding="utf-8") as f:
                            json.dump(dict(answer, id=req["id"]), f)
                        return
                time.sleep(0.02)
        t = threading.Thread(target=run)
        t.start()
        self.addCleanup(t.join, 5)
        return seen

    def test_post_goes_through_the_app_and_its_result_is_removed(self):
        seen = self.app({"status": "done", "text": "Posted."})
        self.assertEqual(rbxteam.run("post", {"to": "@datastore", "text": "what's the profile key?"}), "Posted.")
        self.assertEqual(seen[0]["op"], "post")
        self.assertEqual(seen[0]["from"], "c1")
        self.assertEqual(seen[0]["to"], ["@datastore"])
        self.assertEqual(seen[0]["kind"], "msg")
        self.assertEqual(os.listdir(rbxteam.RESULTS), [], "a post's result is not left for 'status' to list")

    def test_a_refusal_is_an_error(self):
        self.app({"status": "refused", "reason": "A post's kind is msg, done or verify."})
        with self.assertRaises(rbxteam.TeamError) as e:
            rbxteam.run("post", {"to": [], "text": "x", "kind": "merged"})
        self.assertIn("msg, done or verify", str(e.exception))

    def test_board_marks_posts_for_me_trims_and_limits(self):
        self.add(t=1.0, **{"from": "lead"}, to=["@shop-ui"], kind="msg", text="start the shop")
        self.add(t=2.0, **{"from": "datastore"}, to=["@all"], kind="msg", text="x" * 1000)
        self.add(t=3.0, **{"from": "lead"}, to=["@tests"], kind="msg", text="write tests")
        out = rbxteam.run("board", {})
        self.assertIn("You are @shop-ui", out)
        self.assertIn("* [1.0] @lead -> @shop-ui (msg): start the shop", out)
        self.assertIn("  [3.0] @lead -> @tests", out)
        self.assertIn("x" * rbxteam.POST_SHOWN + " ...", out)
        self.assertNotIn("x" * (rbxteam.POST_SHOWN + 1), out)
        out = rbxteam.run("board", {"since": 1.0, "limit": 1})
        self.assertIn("1 older posts not shown", out)
        self.assertIn("write tests", out)
        self.assertNotIn("start the shop", out)

    def test_wait_board_returns_on_a_mention_only(self):
        def later():
            time.sleep(0.3)
            self.add(t=time.time(), **{"from": "lead"}, to=["@tests"], kind="msg", text="not you")
            time.sleep(0.3)
            self.add(t=time.time(), **{"from": "lead"}, to=["@coder"], kind="msg", text="all coders: stop")
        t = threading.Thread(target=later)
        t.start()
        out = rbxteam.run("wait_board", {"timeout": 5})
        t.join()
        self.assertIn("all coders: stop", out)
        self.assertNotIn("not you", out)
        self.assertIn("Nothing new", rbxteam.run("wait_board", {"timeout": 1}))

    def test_the_lead_hears_everything_but_itself(self):
        me = {"name": "lead", "role": "lead", "is_lead": True}
        self.assertTrue(rbxteam.for_me({"from": "tests", "to": ["@shop-ui"]}, me))
        self.assertFalse(rbxteam.for_me({"from": "lead", "to": ["@all"]}, me))
        member = {"name": "shop-ui", "role": "coder"}
        self.assertTrue(rbxteam.for_me({"from": "x", "to": ["all"]}, member))
        self.assertFalse(rbxteam.for_me({"from": "x", "to": ["@tests"]}, member))

    def test_not_in_a_swarm(self):
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ID": "stranger"}):
            with self.assertRaises(rbxteam.TeamError) as e:
                rbxteam.run("board", {})
        self.assertIn("not part of a swarm", str(e.exception))

    def test_studio_turn(self):
        with self.assertRaises(rbxteam.TeamError):
            rbxteam.run("studio_turn", {"op": "grab"})
        seen = self.app({"status": "done", "text": "You have the Studio turn."})
        self.assertIn("You have the Studio turn", rbxteam.run("studio_turn", {"op": "take"}))
        self.assertEqual((seen[0]["op"], seen[0]["turn"]), ("studio", "take"))

    def test_spawn_carries_the_member_handle(self):
        seen = self.app({"status": "running", "name": "Shop UI"})
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": ""}):
            rbxteam.spawn("worker", "Shop UI", "Build the shop menu: a grid of six items, buy buttons and a close button. "
                                               "Report what you made.", member="@shop-ui")
        self.assertEqual(seen[0]["member"], "@shop-ui")


class NoteReplaceTests(unittest.TestCase):
    def test_a_note_with_the_same_title_is_updated_not_duplicated(self):
        import rbxnotes
        root = tempfile.mkdtemp(prefix="notes ")
        self.addCleanup(shutil.rmtree, root, True)
        with mock.patch.object(rbxnotes, "folder", lambda scope, project=None: root):
            t1, p1 = rbxnotes.write_note("Shop buttons are round.", "Shop style", "game")
            t2, p2 = rbxnotes.write_note("Shop buttons are round and soft green.", "Shop style", "game", replace=True)
            t3, p3 = rbxnotes.write_note("Another.", "Shop style", "game")
        self.assertEqual((t1, p1), (t2, p2))
        with open(p1, encoding="utf-8") as f:
            self.assertIn("soft green", f.read())
        self.assertEqual(t3, "Shop style 2")


if __name__ == "__main__":
    unittest.main()
