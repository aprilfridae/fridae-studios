"""A dry run of a Studio run, the Python side only: the tool calls a Lead would make, with stub experts and no AI.

The Lead's calls go through rbxkit_mcp.handle (the same JSON-RPC entry point the AI uses). A small thread stands in for
the desktop app: it picks up rbx_team requests from the work folder and answers each with a finished report that names
a picture, as a real expert would. The user's decisions are written straight into the review item files, as the app's
Review tab does.
"""
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
import rbxteam
import rbxvault


class StubApp:
    """Answers rbx_team requests like the app would, with an expert that finishes at once."""

    def __init__(self, folder):
        self.folder = folder
        self.seen = []
        self.stop = threading.Event()
        self.thread = threading.Thread(target=self.loop, daemon=True)

    def loop(self):
        while not self.stop.is_set():
            try:
                names = sorted(os.listdir(rbxteam.REQUESTS))
            except OSError:
                names = []
            for n in names:
                if not n.endswith(".json"):
                    continue
                path = os.path.join(rbxteam.REQUESTS, n)
                req = rbxpaths.read_json(path)
                if not isinstance(req, dict):
                    continue  # still being written
                os.remove(path)
                self.seen.append(req)
                shot = os.path.join(self.folder, "%s shot é.png" % req["expert"])
                with open(shot, "wb") as f:
                    f.write(b"\x89PNG\r\n\x1a\n" + b"0" * 64)
                os.makedirs(rbxteam.RESULTS, exist_ok=True)
                rbxpaths.write_json(os.path.join(rbxteam.RESULTS, req["id"] + ".json"),
                                    {"id": req["id"], "agent": "c-" + req["expert"], "name": req["name"], "role": req["role"],
                                     "expert": req["expert"], "status": "done", "reason": "",
                                     "text": "%s finished: %s\nPicture: %s" % (req["expert_name"], req["name"], shot)})
            time.sleep(0.02)


class StudioFlowTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="studio flow é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.project = os.path.join(self.tmp, "Obby Gäme")
        os.makedirs(self.project)
        self.work = os.path.join(self.tmp, "work é")
        os.makedirs(self.work)
        self.home = os.path.join(self.tmp, "home é")  # lessons for every game are notes in the home folder
        for p in (mock.patch.object(rbxpaths, "WORK", self.work),
                  mock.patch.object(rbxpaths, "HOME_DIR", self.home),
                  mock.patch.object(rbxteam, "REQUESTS", os.path.join(self.work, "team-requests")),
                  mock.patch.object(rbxteam, "RESULTS", os.path.join(self.work, "team-results")),
                  mock.patch.object(rbxteam, "PICKUP", 5),
                  mock.patch.dict(os.environ, {"RBXKIT_PROJECT": self.project, "RBXKIT_AGENT_ROLE": "",
                                               "RBXKIT_AGENT_ID": "c-lead"})):
            p.start()
            self.addCleanup(p.stop)
        self.app = StubApp(self.tmp)
        self.app.thread.start()
        self.addCleanup(self.app.stop.set)

    # ---- the Lead's tool calls, through the MCP entry point
    def tool(self, _tool, ok=True, **args):
        res = rbxkit_mcp.handle({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                                 "params": {"name": _tool, "arguments": args}})
        text = res["content"][0]["text"]
        self.assertEqual(res["isError"], not ok, text)
        return text

    def stage(self, **args):
        return self.tool("rbx_stage", **args)

    def expert(self, expert_id, name):
        """Spawn a roster expert and collect its report, as the Lead does; returns the picture path it reported."""
        task = ("Goal: %s for a small obby with three checkpoints and a coin shop. Project: %s. Read PROJECT.md. "
                "Report what you did and the picture paths." % (name, self.project))
        text = self.tool("rbx_team", action="spawn", expert=expert_id, name=name, task=task)
        rid = text.split("id: ", 1)[1].split()[0]
        report = self.tool("rbx_team", action="wait", ids=[rid], timeout=5)
        self.assertIn("finished: " + name, report)
        return report.split("Picture: ", 1)[1].splitlines()[0].strip()

    def review(self, **args):
        text = self.stage(action="review", **args)
        return text.split("[", 1)[1].split("]", 1)[0]

    def decide(self, item_id, status, note=""):
        """What the app's Review tab writes when the user presses Approve or sends a Change."""
        path = os.path.join(self.project, ".fridae", "review", item_id + ".json")
        with open(path, encoding="utf-8") as f:
            item = json.load(f)
        item.update(status=status, note=note, decided=time.time())
        with open(path, "w", encoding="utf-8") as f:
            json.dump(item, f)

    def test_a_run_from_idea_to_build(self):
        # kickoff for a new game with an idea: idea done, design started
        self.assertIn("No run yet", self.stage(action="status"))
        self.stage(action="stage", id="idea", status="done")
        self.stage(action="stage", id="design", status="start")

        # the Designer writes the plan; the Lead files it as the design gate's blocking item
        shot = self.expert("designer", "Design plan")
        design = self.review(title="Design plan", summary="Three checkpoints, a coin shop.", media=[shot],
                             expert="designer", blocking=True)
        req = self.app.seen[0]
        self.assertEqual((req["expert"], req["expert_name"], req["role"], req["writes"]), ("designer", "Designer", "worker", False))
        self.assertTrue(req["expert_text"].strip())
        item = rbxstage.load_items(self.project)[0]
        self.assertEqual(item["media"], [os.path.basename(shot)])
        self.assertTrue(os.path.isfile(os.path.join(self.project, ".fridae", "review", design, os.path.basename(shot))))
        self.assertIn("[%s] Design plan (design, blocking, waiting)" % design, self.stage(action="status"))

        # the gate holds until the user approves
        refused = self.tool("rbx_stage", ok=False, action="stage", id="design", status="done")
        self.assertIn("gate", refused)
        self.assertEqual(self.stage(action="decisions"), "No new decisions.")
        self.decide(design, "approved")
        self.assertIn("[%s] Design plan: approved." % design, self.stage(action="decisions"))
        self.assertEqual(self.stage(action="decisions"), "No new decisions.")
        self.assertEqual(self.stage(action="stage", id="design", status="done"), "Stage design done.")
        stages = []  # each finished stage asks the app for a checkpoint
        for n in os.listdir(os.path.join(self.work, "stage-requests")):
            with open(os.path.join(self.work, "stage-requests", n), encoding="utf-8") as f:
                req = json.load(f)
            self.assertEqual((req["project"], req["lead"]), (self.project, "c-lead"))
            stages.append(req["stage"])
        self.assertEqual(sorted(stages), ["design", "idea"])

        # build: two pieces of work, filed non-blocking; the user wants a change to one
        self.stage(action="stage", id="build", status="start")
        world = self.review(title="Obby course", summary="Three checkpoints.", media=[self.expert("world", "Obby course")],
                            expert="world")
        shop = self.review(title="Coin shop", summary="Shop screen.", media=[self.expert("ui", "Coin shop")], expert="ui")
        self.assertEqual([r["writes"] for r in self.app.seen[1:]], [True, True])
        self.decide(world, "approved")
        self.decide(shop, "changes", "Make the buy buttons bigger")
        got = self.stage(action="decisions")
        self.assertIn("[%s] Coin shop: changes wanted. Make the buy buttons bigger" % shop, got)
        self.assertIn("[%s] Obby course: approved." % world, got)
        self.assertEqual(self.stage(action="decisions"), "No new decisions.")  # each answer once

        # a fresh expert of the same kind redoes it; the new item replaces the old one
        shot = self.expert("ui", "Coin shop, bigger buttons")
        self.assertEqual(self.app.seen[-1]["expert"], "ui")
        text = self.stage(action="review", title="Coin shop, bigger buttons", summary="Buttons 1.5x.", media=[shot],
                          expert="ui", replaces=shop)
        self.assertIn("It replaces [%s]." % shop, text)
        new = text.split("[", 1)[1].split("]", 1)[0]

        status = self.stage(action="status")
        self.assertIn("Current stage: build.", status)
        self.assertIn("idea (done)", status)
        self.assertIn("design (done)", status)
        self.assertIn("build (doing)", status)
        self.assertIn("test (todo)", status)
        self.assertIn("Open review items (1):", status)
        self.assertIn("[%s] Coin shop, bigger buttons (build, waiting)" % new, status)
        self.assertNotIn("[%s]" % shop, status)  # superseded: out of the open list
        self.assertNotIn("decision", status.split("Open review items", 1)[1])
        old = [i for i in rbxstage.load_items(self.project) if i["id"] == shop][0]
        self.assertEqual((old["status"], old["superseded_by"]), ("superseded", new))

        # lessons are notes: for the game in .fridae/notes, for every game in the home folder's notes
        self.stage(action="lesson", text="Give the UI builder the button sizes in its brief.", scope="game")
        self.stage(action="lesson", text="Ask for bigger buttons on phones.", scope="all")
        for folder, words in ((os.path.join(self.project, ".fridae", "notes"), "button sizes"),
                              (os.path.join(self.home, "notes"), "bigger buttons")):
            [name] = [f for f in os.listdir(folder) if f.endswith(".md")]
            with open(os.path.join(folder, name), encoding="utf-8") as f:
                self.assertIn(words, f.read())
        self.assertIn("button sizes", rbxvault.run("search", {"query": "button sizes brief", "shelf": "game"}))

    def test_the_lead_cannot_be_spawned_and_experts_cannot_spawn(self):
        task = "x" * 90
        self.assertIn("game's own chat", self.tool("rbx_team", ok=False, action="spawn", expert="lead", name="Lead", task=task))
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": "worker"}):
            self.assertIn("may not start more",
                          self.tool("rbx_team", ok=False, action="spawn", expert="coder", name="Coder", task=task))
        self.assertEqual(self.app.seen, [])


if __name__ == "__main__":
    unittest.main()
