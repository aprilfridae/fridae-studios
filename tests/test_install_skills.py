"""install.py builds Claude Code's skill folders from the vault, with the same folders, files and front matter as the
old skills/<name>/ folders (the baseline), and each built file the vault source as it is now."""
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

    def test_same_folders_and_files(self):
        skills = install.claude_skills()
        self.assertEqual(sorted(skills), sorted(self.base))
        for name, files in skills.items():
            self.assertEqual(sorted(files), sorted(self.base[name]["files"]), name)

    def test_built_output_is_the_vault_source(self):
        """Each built file is its vault source (text files with placeholders filled, a SKILL.md's name the one Claude
        Code knows it by), so guide edits flow through."""
        install.build_claude_skills(self.dest)
        for name, files in install.claude_skills().items():
            for rel, src in files.items():
                with open(src, "rb") as f:
                    want = f.read()
                if src.lower().endswith((".md", ".lua", ".txt")):
                    want = install.built_text(src, want.decode("utf-8")).encode("utf-8")
                with open(os.path.join(self.dest, name, *rel.split("/")), "rb") as f:
                    self.assertEqual(f.read(), want, name + "/" + rel)

    def test_built_folders_match_with_front_matter(self):
        n = install.build_claude_skills(self.dest)
        self.assertEqual(n, len(self.base))
        for name, info in self.base.items():
            folder = os.path.join(self.dest, name)
            built = sorted(os.path.relpath(os.path.join(r, f), folder).replace(os.sep, "/")
                           for r, _, fs in os.walk(folder) for f in fs if f != ".fridae-installed.json")
            self.assertEqual(built, sorted(info["files"]), name)
            with open(os.path.join(folder, "SKILL.md"), encoding="utf-8") as f:
                text = f.read()
            fm = re.match(r"^---\n(.*?)\n---\n", text, re.S).group(1)
            self.assertEqual(install.fill(info["frontmatter"]), fm, name)
            self.assertNotIn("{{RBXKIT", text, name)

    def test_each_built_folder_carries_the_installed_marker(self):
        """A marker the import reads to know Fridae wrote this copy, and whether it was edited since."""
        import rbxskills
        install.build_claude_skills(self.dest)
        for name, files in install.claude_skills().items():
            folder = os.path.join(self.dest, name)
            with open(os.path.join(folder, ".fridae-installed.json"), encoding="utf-8") as f:
                mark = json.load(f)
            self.assertEqual(mark, {"skill": os.path.basename(os.path.dirname(files["SKILL.md"])),
                                    "hash": rbxskills.folder_hash(folder)}, name)
        self.assertEqual(json.load(open(os.path.join(self.dest, "roblox-studio-workflow", ".fridae-installed.json")))["skill"],
                         "workflow")

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
        self.assertTrue(os.path.isfile(os.path.join(home, ".claude", "skills", "roblox-studio-workflow", ".fridae-installed.json")))
        self.assertTrue(os.path.isfile(os.path.join(home, ".claude", "agents", "roblox-tester.md")))


if __name__ == "__main__":
    unittest.main()
