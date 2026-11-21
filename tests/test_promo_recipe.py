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

    def test_other_field_checks(self):
        probs = validate(_recipe(fps=24, version=2, size=[100, 100]), self.tmp)
        self.assertEqual(len(probs), 3, probs)
        r = _recipe(scenes=[{"clip": "clip.mp4", "in": 0, "out": 2, "speed": 9,
                             "crop": {"x": 2, "track": "bad"}}])
        self.assertEqual(len(validate(r, self.tmp)), 3)


if __name__ == "__main__":
    unittest.main()
