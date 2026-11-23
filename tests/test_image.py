"""rbx_image: an AI-generated image saved in the game's project folder at an exact size, and `keep`, which brings an
image that already exists (Gemini's own generate_image output, a file in the project or the app's work folder) there.

Antigravity is stood in for by a small program that answers `models` and one headless turn the way agy 1.5.15 was
recorded doing (app-rs/tests/fixtures/gemini/command-refused-softly-then-image.ndjson): an `init` line with the
conversation, the picture written into `<home>/.gemini/antigravity-cli/brain/<conversation>/<name>_<time>.jpg`, and a
`result` line whose response names that file."""
import io
import json
import os
import shutil
import stat
import sys
import tempfile
import unittest
from unittest import mock

try:
    from PIL import Image
except ImportError:  # the tool needs Pillow; without it these tests have nothing to test
    Image = None

import rbxkit_mcp

POSIX = os.name == "posix"

FAKE_AGY = r'''import json, os, sys, time
log = os.environ["FAKE_AGY_LOG"]
mode = os.environ.get("FAKE_AGY_MODE", "ok")
args = sys.argv[1:]
with open(log, "a") as f:
    f.write(json.dumps(args) + "\n")
if args[:1] == ["models"]:
    if mode == "no-models":
        sys.exit(1)
    print("Fetching available models...")
    for line in ("gemini-4-neon-high\tGemini 4 Neon (High)", "gemini-3.8-flash-high\tGemini 3.8 Flash (High)",
                 "gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)", "gemini-3.8-flash-low\tGemini 3.8 Flash (Low)",
                 "claude-opus-5-5-low\tClaude Opus 5.5 (Low)"):
        print(line)
    sys.exit(0)
ask = json.loads(sys.stdin.readline())["message"]["content"]
with open(log, "a") as f:
    f.write(json.dumps({"ask": ask}) + "\n")
if mode == "signed-out":
    sys.stderr.write("Error: authentication required. Run agy to sign in.\n")
    sys.exit(1)
if mode == "quota":
    print(json.dumps({"event": "init", "conversation_id": "conv-q", "init": {}}))
    print(json.dumps({"event": "result", "result": {"conversation_id": "conv-q", "status": "ERROR", "response": "",
                      "error": {"message": "Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 2h40m52s."}}}))
    sys.exit(1)
if mode == "bad-model" and "--model" in args:
    sys.stderr.write("Error: unknown model\n")
    sys.exit(2)
conv = "conv-1"
brain = os.path.join(os.path.expanduser("~"), ".gemini", "antigravity-cli", "brain", conv)
os.makedirs(brain, exist_ok=True)
from PIL import Image
wide = "16:9" in ask
img = Image.new("RGB", (1376, 768) if wide else (1024, 1024), (30, 90, 200))
img.paste((250, 200, 40), (0, 0, img.size[0] // 2, img.size[1]))
path = os.path.join(brain, "picture_%d.jpg" % int(time.time() * 1000))
img.save(path, quality=90)
print(json.dumps({"event": "init", "conversation_id": conv, "init": {"model": "x"}}))
if mode == "silent":
    print(json.dumps({"event": "result", "result": {"conversation_id": conv, "status": "SUCCESS", "response": "Done."}}))
else:
    print(json.dumps({"event": "result", "result": {"conversation_id": conv, "status": "SUCCESS", "response": "[%s](file://%s)" % (path, path)}}))
'''


def text(res):
    return res["content"][0]["text"]


@unittest.skipUnless(POSIX and Image is not None, "needs a POSIX stand-in for agy, and Pillow")
class ImageToolTests(unittest.TestCase):
    def setUp(self):
        self.tmp = os.path.realpath(tempfile.mkdtemp(prefix="image é "))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.home = os.path.join(self.tmp, "home")
        self.project = os.path.join(self.tmp, "My Game")
        self.work = os.path.join(self.tmp, "work")
        for d in (self.home, self.project, self.work):
            os.makedirs(d)
        # (a shell wrapper: a `#!` line cannot name a Python whose path has a space in it)
        self.agy = os.path.join(self.tmp, "agy")
        with open(self.agy + ".py", "w") as f:
            f.write(FAKE_AGY)
        with open(self.agy, "w") as f:
            f.write('#!/bin/sh\nexec "%s" "%s.py" "$@"\n' % (sys.executable, self.agy))
        os.chmod(self.agy, os.stat(self.agy).st_mode | stat.S_IEXEC)
        self.log = os.path.join(self.tmp, "agy.log")
        env = {"HOME": self.home, "RBXKIT_PROJECT": self.project, "FRIDAE_AGY": self.agy, "FAKE_AGY_LOG": self.log, "FAKE_AGY_MODE": "ok"}
        patch = mock.patch.dict(os.environ, env)
        patch.start()
        self.addCleanup(patch.stop)
        import rbxgen, rbximage
        self.rbxgen, self.rbximage = rbxgen, rbximage
        for mod, name, value in ((rbximage, "WORK", self.work), (rbxgen, "WORK", self.work)):
            p = mock.patch.object(mod, name, value)
            p.start()
            self.addCleanup(p.stop)

    def mode(self, m):
        os.environ["FAKE_AGY_MODE"] = m

    def calls(self):
        with open(self.log) as f:
            return [json.loads(line) for line in f]

    def call(self, **args):
        return rbxkit_mcp.HANDLERS["rbx_image"](args)

    def picture(self, path, size=(640, 480), colour=(10, 160, 60)):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        Image.new("RGB", size, colour).save(path)
        return path

    # ------------------------------------------------------------ generate
    def test_generate_saves_the_image_in_the_project_at_exactly_the_size_asked_for(self):
        r = self.call(action="generate", prompt="a paper boat on a calm lake at dawn", size="1920x1080", name="Thumb 1", folder="store/thumbnails")
        self.assertFalse(r["isError"], text(r))
        out = json.loads(text(r))
        self.assertEqual(out["file"], os.path.join(self.project, "store", "thumbnails", "Thumb 1.png"))
        with Image.open(out["file"]) as img:
            self.assertEqual(img.size, (1920, 1080))
        self.assertEqual(out["size"], "1920x1080")
        self.assertEqual(out["made_at"], "1376x768", "what the model returned, so an enlargement is not a surprise")
        self.assertIn("Not uploaded", out["note"])
        # the model was asked for the nearest shape it offers, on the cheapest Flash model agy lists
        calls = self.calls()
        self.assertEqual(calls[0], ["models"])
        turn = calls[1]
        self.assertEqual(turn[turn.index("--model") + 1], "gemini-3.8-flash-low")
        self.assertIn("16:9", calls[2]["ask"])
        self.assertIn("a paper boat on a calm lake at dawn", calls[2]["ask"])
        # the picture comes back with the result (the saved file, or a smaller copy to look at when it is large)
        kinds = [c["type"] for c in r["content"]]
        self.assertEqual(kinds, ["text", "image"], kinds)

    def test_a_square_icon_is_asked_for_square_and_a_small_file_is_the_picture_itself(self):
        r = self.call(action="generate", prompt="a smooth grey skipping stone", size="512x512", name="Icon")
        out = json.loads(text(r))
        self.assertEqual(out["file"], os.path.join(self.project, "art", "Icon.png"), "the folder when none is named")
        with Image.open(out["file"]) as img:
            self.assertEqual(img.size, (512, 512))
        self.assertIn("1:1", self.calls()[2]["ask"])
        self.assertNotIn("look_at", out)
        self.assertEqual(len(r["content"]), 2)
        self.assertEqual(r["content"][1]["mimeType"], "image/png")

    def test_a_file_too_large_to_hand_back_comes_with_a_smaller_copy_to_look_at(self):
        with mock.patch.object(self.rbximage, "ATTACHED_UP_TO", 200), mock.patch.object(rbxkit_mcp, "MAX_IMAGE", 200):
            r = self.call(action="generate", prompt="x", size="1920x1080", name="Big")
        out = json.loads(text(r))
        self.assertEqual(out["look_at"], os.path.join(self.work, "images", "previews", "Big_preview.jpg"), "not in the project")
        with Image.open(out["look_at"]) as img:
            self.assertEqual(img.size, (1280, 720))
        self.assertEqual(sorted(os.listdir(os.path.join(self.project, "art"))), ["Big.png"])

    def test_a_crop_keeps_the_middle_and_never_stretches(self):
        # the stand-in's picture is yellow on its left half and blue on its right: a tall crop of a wide picture
        # keeps the middle, where the two meet
        out = json.loads(text(self.call(action="generate", prompt="x", size="200x400", name="Tall")))
        with Image.open(out["file"]) as img:
            self.assertEqual(img.size, (200, 400))
            left, right = img.getpixel((10, 200)), img.getpixel((190, 200))
        self.assertGreater(left[0], 200)
        self.assertGreater(right[2], 150)

    def test_the_file_is_never_put_outside_the_project_and_never_over_another(self):
        outside = os.path.join(self.tmp, "elsewhere")
        os.makedirs(outside)
        os.symlink(outside, os.path.join(self.project, "link"))
        for folder in ("../elsewhere", outside, "art/../../elsewhere", ".git/hooks", "a/.fridae", "link", "~/Desktop"):
            r = self.call(action="generate", prompt="x", size="64x64", name="Bad", folder=folder)
            self.assertTrue(r["isError"], folder)
            self.assertIn("project folder", text(r))
        self.assertEqual(os.listdir(outside), [])
        self.assertFalse(os.path.exists(self.log), "refused before anything was generated")
        # an absolute folder inside the project is fine (briefs name them that way)
        inside = os.path.join(self.project, "store")
        a = json.loads(text(self.call(action="generate", prompt="x", size="64x64", name="Icon", folder=inside)))
        b = json.loads(text(self.call(action="generate", prompt="x", size="64x64", name="Icon", folder=inside)))
        self.assertEqual(a["file"], os.path.join(inside, "Icon.png"))
        self.assertEqual(b["file"], os.path.join(inside, "Icon-2.png"), "an earlier image is kept")
        # a name is only ever a file name
        c = json.loads(text(self.call(action="generate", prompt="x", size="64x64", name="../../evil/Na me!.png", folder=inside)))
        self.assertEqual(c["file"], os.path.join(inside, "Na me.png"))

    def test_without_a_project_the_image_goes_to_the_apps_work_folder(self):
        with mock.patch.dict(os.environ, {"RBXKIT_PROJECT": ""}):
            out = json.loads(text(self.call(action="generate", prompt="x", size="64x64", name="Icon", folder="icons")))
        self.assertEqual(out["file"], os.path.join(self.work, "images", "icons", "Icon.png"))

    def test_what_is_asked_wrongly_is_said_plainly_before_anything_is_generated(self):
        for args, word in (({"action": "generate", "size": "64x64"}, "prompt"),
                           ({"action": "generate", "prompt": "x", "size": "big"}, "size"),
                           ({"action": "generate", "prompt": "x", "size": "9000x100"}, "4096"),
                           ({"action": "generate", "prompt": "x", "size": "0x100"}, "size"),
                           ({"action": "paint", "prompt": "x"}, "generate"),
                           ({"action": "keep"}, "source")):
            r = self.call(**args)
            self.assertTrue(r["isError"], args)
            self.assertIn(word, text(r))
        self.assertFalse(os.path.exists(self.log))

    # ------------------------------------------------------- when it fails
    def test_failures_are_said_in_plain_words(self):
        self.mode("quota")
        r = self.call(action="generate", prompt="x", size="64x64")
        self.assertTrue(r["isError"])
        self.assertIn("Individual quota reached", text(r))
        self.assertIn("Resets in 2h40m52s", text(r))
        self.assertIn("plan", text(r))
        self.mode("signed-out")
        r = self.call(action="generate", prompt="x", size="64x64")
        self.assertTrue(r["isError"])
        self.assertIn("not signed in", text(r))
        with mock.patch.dict(os.environ, {"FRIDAE_AGY": ""}), mock.patch.object(self.rbxgen.shutil, "which", lambda name: None):
            r = self.call(action="generate", prompt="x", size="64x64")
        self.assertTrue(r["isError"])
        self.assertIn("not installed", text(r))
        self.assertIn("antigravity.google", text(r))
        self.assertEqual(os.listdir(self.project), [], "nothing was written")

    def test_no_flash_model_or_a_model_agy_will_not_take_means_no_model_is_named(self):
        self.mode("no-models")
        r = self.call(action="generate", prompt="x", size="64x64", name="A")
        self.assertFalse(r["isError"], text(r))
        self.assertNotIn("--model", self.calls()[1])
        os.remove(self.log)
        shutil.rmtree(os.path.join(self.work, "antigravity"), True)
        self.mode("bad-model")
        r = self.call(action="generate", prompt="x", size="64x64", name="B")
        self.assertFalse(r["isError"], text(r))
        turns = [c for c in self.calls() if isinstance(c, list) and c != ["models"]]
        self.assertEqual(["--model" in t for t in turns], [True, False], "once with the model, then once without")

    def test_an_answer_that_does_not_name_the_file_still_finds_it(self):
        self.mode("silent")
        r = self.call(action="generate", prompt="x", size="64x64", name="Quiet")
        self.assertFalse(r["isError"], text(r))

    def test_the_cheapest_flash_model_is_the_lowest_level_and_a_lite_one_first(self):
        pick = self.rbxgen.cheapest_flash
        self.assertEqual(pick(["gemini-4-neon-low", "gemini-3.8-flash-high", "gemini-3.8-flash-medium", "gemini-3.8-flash-low"]), "gemini-3.8-flash-low")
        self.assertEqual(pick(["gemini-3.8-flash-low", "gemini-3.8-flash-lite-high", "gemini-3.8-flash-lite-low"]), "gemini-3.8-flash-lite-low")
        self.assertEqual(pick(["gemini-3.8-flash", "gemini-3.8-flash-high"]), "gemini-3.8-flash")
        self.assertIsNone(pick(["gemini-4-neon-low", "claude-opus-5-5-low"]))
        self.assertIsNone(pick([]))

    def test_textures_use_the_same_cheap_model(self):
        raw = self.rbxgen.antigravity_image(self.agy, "weathered oak planks")
        with Image.open(io.BytesIO(raw)) as img:
            self.assertEqual(img.size, (1024, 1024))
        turn = self.calls()[1]
        self.assertEqual(turn[turn.index("--model") + 1], "gemini-3.8-flash-low")
        self.assertIn("square (1:1)", self.calls()[2]["ask"])

    # ---------------------------------------------------------------- keep
    def test_keep_brings_geminis_own_picture_into_the_project_at_an_exact_size(self):
        brain = os.path.join(self.home, ".gemini", "antigravity-cli", "brain", "00bd7342-4e1f", "grey_stone_17954.jpg")
        self.picture(brain, (1024, 1024))
        r = self.call(action="keep", source="file://" + brain, size="512x512", name="Icon", folder="store")
        self.assertFalse(r["isError"], text(r))
        out = json.loads(text(r))
        self.assertEqual(out["file"], os.path.join(self.project, "store", "Icon.png"))
        with Image.open(out["file"]) as img:
            self.assertEqual(img.size, (512, 512))
        self.assertTrue(os.path.exists(brain), "the original is left where it was")
        self.assertEqual([c["type"] for c in r["content"]], ["text", "image"])
        self.assertFalse(os.path.exists(self.log), "nothing is generated")

    def test_keep_takes_images_from_the_project_and_the_work_folder_and_keeps_their_size_when_none_is_asked(self):
        shot = self.picture(os.path.join(self.work, "shots", "capture.png"), (800, 450))
        out = json.loads(text(self.call(action="keep", source=shot, name="Capture")))
        self.assertEqual((out["file"], out["size"]), (os.path.join(self.project, "art", "Capture.png"), "800x450"))
        mine = self.picture(os.path.join(self.project, "refs", "mood.jpg"), (300, 300))
        out = json.loads(text(self.call(action="keep", source=mine, size="150x150")))
        self.assertEqual(out["file"], os.path.join(self.project, "art", "mood.png"), "named after the source when no name is given")
        # a jpg name is saved as a jpg
        out = json.loads(text(self.call(action="keep", source=mine, name="mood.jpg", folder="store")))
        self.assertEqual(out["file"], os.path.join(self.project, "store", "mood.jpg"))
        with Image.open(out["file"]) as img:
            self.assertEqual(img.format, "JPEG")

    def test_keep_refuses_a_source_elsewhere_and_anything_that_is_no_image(self):
        elsewhere = self.picture(os.path.join(self.tmp, "elsewhere", "private.png"))
        secret = os.path.join(self.home, ".ssh", "id_rsa")
        os.makedirs(os.path.dirname(secret))
        with open(secret, "w") as f:
            f.write("KEY")
        notes = os.path.join(self.project, "notes.png")
        with open(notes, "w") as f:
            f.write("not a picture")
        text_file = os.path.join(self.project, "GAME.md")
        with open(text_file, "w") as f:
            f.write("# Game")
        link = os.path.join(self.project, "sneaky.png")
        os.symlink(elsewhere, link)
        for source, word in ((elsewhere, "inside"), (secret, "image"), (notes, "image"), (text_file, "image"), (link, "inside"),
                             (os.path.join(self.project, "missing.png"), "no such file"), ("https://example.com/a.png", "file on this computer")):
            r = self.call(action="keep", source=source, name="X")
            self.assertTrue(r["isError"], source)
            self.assertIn(word, text(r), source)
        self.assertFalse(os.path.exists(os.path.join(self.project, "art")), "nothing was written")

    # ------------------------------------------------- how the server runs it
    def test_it_is_listed_takes_its_turn_like_the_other_file_writing_tools_and_has_a_time_limit(self):
        tool = next(t for t in rbxkit_mcp.TOOLS if t[0] == "rbx_image")
        self.assertEqual(tool[2]["required"], ["action"])
        self.assertEqual(tool[2]["properties"]["action"]["enum"], ["generate", "keep"])
        for word in ("icon", "thumbnail", "Gemini plan", "never another game's characters or art", "Nothing is uploaded"):
            self.assertIn(word, tool[1])
        self.assertNotIn("rbx_image", rbxkit_mcp.INLINE)
        self.assertNotIn("rbx_image", rbxkit_mcp.BESIDE)
        self.assertTrue(rbxkit_mcp._Job("rbx_image", {}).queued)
        self.assertEqual(rbxkit_mcp._job_life("rbx_image", {}), 620)
        self.assertIn("rbx_image", rbxkit_mcp.INSTRUCTIONS)


if __name__ == "__main__":
    unittest.main()
