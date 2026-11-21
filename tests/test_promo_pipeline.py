"""The render pipeline: outputs, card file, auto-track, progress. Skips without a ready engine."""
import json, os, tempfile, unittest
from unittest import mock

import rbxpaths
from rbxpromo import engine

READY = engine.status()["ready"]


def _moving_square(path, fixtures, seconds=3, size=(640, 360), fps=30):
    """Silent clip: a white square on black moving right at 0.15 of the width per second from x=0.15."""
    import cv2, numpy as np
    n, data = int(seconds * fps), []
    for i in range(n):
        img = np.zeros((size[1], size[0], 3), np.uint8)
        cx = int((0.15 + 0.15 * i / fps) * size[0])
        cv2.rectangle(img, (cx - 20, 140), (cx + 20, 180), (255, 255, 255), -1)
        data.append(fixtures._png(img))
    fixtures._ffmpeg(["-f", "image2pipe", "-framerate", str(fps), "-c:v", "png", "-i", "-", "-c:v", "libx264",
                      "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-an", path], stdin=b"".join(data))
    return path


@unittest.skipUnless(READY, "promo engine not installed")
class PipelineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from tests import promo_fixtures
        cls.fx = promo_fixtures

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="promo pipé ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)
        self.work = os.path.join(self.tmp, "work")
        p = mock.patch.object(rbxpaths, "WORK", self.work)
        p.start()
        self.addCleanup(p.stop)

    def recipe(self, **extra):
        clip = self.fx.make_clip(os.path.join(self.tmp, "clip é.mp4"), 2, audio=False)
        r = {"version": 1, "size": [1080, 1920], "fps": 30, "style": "hype",
             "scenes": [{"title": "BIG NEWS", "duration": 1.0}, {"clip": clip, "in": 0, "out": 1.5}],
             "post": {"caption": "New update is live", "hashtags": ["roblox", "#games"]}}
        r.update(extra)
        path = os.path.join(self.tmp, "promo.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(r, f)
        return path

    def test_render_writes_all_outputs(self):
        from rbxpromo.render import render
        res = render(self.recipe())
        folder = res["folder"]
        self.assertTrue(folder.startswith(os.path.join(self.work, "promos")), folder)
        for name in ("promo.mp4", "cover.jpg", "caption.txt", "sheet.jpg", "layout.json", "promo.json"):
            self.assertGreater(os.path.getsize(os.path.join(folder, name)), 0, name)
        self.assertFalse(os.path.exists(os.path.join(folder, "voice.wav")))
        with open(res["caption"], encoding="utf-8") as f:
            self.assertEqual(f.read(), "New update is live\n#roblox #games")
        self.assertAlmostEqual(res["duration"], 2.5, delta=0.1)
        self.assertTrue(res["check"]["passed"])
        with open(os.path.join(folder, "promo.json"), encoding="utf-8") as f:
            saved = json.load(f)
        self.assertTrue(os.path.isabs(saved["scenes"][1]["clip"]))

    def test_card_file_written(self):
        from rbxpromo.render import render
        with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ID": "agent-7"}):
            res = render(self.recipe())
        cards = os.listdir(os.path.join(self.work, "promo-cards"))
        self.assertEqual(len(cards), 1, cards)
        with open(os.path.join(self.work, "promo-cards", cards[0]), encoding="utf-8") as f:
            card = json.load(f)
        self.assertEqual(cards[0], card["id"] + ".json")
        self.assertRegex(card["id"], "^" + os.path.basename(res["folder"]) + "-[0-9a-f]{8}$")
        self.assertEqual(set(card), {"id", "agent", "folder", "video", "cover", "caption_text", "style",
                                     "duration", "check", "created"})
        self.assertEqual(card["agent"], "agent-7")
        self.assertEqual(card["style"], "hype")
        self.assertEqual(card["caption_text"], "New update is live\n#roblox #games")
        for k in ("folder", "video", "cover"):
            self.assertTrue(os.path.exists(card[k]), k)

    def test_card_written_after_check(self):
        from rbxpromo import render as mod
        order = []
        real = mod.rbxpaths.write_json

        def fake_check(*a):
            order.append("check")
            return {"passed": True, "problems": []}

        def spy(path, *a, **k):
            if os.sep + "promo-cards" + os.sep in path:
                order.append("card")
            return real(path, *a, **k)
        with mock.patch.object(mod, "_run_check", side_effect=fake_check), mock.patch.object(mod.rbxpaths, "write_json", spy):
            mod.render(self.recipe())
        self.assertEqual(order, ["check", "card"])

    def test_real_check_result_lands_on_card(self):
        from rbxpromo import render as mod
        res = mod.render(self.recipe())
        chk = res["check"]
        self.assertEqual(set(chk), {"passed", "problems", "notes"})
        self.assertIsInstance(chk["passed"], bool)
        self.assertTrue(chk["passed"], chk)
        self.assertTrue(any("silent" in n for n in chk["notes"]), chk)     # the fixture promo has no music or voice
        self.assertEqual(self._card(res)["check"], chk)

    def test_failed_check_is_reported_in_card(self):
        from rbxpromo import render as mod
        bad = {"passed": False, "problems": ["caption in a safe zone"]}
        with mock.patch.object(mod, "_run_check", return_value=bad):
            res = mod.render(self.recipe())
        self.assertEqual(res["check"], bad)

    def test_auto_track_focus(self):
        from rbxpromo.render import auto_track
        clip = _moving_square(os.path.join(self.tmp, "square.mp4"), self.fx)
        keys = auto_track({"clip": clip, "in": 0, "out": 3, "crop": {"x": 0.15, "track": "auto"}})
        self.assertGreaterEqual(len(keys), 8)
        for k in keys:
            self.assertAlmostEqual(k["x"], 0.15 + 0.15 * k["t"], delta=0.1, msg=k)
        self.assertEqual([k["t"] for k in keys], sorted(k["t"] for k in keys))
        self.assertGreater(keys[-1]["x"], keys[0]["x"] + 0.2)

    def test_auto_track_fills_focus_in_recipe_copy(self):
        from rbxpromo.render import render
        clip = _moving_square(os.path.join(self.tmp, "square.mp4"), self.fx, seconds=2)
        path = self.recipe(scenes=[{"clip": clip, "in": 0, "out": 2, "crop": {"x": 0.15, "track": "auto"}}])
        res = render(path)
        with open(os.path.join(res["folder"], "render.json"), encoding="utf-8") as f:
            self.assertTrue(json.load(f)["scenes"][0]["focus"])
        with open(os.path.join(res["folder"], "promo.json"), encoding="utf-8") as f:
            self.assertNotIn("focus", json.load(f)["scenes"][0])

    def test_pan_speed_is_limited(self):
        from rbxpromo.render import _smooth, MAX_PAN
        keys = _smooth([(i * 0.25, 0.0 if i < 4 else 1.0) for i in range(12)], 0.0, 1.0, 0.25)
        for a, b in zip(keys, keys[1:]):
            self.assertLessEqual(abs(b["x"] - a["x"]) / (b["t"] - a["t"]), MAX_PAN + 1e-3)

    def test_progress_reported(self):
        from rbxpromo.render import render
        seen = []
        render(self.recipe(), on_progress=seen.append)
        self.assertGreater(len(seen), 3)
        self.assertEqual(seen, sorted(seen))
        self.assertEqual(len(set(seen)), len(seen))
        self.assertEqual(seen[-1], 1.0)

    def test_invalid_recipe_no_render(self):
        from rbxpromo.recipe import RecipeError
        from rbxpromo.render import render
        path = os.path.join(self.tmp, "bad.json")
        with open(path, "w") as f:
            json.dump({"version": 1, "size": [1080, 1920], "fps": 30, "style": "hype",
                       "scenes": [{"clip": os.path.join(self.tmp, "missing.mp4"), "in": 0, "out": 1}]}, f)
        with self.assertRaises(RecipeError) as cm:
            render(path)
        self.assertTrue(any("missing.mp4" in p for p in cm.exception.problems))
        self.assertFalse(os.path.exists(os.path.join(self.work, "promos")))
        self.assertFalse(os.path.exists(os.path.join(self.work, "promo-cards")))

    def test_voice_text_makes_voice_and_captions(self):
        from rbxpromo.render import render
        res = render(self.recipe(audio={"voice_text": "Big news today."}, captions={"source": "voice"}))
        folder = res["folder"]
        self.assertTrue(os.path.getsize(os.path.join(folder, "voice.wav")) > 1000)
        with open(os.path.join(folder, "render.json"), encoding="utf-8") as f:
            saved = json.load(f)
        self.assertEqual(saved["audio"]["voice"], os.path.join(folder, "voice.wav"))
        self.assertEqual([w["text"] for w in saved["captions"]["words"]], ["Big", "news", "today."])
        with open(os.path.join(folder, "promo.json"), encoding="utf-8") as f:
            original = json.load(f)
        self.assertEqual(original["audio"], {"voice_text": "Big news today."})
        self.assertNotIn("words", original["captions"])
        again = render(os.path.join(folder, "promo.json"), out_dir=folder)     # edit-and-render-again works
        self.assertEqual(again["folder"], folder)
        with open(os.path.join(folder, "layout.json"), encoding="utf-8") as f:
            self.assertIn("caption", {b["kind"] for b in json.load(f)})

    def test_voice_caption_source_without_voice_is_an_error(self):
        from rbxpromo.recipe import RecipeError
        from rbxpromo.render import render
        with self.assertRaises(RecipeError):
            render(self.recipe(captions={"source": "voice"}))
        self.assertFalse(os.path.exists(os.path.join(self.work, "promos")))

    def test_failed_render_leaves_no_card_or_folder(self):
        from rbxpromo import render as mod
        with mock.patch.object(mod, "_run_renderer", side_effect=engine.EngineError("boom")):
            with self.assertRaises(engine.EngineError):
                mod.render(self.recipe())
        self.assertFalse(os.path.exists(os.path.join(self.work, "promo-cards")))
        self.assertEqual(os.listdir(os.path.join(self.work, "promos", "general")), [])

    def _card(self, res):
        cards = os.listdir(os.path.join(self.work, "promo-cards"))
        self.assertEqual(len(cards), 1, cards)
        with open(os.path.join(self.work, "promo-cards", cards[0]), encoding="utf-8") as f:
            return json.load(f)

    def test_sheet_failure_keeps_video_and_card(self):
        from rbxpromo import render as mod
        with mock.patch.object(mod, "_sheet", side_effect=RuntimeError("sheet boom")):
            res = mod.render(self.recipe())
        self.assertTrue(os.path.isfile(res["video"]))
        self.assertIsNone(res["sheet"])
        self.assertTrue(any("sheet boom" in n for n in res["check"]["notes"]), res["check"])
        self.assertEqual(self._card(res)["check"], res["check"])

    def test_check_failure_keeps_video_and_card(self):
        from rbxpromo import render as mod
        with mock.patch.object(mod, "_run_check", side_effect=RuntimeError("check boom")):
            res = mod.render(self.recipe())
        self.assertTrue(os.path.isfile(res["video"]))
        self.assertTrue(os.path.isfile(res["sheet"]))
        self.assertFalse(res["check"]["passed"])
        self.assertEqual(res["check"]["problems"], ["The check could not run: check boom"])
        self.assertEqual(self._card(res)["check"], res["check"])

    def test_caption_write_failure_keeps_video_and_card(self):
        from rbxpromo import render as mod
        with mock.patch.object(mod, "_write_caption", side_effect=OSError("disk full")):
            res = mod.render(self.recipe())
        self.assertTrue(os.path.isfile(res["video"]))
        self.assertFalse(res["check"]["passed"])
        self.assertTrue(any("disk full" in p for p in res["check"]["problems"]), res["check"])
        self.assertTrue(os.path.isdir(self._card(res)["folder"]))

    def test_sheet_under_non_ascii_folder_with_windows_like_cv2(self):
        import cv2
        from rbxpromo import render as mod
        real_read, real_write = cv2.imread, cv2.imwrite
        r = mock.patch.object(cv2, "imread", lambda p, *a, **k: None if not str(p).isascii() else real_read(p, *a, **k))
        w = mock.patch.object(cv2, "imwrite", lambda p, *a, **k: False if not str(p).isascii() else real_write(p, *a, **k))
        with r, w:
            res = mod.render(self.recipe())          # self.tmp holds "pipé"
        self.assertTrue(os.path.isfile(res["sheet"]))
        self.assertTrue(res["check"]["passed"], res["check"])

    def test_card_id_unique_per_folder(self):
        from rbxpromo import render as mod
        a = mod.render(self.recipe(), out_dir=os.path.join(self.tmp, "x", "out"))
        b = mod.render(self.recipe(), out_dir=os.path.join(self.tmp, "y", "out"))
        ids = []
        for n in os.listdir(os.path.join(self.work, "promo-cards")):
            with open(os.path.join(self.work, "promo-cards", n), encoding="utf-8") as f:
                ids.append(json.load(f)["id"])
        self.assertEqual(len(set(ids)), 2, ids)
        for i in ids:
            self.assertRegex(i, r"^out-[0-9a-f]{8}$")
        self.assertTrue(os.path.isdir(a["folder"]) and os.path.isdir(b["folder"]))

    def test_promo_folder_names(self):
        from rbxpromo.render import promo_folder
        a = promo_folder("my/project:x", "THIS PET is INSANE!")
        self.assertEqual(os.path.dirname(a), os.path.join(self.work, "promos", "my_project_x"))
        self.assertRegex(os.path.basename(a), r"^\d{8}-\d{6}-this-pet-is-insane$")
        self.assertRegex(os.path.basename(promo_folder("c", "éé")), r"-promo$")


if __name__ == "__main__":
    unittest.main()
