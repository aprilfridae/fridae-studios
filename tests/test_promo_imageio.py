"""Image files under non-ASCII paths (Windows: cv2.imread/imwrite only take ANSI code page paths)."""
import os, tempfile, unittest
from unittest import mock

from rbxpromo import engine

try:
    import cv2, numpy as np
    HAS_CV2 = True
except ImportError:
    HAS_CV2 = False
READY = engine.status()["ready"]


def _windows_like_cv2():
    """Patches making cv2.imread/imwrite fail on any non-ASCII path, as they do on Windows."""
    real_read, real_write = cv2.imread, cv2.imwrite

    def read(path, *a, **k):
        return None if not str(path).isascii() else real_read(path, *a, **k)

    def write(path, *a, **k):
        return False if not str(path).isascii() else real_write(path, *a, **k)
    return mock.patch.object(cv2, "imread", read), mock.patch.object(cv2, "imwrite", write)


@unittest.skipUnless(HAS_CV2, "needs OpenCV")
class ImageIOTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="imgio José ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def test_round_trip_non_ascii_path(self):
        import rbxvideo
        img = np.zeros((20, 30, 3), np.uint8)
        img[:, :15] = (0, 0, 255)
        a, b = _windows_like_cv2()
        with a, b:
            for ext in (".png", ".jpg"):
                p = os.path.join(self.tmp, "föto é" + ext)
                rbxvideo.imwrite(p, img, [cv2.IMWRITE_JPEG_QUALITY, 88] if ext == ".jpg" else None)
                self.assertTrue(os.path.getsize(p) > 0)
                back = rbxvideo.imread(p)
                self.assertEqual(back.shape, img.shape)
                self.assertLess(float(np.abs(back.astype(int) - img).mean()), 8)

    def test_write_failure_raises(self):
        import rbxvideo
        with self.assertRaises(OSError):
            rbxvideo.imwrite(os.path.join(self.tmp, "missing folder", "x.png"), np.zeros((4, 4, 3), np.uint8))

    def test_read_failure_raises(self):
        import rbxvideo
        bad = os.path.join(self.tmp, "bad é.png")
        with open(bad, "wb") as f:
            f.write(b"not an image")
        with self.assertRaises(OSError):
            rbxvideo.imread(bad)
        with self.assertRaises(OSError):
            rbxvideo.imread(os.path.join(self.tmp, "nope.png"))

    def test_capture_error_names_existing_path(self):
        import rbxvideo
        bad = os.path.join(self.tmp, "junk é.mp4")
        with open(bad, "wb") as f:
            f.write(b"\0" * 64)
        with self.assertRaises(ValueError) as cm:
            rbxvideo.open_capture(bad)
        self.assertIn(bad, str(cm.exception))


@unittest.skipUnless(READY, "promo engine not installed")
class NonAsciiSheetTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="sheet José ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def test_frames_sheet_written_with_windows_like_cv2(self):
        import rbxvideo
        from tests.promo_fixtures import make_clip
        c = make_clip(os.path.join(self.tmp, "clip é.mp4"), 2, size=(320, 180), audio=False)
        a, b = _windows_like_cv2()
        with a, b:
            res = rbxvideo.frames(c, count=4, out_dir=os.path.join(self.tmp, "out é"))
            sep = rbxvideo.frames(c, count=2, out_dir=os.path.join(self.tmp, "sep é"), sheet=False)
        for p in res["look_at"] + sep["look_at"]:
            self.assertTrue(os.path.getsize(p) > 0, p)

    def test_probe_sheet_folder_is_ascii_and_exists(self):
        from rbxpromo import probe
        from tests.promo_fixtures import make_clip
        c = make_clip(os.path.join(self.tmp, "Jörg's clíp é.mp4"), 2, size=(320, 180), audio=False)
        out = os.path.join(self.tmp, "out é")
        a, b = _windows_like_cv2()
        with a, b:
            r = probe.probe([c], out)[c]
        self.assertTrue(os.path.isfile(r["sheet"]), r["sheet"])
        self.assertTrue(os.path.relpath(r["sheet"], out).isascii(), r["sheet"])


if __name__ == "__main__":
    unittest.main()
