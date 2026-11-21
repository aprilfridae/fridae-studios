"""Renders through the engine's Remotion project (promo/scripts/render.mjs). Skips without an engine."""
import io, json, os, subprocess, tempfile, unittest, wave

from rbxpromo import engine
from rbxpromo.recipe import STYLES, recipe_duration, validate

HAS_NODE = engine.status()["node"] is not None


def _render_script():
    return os.path.join(engine.engine_dir(), "renderer", "scripts", "render.mjs")


def run_render(recipe_path, out, *extra, cwd=None):
    cmd = engine.node_cmd(_render_script(), recipe_path, out, *extra)
    r = subprocess.run(cmd, env=engine.node_env(), capture_output=True, text=True, encoding="utf-8",
                       errors="replace", timeout=900, cwd=cwd)
    lines = [json.loads(l) for l in r.stdout.splitlines() if l.startswith("{")]
    return r, lines


def ffprobe(path):
    r = subprocess.run([engine.ffprobe(), "-v", "error", "-show_streams", "-show_format", "-of", "json", path],
                       env=engine.tool_env(), capture_output=True, text=True, check=True)
    return json.loads(r.stdout)


def pcm(path, start, seconds):
    """Mono 16-bit samples of the file's audio from `start` for `seconds` (via a WAV pipe)."""
    import numpy as np
    r = subprocess.run([engine.ffmpeg(), "-v", "error", "-ss", str(start), "-t", str(seconds), "-i", path,
                        "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", "-f", "wav", "-"],
                       env=engine.tool_env(), capture_output=True, check=True)
    with wave.open(io.BytesIO(r.stdout)) as w:
        return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(float)


def rms(samples):
    import numpy as np
    return float(np.sqrt(np.mean(samples ** 2))) if len(samples) else 0.0


@unittest.skipUnless(HAS_NODE, "promo engine not installed")
class RenderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        st = engine.status()
        if not (st["renderer"] and st["browser"]):
            engine.install()
        from tests import promo_fixtures
        cls.fx = promo_fixtures

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="promo render ")
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def make_recipe(self, clip_dir, style="hype"):
        os.makedirs(clip_dir, exist_ok=True)
        silent = self.fx.make_clip(os.path.join(clip_dir, "silent clip.mp4"), 3, audio=False)
        four3 = self.fx.make_clip(os.path.join(clip_dir, "clip 4x3 é.mp4"), 3, size=(960, 720), cuts=(1.5,))
        music = os.path.join(self.tmp, "song.wav")   # 2 s: shorter than the 6 s video, so it must loop
        self.fx._ffmpeg(["-f", "lavfi", "-i", "sine=frequency=330:duration=2", "-c:a", "pcm_s16le", music])
        words = os.path.join(self.tmp, "words.json")
        texts = "this pet is insane win five thousand dollars in the new update play now".split()
        with open(words, "w", encoding="utf-8") as f:
            json.dump([{"text": t, "start": 1.1 + i * 0.4, "end": 1.1 + i * 0.4 + 0.35} for i, t in enumerate(texts)], f)
        recipe = {
            "version": 1, "size": [1080, 1920], "fps": 30, "style": style,
            "audio": {"music": music, "music_volume": 0.8},
            "scenes": [
                {"title": "THIS PET IS INSANE", "duration": 1.0},
                {"clip": silent, "in": 0, "out": 3, "crop": {"x": 0.3}, "zoom": "punch@0.4", "transition": "whip"},
                {"clip": four3, "in": 0.5, "out": 3, "speed": 1.25, "transition": "flash",
                 "focus": [{"t": 0, "x": 0.2}, {"t": 2, "x": 0.8}]},
            ],
            "captions": {"source": "words", "words": words, "highlight": "auto"},
            "end_card": {"title": "Pet Tycoon", "cta": "Play now"},
        }
        path = os.path.join(self.tmp, "promo.json")
        with open(path, "w", encoding="utf-8") as f:
            json.dump(recipe, f, ensure_ascii=False)
        self.assertEqual(validate(recipe, self.tmp), [])
        return path, recipe

    def check_video(self, out, recipe):
        info = ffprobe(out)
        v = [s for s in info["streams"] if s["codec_type"] == "video"]
        a = [s for s in info["streams"] if s["codec_type"] == "audio"]
        self.assertEqual(len(v), 1)
        self.assertEqual(len(a), 1, "audio stream present")
        self.assertEqual((v[0]["width"], v[0]["height"]), (1080, 1920))
        self.assertEqual(v[0]["codec_name"], "h264")
        self.assertEqual(a[0]["codec_name"], "aac")
        self.assertEqual(v[0]["r_frame_rate"], "30/1")
        self.assertAlmostEqual(float(info["format"]["duration"]), recipe_duration(recipe), delta=0.1)

    def check_ok(self, r, lines, out):
        self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
        self.assertTrue(os.path.isfile(out))
        progress = [l["progress"] for l in lines if "progress" in l]
        self.assertTrue(progress, "progress lines printed")
        self.assertEqual(progress, sorted(progress))
        self.assertTrue(lines[-1].get("ok"))
        return lines[-1]

    def test_render_two_scenes(self):
        path, recipe = self.make_recipe(os.path.join(self.tmp, "clips"))
        out, layout = "promo.mp4", "layout.json"   # relative to the working directory
        r, lines = run_render(path, out, "--layout", layout, cwd=self.tmp)
        out, layout = os.path.join(self.tmp, out), os.path.join(self.tmp, layout)
        self.assertFalse(os.path.exists(os.path.join(self.tmp, ".remotion")), "no browser download into the cwd")
        final = self.check_ok(r, lines, out)
        self.assertAlmostEqual(final["duration"], recipe_duration(recipe), delta=0.1)
        self.check_video(out, recipe)
        # music (2 s) loops under the whole 6 s video and fades out at the end
        self.assertGreater(rms(pcm(out, 4.5, 0.5)), 300, "music still playing after its own length")
        self.assertLess(rms(pcm(out, 5.92, 0.06)), rms(pcm(out, 4.5, 0.5)) * 0.5, "music fades out")
        with open(layout, encoding="utf-8") as f:
            boxes = json.load(f)
        kinds = {b["kind"] for b in boxes}
        self.assertEqual(kinds, {"caption", "title", "end"})
        for b in boxes:
            self.assertTrue(0 <= b["x"] and b["x"] + b["w"] <= 1080 and 0 <= b["y"] and b["y"] + b["h"] <= 1920, b)
            self.assertLess(b["from"], b["to"])
        cuts = (1.0, 4.0)
        for b in boxes:
            if b["kind"] == "caption":
                self.assertFalse(any(b["from"] < c - 1e-3 and b["to"] > c + 1e-3 for c in cuts), b)

    def test_render_unicode_path(self):
        path, recipe = self.make_recipe(os.path.join(self.tmp, "Vidéos é"), style="cinematic")
        out = os.path.join(self.tmp, "Vidéos é", "promo é.mp4")
        r, lines = run_render(path, out, "--layout", os.path.join(self.tmp, "Vidéos é", "layout.json"))
        self.check_ok(r, lines, out)
        self.check_video(out, recipe)

    def test_silent_recipe_still_has_audio(self):
        clip = self.fx.make_clip(os.path.join(self.tmp, "quiet.mp4"), 2, audio=False)
        recipe = {"version": 1, "size": [1080, 1920], "fps": 60, "style": "showcase",
                  "scenes": [{"clip": clip, "in": 0, "out": 2}]}
        path = os.path.join(self.tmp, "silent.json")
        with open(path, "w") as f:
            json.dump(recipe, f)
        out = os.path.join(self.tmp, "silent.mp4")
        r, lines = run_render(path, out, "--layout", os.path.join(self.tmp, "l.json"))
        self.check_ok(r, lines, out)
        info = ffprobe(out)
        self.assertTrue(any(s["codec_type"] == "audio" and s["codec_name"] == "aac" for s in info["streams"]))
        v = [s for s in info["streams"] if s["codec_type"] == "video"][0]
        self.assertEqual(v["r_frame_rate"], "60/1")

    def test_still_per_style(self):
        """One still per style (title card at 0.5 s); with PROMO_STILLS_DIR set, also 2.2 s (clip + caption) and
        5.5 s (end card), kept in that folder for a look."""
        keep = os.environ.get("PROMO_STILLS_DIR")
        for style in STYLES:
            path, _ = self.make_recipe(os.path.join(self.tmp, "clips"), style=style)
            folder = keep or self.tmp
            for sec in ("0.5", "2.2", "5.5"):
                still = os.path.join(folder, "%s-%s.jpg" % (style, sec))
                r, lines = run_render(path, "-", "--layout", os.path.join(self.tmp, "l.json"), "--still", sec, still)
                self.assertEqual(r.returncode, 0, r.stdout[-2000:] + r.stderr[-2000:])
                self.assertTrue(os.path.getsize(still) > 10000, still)
                if not keep:
                    break

    def test_music_longer_than_video_ducks_under_voice(self):
        clip = self.fx.make_clip(os.path.join(self.tmp, "a.mp4"), 3, audio=False)
        music = os.path.join(self.tmp, "long song.wav")   # 10 s: longer than the 3 s video, so it is cut and faded
        self.fx._ffmpeg(["-f", "lavfi", "-i", "sine=frequency=330:duration=10", "-c:a", "pcm_s16le", music])
        voice = os.path.join(self.tmp, "voice.wav")       # 1 s of silence: what remains audible is the ducked music
        with wave.open(voice, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(24000)
            w.writeframes(b"\0\0" * 24000)
        recipe = {"version": 1, "size": [1080, 1920], "fps": 30, "style": "hype",
                  "audio": {"music": music, "voice": voice, "music_volume": 0.8},
                  "scenes": [{"clip": clip, "in": 0, "out": 3}]}
        self.assertEqual(validate(recipe, self.tmp), [])
        path = os.path.join(self.tmp, "r.json")
        with open(path, "w") as f:
            json.dump(recipe, f)
        out = os.path.join(self.tmp, "duck.mp4")
        r, lines = run_render(path, out)
        self.check_ok(r, lines, out)
        self.assertAlmostEqual(float(ffprobe(out)["format"]["duration"]), 3.0, delta=0.1)
        ducked, full = rms(pcm(out, 0.3, 0.5)), rms(pcm(out, 1.6, 0.6))
        self.assertGreater(full, 300)
        self.assertLess(ducked, full * 0.4, (ducked, full))
        self.assertLess(rms(pcm(out, 2.92, 0.06)), full * 0.5, "music fades out at the end")

    def test_bad_recipe_reports_json_error(self):
        path = os.path.join(self.tmp, "bad.json")
        with open(path, "w") as f:
            json.dump({"version": 1, "size": [1080, 1920], "fps": 30, "style": "hype",
                       "scenes": [{"clip": os.path.join(self.tmp, "missing.mp4"), "in": 0, "out": 1}]}, f)
        r, lines = run_render(path, os.path.join(self.tmp, "x.mp4"), "--layout", os.path.join(self.tmp, "l.json"))
        self.assertNotEqual(r.returncode, 0)
        self.assertTrue(lines and lines[-1].get("ok") is False and "missing.mp4" in lines[-1]["error"], r.stdout)


class StylesListTests(unittest.TestCase):
    def test_styles_from_json_folder(self):
        folder = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "promo", "src", "styles")
        names = sorted(f[:-5] for f in os.listdir(folder) if f.endswith(".json"))
        self.assertEqual(sorted(STYLES), names)
        self.assertEqual(set(STYLES), {"hype", "cinematic", "meme", "countdown", "update-log", "lyric", "showcase", "pov"})

    def test_styles_fallback_without_folder(self):
        from unittest import mock
        from rbxpromo import recipe
        with mock.patch("os.listdir", side_effect=FileNotFoundError):
            self.assertEqual(recipe._style_names(), recipe._FALLBACK_STYLES)


if __name__ == "__main__":
    unittest.main()
