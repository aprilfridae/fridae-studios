"""The tool server notes each result's pictures for the app (FRIDAE_TOOL_PICTURES): a Gemini chat kept open over ACP
hands the app no tool results, so the app reads the image files of a result from this file instead."""
import json
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxkit_mcp


class ToolPicturesTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="pictures é ")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.notes = os.path.join(self.tmp, "c1.jsonl")
        self.png = os.path.join(self.tmp, "spark_check.png")
        with open(self.png, "wb") as f:
            f.write(b"\x89PNG\r\n\x1a\n")

    def lines(self):
        with open(self.notes, encoding="utf-8") as f:
            return [json.loads(line) for line in f]

    def test_a_result_with_pictures_is_noted_one_line_per_result(self):
        text = 'made "%s" and "%s"' % (self.png, os.path.join(self.tmp, "missing.png"))
        with mock.patch.dict(os.environ, {"FRIDAE_TOOL_PICTURES": self.notes}):
            r = rbxkit_mcp.result(text)
            rbxkit_mcp.result(text)
        self.assertEqual(self.lines(), [{"images": [self.png]}, {"images": [self.png]}])
        # the result itself is as ever: its text, then the picture
        self.assertEqual([c["type"] for c in r["content"]], ["text", "image"])

    def test_nothing_is_noted_without_pictures_for_a_failed_result_or_when_the_app_did_not_ask(self):
        with mock.patch.dict(os.environ, {"FRIDAE_TOOL_PICTURES": self.notes}):
            rbxkit_mcp.result("no pictures here")
            rbxkit_mcp.result('failed at "%s"' % self.png, ok=False)
            rbxkit_mcp.result('text only "%s"' % self.png, with_images=False)
        self.assertFalse(os.path.exists(self.notes))
        with mock.patch.dict(os.environ, {"FRIDAE_TOOL_PICTURES": ""}):
            rbxkit_mcp.result('made "%s"' % self.png)
        self.assertFalse(os.path.exists(self.notes))

    def test_a_file_that_cannot_be_written_never_breaks_the_result(self):
        with mock.patch.dict(os.environ, {"FRIDAE_TOOL_PICTURES": os.path.join(self.tmp, "no such folder", "c1.jsonl")}):
            r = rbxkit_mcp.result('made "%s"' % self.png)
        self.assertFalse(r["isError"])
        self.assertEqual(len(r["content"]), 2)


if __name__ == "__main__":
    unittest.main()
