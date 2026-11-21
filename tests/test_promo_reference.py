"""Reference-video breakdown and the style list. Skips without an installed engine."""
import os, tempfile, unittest

from rbxpromo import engine

READY = engine.status()["ready"]
if READY:
    from rbxpromo import reference
    from rbxpromo.recipe import STYLES, validate
    from tests.promo_fixtures import make_clip


@unittest.skipUnless(READY, "promo engine not installed")
class ReferenceTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="promo ref é ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def test_reference_shot_lengths(self):
        clip = make_clip(os.path.join(self.tmp, "tik tok é.mp4"), 6, size=(720, 1280), cuts=(1.5, 3.0, 4.2))
        r = reference.reference(clip, os.path.join(self.tmp, "out"))
        want = [(0, 1.5), (1.5, 3.0), (3.0, 4.2), (4.2, 6.0)]
        self.assertEqual(len(r["shots"]), len(want))
        for got, (a, b) in zip(r["shots"], want):
            self.assertAlmostEqual(got["start"], a, delta=0.15)
            self.assertAlmostEqual(got["end"], b, delta=0.15)
        self.assertEqual(r["words"], [])
        self.assertTrue(os.path.isfile(r["sheet"]))
        self.assertIsInstance(r["beats"], list)
        sk = r["skeleton"]
        self.assertEqual(len(sk["scenes"]), 4)
        self.assertTrue(all(s["clip"] is None for s in sk["scenes"]))
        # the problems before filling name the missing clips; once filled it validates
        self.assertTrue(validate(sk, self.tmp))
        filler = make_clip(os.path.join(self.tmp, "game.mp4"), 3, audio=False)
        for s in sk["scenes"]:
            s["clip"] = filler
            s["out"] = min(s["out"], 3)
        self.assertEqual(validate(sk, self.tmp), [])

    def test_reference_rejects_url(self):
        for url in ("https://www.tiktok.com/@a/video/1", "http://vm.tiktok.com/x", "www.tiktok.com/@a/video/1"):
            with self.assertRaises(ValueError) as cm:
                reference.reference(url, os.path.join(self.tmp, "out"))
            self.assertIn("save the video file", str(cm.exception))
        self.assertFalse(os.path.exists(os.path.join(self.tmp, "out")))

    def test_reference_missing_file(self):
        with self.assertRaises(ValueError):
            reference.reference(os.path.join(self.tmp, "nope.mp4"), os.path.join(self.tmp, "out"))

    def test_beats_found_in_clicks(self):
        import numpy as np
        rate = 8000
        x = np.zeros(rate * 4)
        for t in (0.5, 1.5, 2.5, 3.5):
            i = int(t * rate)
            x[i:i + 400] = 0.8 * np.sin(np.arange(400) * 0.4)
        beats = reference.beats_from_samples(x, rate)
        self.assertEqual(len(beats), 4)
        for got, want in zip(beats, (0.5, 1.5, 2.5, 3.5)):
            self.assertAlmostEqual(got, want, delta=0.05)

    @staticmethod
    def _bed(seconds, rate=8000, seed=1):
        import numpy as np
        t = np.arange(int(seconds * rate)) / rate
        rng = np.random.default_rng(seed)
        return (0.12 * np.sin(2 * np.pi * 55 * t) + 0.08 * np.sin(2 * np.pi * 220 * t)
                + 0.03 * rng.standard_normal(len(t)))

    def test_beats_found_over_a_music_bed(self):
        import numpy as np
        rate = 8000
        x = self._bed(8)
        kicks = [0.5 * k for k in range(1, 16)]
        for k in kicks:                       # decaying 120 -> 60 Hz thump
            n = int(0.18 * rate)
            tt = np.arange(n) / rate
            x[int(k * rate):int(k * rate) + n] += 0.5 * np.exp(-tt * 18) * np.sin(2 * np.pi * (120 * tt - 160 * tt * tt))
        beats = reference.beats_from_samples(x, rate)
        hit = sum(1 for k in kicks if any(abs(b - k) <= 0.05 for b in beats))
        self.assertGreaterEqual(hit, 0.8 * len(kicks), beats)
        self.assertLessEqual(len(beats), len(kicks) + 3, beats)

    def test_no_beats_in_steady_tone_or_silence(self):
        import numpy as np
        self.assertLessEqual(len(reference.beats_from_samples(self._bed(8), 8000)), 1)
        self.assertEqual(reference.beats_from_samples(np.zeros(8000 * 3), 8000), [])

    def test_styles_lists_all(self):
        out = os.path.join(self.tmp, "styles")
        items = reference.styles(out)
        self.assertEqual([i["name"] for i in items], list(STYLES))
        for i in items:
            self.assertTrue(i["about"])
            self.assertTrue(os.path.isfile(i["still"]), i)
        mtimes = {i["name"]: os.path.getmtime(i["still"]) for i in items}
        again = reference.styles(out)             # cached: stills are not rendered again
        self.assertEqual(mtimes, {i["name"]: os.path.getmtime(i["still"]) for i in again})


if __name__ == "__main__":
    unittest.main()
