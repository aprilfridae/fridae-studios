"""rbxmerge.merge3: the three-way merge Pull newest uses, with no git. A clash is None, never conflict marks."""
import unittest

import rbxmerge


class Merge3Tests(unittest.TestCase):
    def test_disjoint_changes_combine(self):
        self.assertEqual(rbxmerge.merge3(b"A\nb\nc\nd\n", b"a\nb\nc\nd\n", b"a\nb\nc\nD\n"), b"A\nb\nc\nD\n")

    def test_same_line_changed_both_ways_is_none(self):
        self.assertIsNone(rbxmerge.merge3(b"a\nmine\nc\n", b"a\nb\nc\n", b"a\ntheirs\nc\n"))

    def test_identical_change_is_clean(self):
        self.assertEqual(rbxmerge.merge3(b"a\nX\nc\n", b"a\nb\nc\n", b"a\nX\nc\n"), b"a\nX\nc\n")

    def test_adjacent_lines_are_none(self):
        self.assertIsNone(rbxmerge.merge3(b"A\nb\nc\n", b"a\nb\nc\n", b"a\nB\nc\n"))

    def test_both_append_different_lines_is_none(self):
        self.assertIsNone(rbxmerge.merge3(b"a\nx\n", b"a\n", b"a\ny\n"))

    def test_crlf_local_lf_vault_combines(self):
        """Review Focus 3: the user's file re-saved with Windows line endings, the vault changed another line."""
        self.assertEqual(rbxmerge.merge3(b"a\r\nb\r\nc\r\nd\r\n", b"a\nb\nc\nd\n", b"a\nb\nc\nD\n"),
                         b"a\r\nb\r\nc\r\nD\n")

    def test_no_conflict_marks_ever(self):
        clashing = [(b"a\nmine\nc\n", b"a\nb\nc\n", b"a\ntheirs\nc\n"),
                    (b"A\nb\nc\n", b"a\nb\nc\n", b"a\nB\nc\n"),
                    (b"a\nx\n", b"a\n", b"a\ny\n")]
        for local, base, new in clashing:
            out = rbxmerge.merge3(local, base, new)
            self.assertIsNone(out)
            self.assertNotIn(b"<<<<<<<", out or b"")

    def test_binary_and_big_are_none(self):
        self.assertIsNone(rbxmerge.merge3(b"a\0", b"a", b"b"))
        x = b"x\n" * ((1 << 19) + 1)
        self.assertGreater(len(x), 1 << 20)
        self.assertIsNone(rbxmerge.merge3(x, x, b"y\n"))

    def test_is_binary(self):
        for path, data in (("Rig.FBX", b"text"), ("a.md", b"\xff\xfe"), ("a.md", b"a\0b")):
            self.assertTrue(rbxmerge.is_binary(path, data), path)
        for path, data in (("a.md", b"plain"), ("build.py", b"import bpy\n")):
            self.assertFalse(rbxmerge.is_binary(path, data), path)


if __name__ == "__main__":
    unittest.main()
