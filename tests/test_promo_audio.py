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

    def test_long_script_is_not_cut_off(self):
        places = ["castle", "jungle", "harbor", "desert", "volcano", "market", "island", "tower", "canyon", "garden"]
        things = ["glowing key", "silver map", "hidden door", "golden crown", "tiny robot", "ancient bridge"]
        # varied sentences: Whisper drops repeated ones, which would hide what this test checks
        sentences = ["In the %s you can find a %s and then meet new friends." % (p, t) for p in places[:8] for t in things[:2]]
        text = " ".join(sentences) + " The very last sentence ends here."
        out = os.path.join(self.tmp.name, "long.wav")
        r = self.audio.voice(text, out)
        self.assertGreater(r["duration"], 60)  # about 220 words, well past the model's ~510 token limit
        self.assertEqual([w["text"].lower().strip(".") for w in r["words"][-4:]], ["last", "sentence", "ends", "here"])
        self.assertAlmostEqual(len(r["words"]), len(text.split()), delta=len(text.split()) * 0.1)  # Whisper may mishear a few
        starts = [w["start"] for w in r["words"]]
        self.assertEqual(starts, sorted(starts))
        self.assertGreater(len(set(starts)), len(starts) * 0.95)  # no pile-up of equal times past 30 s

    def test_voice_raises_when_speech_misses_the_script(self):
        from unittest import mock
        with mock.patch.object(self.audio, "transcribe", return_value={"words": [{"text": "hello", "start": 0, "end": 1}], "duration": 1}):
            with self.assertRaises(self.audio.engine.EngineError):
                self.audio.voice("this is a much longer script than the speech that came back from the model", os.path.join(self.tmp.name, "m.wav"))

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
