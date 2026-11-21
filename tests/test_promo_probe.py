import os, tempfile, unittest
from unittest import mock

from rbxpromo import engine

READY = engine.status()["ready"]
if READY:
    from rbxpromo import probe
    from tests.promo_fixtures import make_clip, make_vfr


@unittest.skipUnless(READY, "promo engine not installed")
class ProbeTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="promo probe ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def p(self, name):
        return os.path.join(self.tmp, name)

    def test_probe_basic(self):
        c = make_clip(self.p("a b.mp4"), 5)
        i = probe.probe_file(c)
        self.assertAlmostEqual(i["duration"], 5, delta=0.1)
        self.assertEqual((i["width"], i["height"]), (1280, 720))
        self.assertTrue(i["has_audio"])
        self.assertFalse(i["vfr"])
        self.assertAlmostEqual(i["fps"], 30, delta=0.1)

    def test_probe_no_audio(self):
        c = make_clip(self.p("q.mp4"), 3, audio=False)
        r = probe.probe([c], self.tmp + "/out")[c]
        self.assertFalse(r["info"]["has_audio"])
        self.assertEqual(r["loud"], [])
        self.assertTrue(os.path.exists(r["sheet"]))

    def test_probe_detects_cuts(self):
        c = make_clip(self.p("c.mp4"), 6, cuts=(2, 4))
        r = probe.probe([c], self.tmp + "/out")[c]
        for t in (2, 4):
            self.assertTrue(any(abs(x - t) <= 0.2 for x in r["cuts"]), (t, r["cuts"]))
        self.assertEqual(len(r["cuts"]), 2, r["cuts"])
        self.assertTrue(r["loud"] and len(r["loud"]) <= 10)
        self.assertEqual(len(r["loud"][0]), 3)
        self.assertTrue(r["motion"] and len(r["motion"]) <= 10)
        vals = [m[2] for m in r["motion"]]
        self.assertEqual(vals, sorted(vals, reverse=True))

    def test_probe_portrait_and_4x3(self):
        for size in ((900, 1600), (640, 480)):
            c = make_clip(self.p("%dx%d.mp4" % size), 2, size=size)
            r = probe.probe([c], self.tmp + "/out")[c]
            self.assertEqual((r["info"]["width"], r["info"]["height"]), size)

    def test_probe_vfr_flagged(self):
        src = make_clip(self.p("src.mp4"), 5, audio=False)
        v = make_vfr(self.p("vfr.mp4"), 5)
        self.assertTrue(probe.probe_file(v)["vfr"])
        self.assertFalse(probe.probe_file(src)["vfr"])

    def test_no_cuts_in_fast_pan(self):
        c = make_clip(self.p("pan.mp4"), 4, size=(640, 360), audio=False, pan=True)
        self.assertEqual(probe.probe([c], self.tmp + "/out")[c]["cuts"], [])

    def test_low_contrast_cut_found(self):
        c = make_clip(self.p("lc.mp4"), 4, size=(640, 360), audio=False, cuts=(2,), shift=28)
        cuts = probe.probe([c], self.tmp + "/out")[c]["cuts"]
        self.assertEqual(len(cuts), 1, cuts)
        self.assertAlmostEqual(cuts[0], 2, delta=0.2)

    def test_garbage_file_raises_value_error(self):
        g = self.p("junk.mp4")
        with open(g, "wb") as f:
            f.write(os.urandom(4096))
        with self.assertRaises(ValueError):
            probe.probe_file(g)
        with self.assertRaises(ValueError):
            probe.probe([g], self.tmp + "/out")

    def test_sheet_systemexit_becomes_value_error(self):
        c = make_clip(self.p("x.mp4"), 2, size=(320, 180), audio=False)
        with mock.patch("rbxvideo.frames", side_effect=SystemExit("no frames could be read")):
            with self.assertRaises(ValueError):
                probe.probe([c], self.tmp + "/out")

    def test_truncation_warning(self):
        c = make_clip(self.p("t.mp4"), 2, size=(320, 180), audio=False)
        real = probe.probe_file

        def lying(path, warnings=None):
            d = real(path, warnings)
            d["duration"] = 10.0
            return d
        with mock.patch.object(probe, "probe_file", side_effect=lying):
            w = probe.probe([c], self.tmp + "/out")[c]["warnings"]
        self.assertTrue(any("only" in x and "of 10.0 s" in x for x in w), w)

    def test_audio_decode_failure_is_a_warning(self):
        c = make_clip(self.p("a.mp4"), 2, size=(320, 180))
        with mock.patch.object(engine, "ffmpeg", return_value=engine.ffprobe()):   # wrong tool: fails
            w = []
            self.assertEqual(probe._loud_windows(c, w), [])
        self.assertTrue(any(x.startswith("audio could not be decoded") for x in w), w)

    def test_clean_clip_has_no_warnings(self):
        c = make_clip(self.p("w.mp4"), 2, size=(320, 180))
        self.assertEqual(probe.probe([c], self.tmp + "/out")[c]["warnings"], [])

    def test_progress_callback(self):
        c = make_clip(self.p("p.mp4"), 2, size=(320, 180), audio=False)
        seen = []
        probe.probe([c], self.tmp + "/out", on_progress=seen.append)
        self.assertTrue(seen and seen[-1] == 1.0 and all(0 <= x <= 1 for x in seen))

    def test_unicode_path(self):
        d = os.path.join(self.tmp, "Jos\u00e9")
        os.makedirs(d)
        c = make_clip(os.path.join(d, "clip \u00e9.mp4"), 2, size=(320, 180))
        r = probe.probe([c], os.path.join(self.tmp, "out \u00e9"))[c]
        self.assertAlmostEqual(r["info"]["duration"], 2, delta=0.1)
        self.assertTrue(os.path.exists(r["sheet"]))


if __name__ == "__main__":
    unittest.main()
