"""Local voiceover (Kokoro) and word timing (Whisper). Skips without a ready engine."""
import os, tempfile, time, unittest, wave

from rbxpromo import engine

READY = engine.status()["ready"]


@unittest.skipUnless(READY, "promo engine is not installed")
class AudioTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from rbxpromo import audio
        cls.audio = audio
        cls.tmp = tempfile.TemporaryDirectory()
        cls.wav = os.path.join(cls.tmp.name, "voice café.wav")  # a name with a space and a non-ASCII letter
        cls.spoken = audio.voice("Hello world, play now", cls.wav)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_voice_hello(self):
        r = self.spoken
        self.assertEqual(r["file"], self.wav)
        self.assertTrue(os.path.isfile(self.wav))
        self.assertTrue(0.8 <= r["duration"] <= 4, r["duration"])
        with wave.open(self.wav) as w:
            self.assertEqual(w.getframerate(), 24000)
        words = [x["text"].strip(" ,.!?").lower() for x in r["words"]]
        self.assertIn("hello", words)
        self.assertIn("play", words)
        for x in r["words"]:
            self.assertLessEqual(x["start"], x["end"])

    def test_transcribe_voice_roundtrip(self):
        r = self.audio.transcribe(self.wav)
        self.assertGreaterEqual(len(r["words"]), 3)
        starts = [w["start"] for w in r["words"]]
        self.assertEqual(starts, sorted(starts))
        self.assertGreater(r["duration"], 0.8)

    def test_transcribe_cached(self):
        first = self.audio.transcribe(self.wav)
        t = time.time()
        second = self.audio.transcribe(self.wav)
        self.assertLess(time.time() - t, 1.0)
        self.assertEqual(first, second)

    def test_voice_cached(self):
        out = os.path.join(self.tmp.name, "again.wav")
        t = time.time()
        r = self.audio.voice("Hello world, play now", out)
        self.assertLess(time.time() - t, 1.0)
        self.assertTrue(os.path.isfile(out))
        self.assertEqual(r["words"], self.spoken["words"])

    def test_transcribe_silent_file(self):
        path = os.path.join(self.tmp.name, "silence.wav")
        with wave.open(path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(16000)
            w.writeframes(b"\0\0" * 16000 * 3)
        r = self.audio.transcribe(path)
        self.assertEqual(r["words"], [])
        self.assertAlmostEqual(r["duration"], 3.0, delta=0.2)

    def test_empty_text_is_an_error(self):
        with self.assertRaises(ValueError):
            self.audio.voice("   ", os.path.join(self.tmp.name, "x.wav"))

    def test_missing_file_is_an_error(self):
        with self.assertRaises(ValueError):
            self.audio.transcribe(os.path.join(self.tmp.name, "nope.wav"))


class AlignTests(unittest.TestCase):
    def test_glued_words_are_split_by_script(self):
        from rbxpromo.audio import align_to_script
        got = align_to_script("Hello world, play now", [
            {"text": "Hello", "start": 0, "end": 0.5}, {"text": "Worldplay", "start": 0.5, "end": 1.1},
            {"text": "Now.", "start": 1.1, "end": 1.3}])
        self.assertEqual([w["text"] for w in got], ["Hello", "world,", "play", "now"])
        self.assertEqual(got[2]["end"], 1.1)
        self.assertLess(got[1]["end"], got[2]["start"] + 0.001)

    def test_mismatch_keeps_whisper_words(self):
        from rbxpromo.audio import align_to_script
        w = [{"text": "other", "start": 0, "end": 1}]
        self.assertEqual(align_to_script("Hello", w), w)


if __name__ == "__main__":
    unittest.main()
