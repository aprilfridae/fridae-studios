"""The promo self-check: safe zones, hook, loudness, contrast. Pure-layout tests need no engine."""
import json, os, tempfile, unittest

from rbxpromo import check as mod
from rbxpromo import engine
from rbxpromo.recipe import STYLES

READY = engine.status()["ready"]


def box(frm, to, x, y, w, h, kind="caption"):
    return {"from": frm, "to": to, "x": x, "y": y, "w": w, "h": h, "kind": kind}


def recipe(*scenes, **extra):
    r = {"version": 1, "size": [1080, 1920], "fps": 30, "style": "hype", "scenes": list(scenes)}
    r.update(extra)
    return r


TITLE = {"title": "HELLO", "duration": 3.0}
CENTRED = box(0.2, 1.5, 240, 700, 600, 200)        # well inside the screen, clear of every zone


class SafeZoneTests(unittest.TestCase):
    def test_caption_in_right_rail_fails(self):
        res = mod.check(None, [box(2.0, 3.0, 800, 900, 270, 150)], recipe(TITLE, {"title": "B", "duration": 3}))
        self.assertFalse(res["passed"])
        text = " ".join(res["problems"])
        self.assertIn("2.0 s", text)
        for name in ("TikTok", "Reels", "Shorts"):
            self.assertIn(name, text)
        self.assertIn("right rail", text)

    def test_centered_caption_passes(self):
        res = mod.check(None, [CENTRED], recipe(TITLE))
        self.assertEqual(res, {"passed": True, "problems": [], "notes": []})

    def test_small_overlap_tolerated(self):
        # 1 % of the box dips into the tiktok bottom area (y >= 1536): under the 2 % limit
        b = box(0.2, 1.0, 240, 1536 - 198, 600, 200)
        self.assertEqual(mod._safe_zone_problems([b]), [])
        self.assertTrue(mod._safe_zone_problems([box(0.2, 1.0, 240, 1536 - 190, 600, 200)]))

    def test_repeats_collapse_to_first_time(self):
        probs = mod._safe_zone_problems([box(t, t + 0.5, 0, 0, 1080, 100) for t in (1.0, 2.0, 3.0)])
        self.assertTrue(all("1.0 s" in p and "2 more" in p for p in probs), probs)


class HookTests(unittest.TestCase):
    def test_no_hook_fails(self):
        clip = {"clip": "a.mp4", "in": 0, "out": 5}
        late = box(2.5, 3.0, 240, 700, 600, 200)
        res = mod.check(None, [late], recipe(clip, {"clip": "b.mp4", "in": 0, "out": 5}))
        self.assertFalse(res["passed"])
        self.assertTrue(any("hook" in p.lower() and "2.5 s" in p for p in res["problems"]), res["problems"])

    def test_no_hook_when_nothing_at_all(self):
        res = mod.check(None, [], recipe({"clip": "a.mp4", "in": 0, "out": 5}))
        self.assertTrue(any("none at all" in p for p in res["problems"]))

    def test_title_at_start_is_a_hook(self):
        self.assertEqual(mod._hook_problems([box(0, 3, 240, 700, 600, 200, "title")], recipe(TITLE)), [])

    def test_caption_by_one_second_is_a_hook(self):
        clip = {"clip": "a.mp4", "in": 0, "out": 5}
        self.assertEqual(mod._hook_problems([box(1.0, 2.0, 240, 700, 600, 200)], recipe(clip)), [])

    def test_early_scene_boundary_is_a_cut(self):
        r = recipe({"clip": "a.mp4", "in": 0, "out": 0.8}, {"clip": "b.mp4", "in": 0, "out": 5})
        self.assertEqual(mod._hook_problems([], r), [])

    def test_first_clip_alone_is_not_a_hook_and_end_of_video_is_not_a_cut(self):
        self.assertTrue(mod._hook_problems([], recipe({"clip": "a.mp4", "in": 0, "out": 0.5})))

    def test_end_card_text_is_not_a_hook(self):
        clip = {"clip": "a.mp4", "in": 0, "out": 5}
        self.assertTrue(mod._hook_problems([box(0.5, 5, 240, 700, 600, 200, "end")], recipe(clip)))


class LengthTests(unittest.TestCase):
    def test_long_video_note(self):
        res = mod.check(None, [CENTRED], recipe({"title": "A", "duration": 61}))
        self.assertIn("Shorts", " ".join(res["notes"]))
        self.assertTrue(res["passed"])

    def test_sixty_seconds_has_no_note(self):
        self.assertEqual(mod.check(None, [CENTRED], recipe({"title": "A", "duration": 60}))["notes"], [])


class ContrastMathTests(unittest.TestCase):
    def test_wcag_ratio(self):
        self.assertAlmostEqual(mod.contrast_ratio((255, 255, 255), (0, 0, 0)), 21.0, places=1)
        self.assertAlmostEqual(mod.contrast_ratio((255, 255, 255), (255, 255, 255)), 1.0)

    def test_caption_presets_are_readable_from_source(self):
        presets = mod._caption_presets()
        self.assertEqual(presets["punch"], {"color": "#FFFFFF", "stroke": 0.07, "strokeColor": "#000000"})
        self.assertEqual(presets["cinema"]["stroke"], 0.0)
        self.assertEqual(len(presets), 8)


def _sine(freq, seconds, peak_db, channels, rate=48000):
    import numpy as np
    t = np.arange(int(seconds * rate)) / rate
    x = 10 ** (peak_db / 20) * np.sin(2 * np.pi * freq * t)
    return np.tile(x[:, None], (1, channels))


class LoudnessMathTests(unittest.TestCase):
    """BS.1770 reference: a stereo 1 kHz sine whose peak is -23 dBFS reads -23 LUFS (K-weighting adds
    +0.7 dB at 1 kHz, which the -0.691 offset cancels, and two channels add 3 dB to the mean square)."""

    def test_stereo_reference_level(self):
        self.assertAlmostEqual(mod.integrated_loudness(_sine(1000, 5, -23.0, 2)), -23.0, delta=0.5)

    def test_mono_full_scale(self):
        self.assertAlmostEqual(mod.integrated_loudness(_sine(1000, 5, 0.0, 1)), -3.0, delta=0.5)

    def test_quiet_part_is_gated_out(self):
        import numpy as np
        loud, quiet = _sine(1000, 4, -20.0, 2), _sine(1000, 4, -45.0, 2)
        self.assertAlmostEqual(mod.integrated_loudness(np.vstack([loud, quiet])), -20.0, delta=0.5)

    def test_silence_is_minus_infinity(self):
        import numpy as np
        self.assertEqual(mod.integrated_loudness(np.zeros((48000, 2))), float("-inf"))

    def test_rejects_other_rates(self):
        with self.assertRaises(ValueError):
            mod.integrated_loudness(_sine(1000, 1, -20, 1, rate=44100), rate=44100)


@unittest.skipUnless(READY, "promo engine not installed")
class MediaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from tests import promo_fixtures
        cls.fx = promo_fixtures

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="promo chéck ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def video(self, name, peak_db=None, colour=None, seconds=3):
        """A 1080x1920 clip: flat `colour` (BGR) or the gradient fixture; a stereo 1 kHz sine (identical channels, so LUFS == peak dBFS) at `peak_db`, or no audio."""
        import cv2, numpy as np
        path = os.path.join(self.tmp, name)
        img = np.zeros((1920, 1080, 3), np.uint8)
        img[:] = colour if colour is not None else (90, 90, 90)
        data = self.fx._png(img) * (seconds * 10)
        args = ["-f", "image2pipe", "-framerate", "10", "-c:v", "png", "-i", "-"]
        if peak_db is not None:
            args += ["-f", "lavfi", "-i", "sine=frequency=1000:duration=%d,volume=%fdB,pan=stereo|c0=c0|c1=c0" % (seconds, peak_db + 18.06)]
            args += ["-ar", "48000", "-c:a", "aac", "-b:a", "256k", "-shortest"]
        else:
            args += ["-an"]
        self.fx._ffmpeg(args + ["-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", path], stdin=data)
        return path

    def test_loudness_measured(self):
        path = self.video("quiet.mp4", peak_db=-30.0)
        res = mod.check(path, [box(0.2, 1.0, 240, 700, 600, 200, "title")], recipe(TITLE))
        loud = [p for p in res["problems"] if "LUFS" in p]
        self.assertEqual(len(loud), 1, res)
        self.assertIn("-30.", loud[0])
        self.assertFalse(res["passed"])

    def test_loudness_in_range_passes(self):
        path = self.video("ok.mp4", peak_db=-14.0)
        self.assertAlmostEqual(mod.measure_loudness(path), -14.0, delta=0.6)
        res = mod.check(path, [], recipe(TITLE))
        self.assertFalse([p for p in res["problems"] if "LUFS" in p], res)

    def test_loud_video_fails(self):
        res = mod.check(self.video("loud.mp4", peak_db=-6.0), [], recipe(TITLE))
        self.assertTrue(any("too loud" in p and "-6." in p for p in res["problems"]), res)

    def test_no_audio_track_is_a_note(self):
        res = mod.check(self.video("mute.mp4"), [CENTRED], recipe(TITLE))
        self.assertEqual(res["problems"], [])
        self.assertTrue(any("no audio" in n for n in res["notes"]), res)

    def test_silent_audio_is_a_note_not_a_problem(self):
        path = os.path.join(self.tmp, "silent.mp4")
        self.fx._ffmpeg(["-i", self.video("v.mp4"), "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-t", "3",
                         "-c:v", "copy", "-c:a", "aac", path])
        res = mod.check(path, [CENTRED], recipe(TITLE))
        self.assertEqual(res["problems"], [], res)
        self.assertTrue(any("silent" in n for n in res["notes"]), res)

    def test_missing_video_raises(self):
        with self.assertRaises(FileNotFoundError):
            mod.check(os.path.join(self.tmp, "nope.mp4"), [], recipe(TITLE))

    def test_light_caption_on_white_fails_and_on_dark_passes(self):
        cin = recipe(TITLE, style="cinematic", captions={"source": "words"})   # cinema preset: cream, no outline
        lay = [box(0.5, 2.0, 240, 700, 600, 200)]
        white = mod.check(self.video("white.mp4", -14.0, colour=(250, 250, 250)), lay, cin)
        self.assertTrue(any("1.2 s" in p and "hard to read" in p for p in white["problems"]), white)
        dark = mod.check(self.video("dark.mp4", -14.0, colour=(20, 20, 20)), lay, cin)
        self.assertEqual(dark["problems"], [], dark)

    def test_outlined_caption_passes_on_white(self):
        hype = recipe(TITLE, captions={"source": "words"})      # punch: white with a black outline
        res = mod.check(self.video("white2.mp4", -14.0, colour=(250, 250, 250)), [box(0.5, 2.0, 240, 700, 600, 200)], hype)
        self.assertEqual(res["problems"], [], res)

    def test_contrast_reads_only_a_handful_of_frames(self):
        from unittest import mock
        lay = [box(i * 0.05, i * 0.05 + 0.05, 240, 700, 600, 200) for i in range(60)]
        with mock.patch.object(mod, "_box_luminance", return_value=0.0) as m:
            mod.check(self.video("many.mp4", -14.0), lay, recipe(TITLE, captions={"source": "words"}))
        self.assertLessEqual(m.call_count, mod.MAX_CONTRAST_BOXES)


@unittest.skipUnless(READY, "promo engine not installed")
class ShippedStyleTests(unittest.TestCase):
    """Render each shipped style through the real renderer and run the safe-zone check on its layout.json."""

    @classmethod
    def setUpClass(cls):
        from tests import promo_fixtures
        cls.fx = promo_fixtures

    def test_every_style_stays_out_of_the_safe_zones(self):
        from tests.test_promo_render import run_render
        with tempfile.TemporaryDirectory(prefix="promo zones ") as tmp:
            clip = self.fx.make_clip(os.path.join(tmp, "clip.mp4"), 3, audio=False)
            words = os.path.join(tmp, "words.json")
            text = ("this is an unbelievably extraordinary update with seventeen thousand brand new pets "
                    "and legendary rewards for everyone who plays today").split()
            with open(words, "w", encoding="utf-8") as f:
                json.dump([{"text": t, "start": 0.2 + i * 0.12, "end": 0.2 + i * 0.12 + 0.1} for i, t in enumerate(text)], f)
            self.assertTrue(STYLES)
            for style in STYLES:
                with self.subTest(style=style):
                    rec = {"version": 1, "size": [1080, 1920], "fps": 30, "style": style,
                           "scenes": [{"title": "THIS PET IS ABSOLUTELY INSANE AND YOU WILL NOT BELIEVE IT", "duration": 1.0},
                                      {"clip": clip, "in": 0, "out": 3}],
                           "captions": {"source": "words", "words": words, "highlight": "auto"},
                           "end_card": {"title": "Pet Tycoon Update Twelve", "cta": "Play now on Roblox"}}
                    rp, lp = os.path.join(tmp, style + ".json"), os.path.join(tmp, style + "-layout.json")
                    with open(rp, "w", encoding="utf-8") as f:
                        json.dump(rec, f)
                    r, _ = run_render(rp, "-", "--layout", lp, "--still", "0.5", os.path.join(tmp, style + ".jpg"))   # layout only: no video
                    self.assertEqual(r.returncode, 0, r.stdout[-800:] + r.stderr[-800:])
                    with open(lp, encoding="utf-8") as f:
                        layout = json.load(f)
                    self.assertEqual({b["kind"] for b in layout}, {"caption", "title", "end"})
                    self.assertEqual(mod._safe_zone_problems(layout), [])


if __name__ == "__main__":
    unittest.main()
