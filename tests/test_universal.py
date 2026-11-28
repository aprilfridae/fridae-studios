"""One behaviour on Windows, macOS and Linux: where Roblox Studio's content folder is, and what the Studio tools say on
a system without Studio (Linux)."""
import os, sys, tempfile, unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import rbxpaths
import rbxstudio


class StudioContentDir(unittest.TestCase):
    def test_windows_takes_the_newest_studio_version(self):
        with tempfile.TemporaryDirectory() as local:
            for name, t in (("version-a", 100), ("version-b", 200), ("not-studio", 300)):
                d = os.path.join(local, "Roblox", "Versions", name)
                os.makedirs(d)
                if name != "not-studio":
                    open(os.path.join(d, "RobloxStudioBeta.exe"), "w").close()
                os.utime(d, (t, t))
            got = rbxpaths.studio_content_dir("win32", {"LOCALAPPDATA": local})
            self.assertEqual(got, os.path.join(local, "Roblox", "Versions", "version-b", "content"))

    def test_windows_without_localappdata_and_linux_have_none(self):
        self.assertIsNone(rbxpaths.studio_content_dir("win32", {}), "never a path relative to the current folder")
        self.assertIsNone(rbxpaths.studio_content_dir("linux", {"LOCALAPPDATA": "/x"}))

    def test_a_mac_uses_the_apps_own_content_folder_when_studio_is_installed(self):
        with mock.patch("os.path.isdir", lambda p: p == "/Applications/RobloxStudio.app/Contents/Resources/content"):
            self.assertEqual(rbxpaths.studio_content_dir("darwin", {}), "/Applications/RobloxStudio.app/Contents/Resources/content")
        with mock.patch("os.path.isdir", lambda p: False):
            self.assertIsNone(rbxpaths.studio_content_dir("darwin", {}))


class StudioOnLinux(unittest.TestCase):
    def test_studio_is_here_on_windows_and_macos_only(self):
        self.assertEqual(rbxpaths.STUDIO_HERE, sys.platform in ("win32", "darwin"))

    def test_without_studio_every_studio_tool_is_off_and_says_why(self):
        with mock.patch.object(rbxstudio, "STUDIO_HERE", False), mock.patch.dict(os.environ, {"FRIDAE_NO_STUDIO": ""}):
            self.assertTrue(rbxstudio.studio_off())
            self.assertIsNone(rbxstudio.launcher())
        self.assertIn("Linux", rbxpaths.NO_STUDIO_HERE)


if __name__ == "__main__":
    unittest.main()
