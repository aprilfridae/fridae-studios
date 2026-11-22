"""install.py builds Claude Code's skill folders from the vault exactly as the old skills/<name>/ folders were."""
import hashlib
import json
import os
import re
import shutil
import tempfile
import unittest

import install


class ClaudeSkillsTests(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(os.path.dirname(__file__), "claude_skills_baseline.json"), encoding="utf-8") as f:
            self.base = json.load(f)  # captured from skills/ before the move into vault/
        self.dest = tempfile.mkdtemp(prefix="claude skills é ")  # never the real ~/.claude
        self.addCleanup(shutil.rmtree, self.dest, True)

    def test_same_folders_files_and_contents(self):
        skills = install.claude_skills()
        self.assertEqual(sorted(skills), sorted(self.base))
        for name, files in skills.items():
            self.assertEqual(sorted(files), sorted(self.base[name]["files"]), name)
            for rel, src in files.items():
                with open(src, "rb") as f:
                    self.assertEqual(hashlib.sha1(f.read()).hexdigest(), self.base[name]["files"][rel], name + "/" + rel)

    def test_built_folders_match_with_front_matter(self):
        n = install.build_claude_skills(self.dest)
        self.assertEqual(n, len(self.base))
        for name, info in self.base.items():
            folder = os.path.join(self.dest, name)
            built = sorted(os.path.relpath(os.path.join(r, f), folder).replace(os.sep, "/")
                           for r, _, fs in os.walk(folder) for f in fs)
            self.assertEqual(built, sorted(info["files"]), name)
            with open(os.path.join(folder, "SKILL.md"), encoding="utf-8") as f:
                text = f.read()
            fm = re.match(r"^---\n(.*?)\n---\n", text, re.S).group(1)
            self.assertEqual(install.fill(info["frontmatter"]), fm, name)
            self.assertNotIn("{{RBXKIT", text, name)

    def test_install_claude_writes_only_under_its_home(self):
        home = os.path.join(self.dest, "home")
        os.makedirs(os.path.join(home, ".claude"))
        old = install.HOME, install.say
        install.HOME, install.say = home, lambda *a: None
        try:
            self.assertTrue(install.install_claude())
        finally:
            install.HOME, install.say = old
        self.assertTrue(os.path.isfile(os.path.join(home, ".claude", "skills", "roblox-studio-workflow", "SKILL.md")))
        self.assertTrue(os.path.isfile(os.path.join(home, ".claude", "agents", "roblox-tester.md")))


if __name__ == "__main__":
    unittest.main()
