import os, tempfile, unittest

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


if __name__ == "__main__":
    unittest.main()
