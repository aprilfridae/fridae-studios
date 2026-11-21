import os, tempfile, unittest

from rbxpromo.recipe import STYLES, RecipeError, load_recipe, recipe_duration, validate


def _touch(path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(b"x")
    return path


def _recipe(**over):
    r = {"version": 1, "size": [1080, 1920], "fps": 30, "style": "hype",
         "scenes": [{"clip": "clip.mp4", "in": 0, "out": 3}]}
    r.update(over)
    return r


class RecipeTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)
        _touch(os.path.join(self.tmp, "clip.mp4"))

    def test_minimal_valid(self):
        self.assertEqual(validate(_recipe(), self.tmp), [])

    def test_relative_paths_resolve_against_recipe_dir(self):
        sub = os.path.join(self.tmp, "proj")
        _touch(os.path.join(sub, "a.mp4"))
        r = _recipe(scenes=[{"clip": "a.mp4", "in": 0, "out": 1}])
        self.assertEqual(validate(r, sub), [])
        self.assertTrue(any("not found" in p for p in validate(r, self.tmp)))

    def test_load_recipe_uses_its_own_folder(self):
        import json
        sub = os.path.join(self.tmp, "proj")
        _touch(os.path.join(sub, "a.mp4"))
        path = os.path.join(sub, "r.json")
        with open(path, "w") as f:
            json.dump(_recipe(scenes=[{"clip": "a.mp4", "in": 0, "out": 1}]), f)
        self.assertEqual(load_recipe(path)["style"], "hype")

    def test_load_recipe_raises_with_problems(self):
        path = os.path.join(self.tmp, "bad.json")
        with open(path, "w") as f:
            f.write('{"version": 1, "style": "nope", "scenes": []}')
        with self.assertRaises(RecipeError) as cm:
            load_recipe(path)
        self.assertTrue(cm.exception.problems)

    def test_path_with_spaces_and_unicode(self):
        _touch(os.path.join(self.tmp, "Vidéos é", "clip 1.mp4"))
        r = _recipe(scenes=[{"clip": os.path.join("Vidéos é", "clip 1.mp4"), "in": 0, "out": 2}])
        self.assertEqual(validate(r, self.tmp), [])

    def test_missing_clip_file(self):
        r = _recipe(scenes=[{"clip": "nope.mp4", "in": 0, "out": 2}])
        probs = validate(r, self.tmp)
        self.assertTrue(any("nope.mp4" in p and "not found" in p for p in probs), probs)

    def test_in_after_out(self):
        r = _recipe(scenes=[{"clip": "clip.mp4", "in": 0, "out": 2},
                            {"clip": "clip.mp4", "in": 5, "out": 2}])
        probs = validate(r, self.tmp)
        self.assertIn("scene 1: 'in' must be before 'out'", " ".join(probs))

    def test_unknown_style(self):
        probs = validate(_recipe(style="boring"), self.tmp)
        self.assertTrue(any("boring" in p and all(s in p for s in STYLES) for p in probs), probs)

    def test_voice_text_and_project_checked(self):
        for bad in ("", "  ", 5, ["a"]):
            probs = validate(_recipe(audio={"voice_text": bad}), self.tmp)
            self.assertTrue(any("voice_text" in p for p in probs), (bad, probs))
        self.assertEqual(validate(_recipe(audio={"voice_text": "Hello there"}, project="my game"), self.tmp), [])
        probs = validate(_recipe(project=3), self.tmp)
        self.assertTrue(any("'project'" in p for p in probs), probs)

    def test_voice_and_voice_text_together_rejected(self):
        _touch(os.path.join(self.tmp, "voice.wav"))
        probs = validate(_recipe(audio={"voice": "voice.wav", "voice_text": "Hi"}), self.tmp, durations={"voice.wav": 1.0})
        self.assertTrue(any("not both" in p for p in probs), probs)

    def test_voice_longer_than_scenes(self):
        _touch(os.path.join(self.tmp, "voice.wav"))
        r = _recipe(audio={"voice": "voice.wav"},
                    scenes=[{"clip": "clip.mp4", "in": 0, "out": 6}])
        probs = validate(r, self.tmp, durations={"voice.wav": 9.0})
        self.assertTrue(any("voice is 9.0 s but the video is 6.0 s" in p for p in probs), probs)

    def test_recipe_duration(self):
        r = _recipe(scenes=[{"clip": "clip.mp4", "in": 0, "out": 2, "speed": 2},
                            {"title": "Hi", "duration": 1.2}])
        self.assertAlmostEqual(recipe_duration(r), 2.2)

    def test_absolute_path_with_spaces_and_accents(self):
        clip = _touch(os.path.join(self.tmp, "José Vidéos", "clip 1.mp4"))
        r = _recipe(scenes=[{"clip": clip, "in": 0, "out": 2}])
        self.assertEqual(validate(r, os.path.join(self.tmp, "elsewhere")), [])

    def test_windows_drive_path_is_absolute_on_any_os(self):
        from unittest import mock
        from rbxpromo import recipe as mod
        win = "C:\\Users\\José\\Videos\\clip 1.mp4"
        self.assertEqual(mod._resolve(win, self.tmp), win)
        r = _recipe(scenes=[{"clip": win, "in": 0, "out": 2}])
        with mock.patch.object(mod.os.path, "isfile", side_effect=lambda p: p == win):
            self.assertEqual(validate(r, self.tmp), [])

    def _audio_recipe(self, audio, out=6):
        _touch(os.path.join(self.tmp, "voice.wav"))
        _touch(os.path.join(self.tmp, "music.mp3"))
        return _recipe(audio=audio, scenes=[{"clip": "clip.mp4", "in": 0, "out": out}])

    def test_music_shorter_than_scenes_is_valid(self):
        r = self._audio_recipe({"music": "music.mp3"})
        self.assertEqual(validate(r, self.tmp, durations={"music.mp3": 2.0}), [])

    def test_voice_shorter_than_scenes_is_valid(self):
        r = self._audio_recipe({"voice": "voice.wav"})
        self.assertEqual(validate(r, self.tmp, durations={"voice.wav": 4.0}), [])

    def test_voice_equal_or_within_tolerance_is_valid(self):
        r = self._audio_recipe({"voice": "voice.wav"})
        for d in (6.0, 6.04):
            self.assertEqual(validate(r, self.tmp, durations={"voice.wav": d}), [], d)
        self.assertTrue(validate(r, self.tmp, durations={"voice.wav": 6.2}))

    def test_bad_scene_does_not_hide_voice_problem(self):
        r = self._audio_recipe({"voice": "voice.wav"})
        r["scenes"].append({"clip": "clip.mp4", "in": 5, "out": 2})
        probs = validate(r, self.tmp, durations={"voice.wav": 9.0})
        self.assertTrue(any("scene 1: 'in' must be before 'out'" in p for p in probs), probs)
        self.assertTrue(any("voice is 9.0 s but the video is 6.0 s" in p for p in probs), probs)

    def test_clip_and_voice_volume_ranges(self):
        for key in ("clip_volume", "voice_volume"):
            for ok in (0, 0.5, 1, 2):
                self.assertEqual(validate(_recipe(audio={key: ok}), self.tmp), [], (key, ok))
            for bad in (-0.1, 2.01, "1", True, None):
                probs = validate(_recipe(audio={key: bad}), self.tmp)
                self.assertTrue(any(key in p and "0 and 2" in p for p in probs), (key, bad, probs))

    def test_other_field_checks(self):
        probs = validate(_recipe(fps=24, version=2, size=[100, 100]), self.tmp)
        self.assertEqual(len(probs), 3, probs)
        r = _recipe(scenes=[{"clip": "clip.mp4", "in": 0, "out": 2, "speed": 9,
                             "crop": {"x": 2, "track": "bad"}}])
        self.assertEqual(len(validate(r, self.tmp)), 3)


if __name__ == "__main__":
    unittest.main()
