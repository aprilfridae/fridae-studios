"""What the build bundles from vault/: never the Roblox guides (they come from the Fridae vault into Knowledge)."""
import os
import shutil
import tempfile
import unittest

import build_datas


class BuildDatasTests(unittest.TestCase):
    def test_guides_are_never_bundled(self):
        here = tempfile.mkdtemp(prefix="build datas ")
        self.addCleanup(shutil.rmtree, here, True)
        made = {}
        for rel in ("vault/Roblox/a.md", "vault/Roblox/_tools/x.json", "vault/Fridae/Data/roblox_api.json",
                    "vault/Fridae/Guides/Core/s/SKILL.md", "vault/.obsidian/y.json", "vault/Fridae/.DS_Store"):
            path = os.path.join(here, *rel.split("/"))
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write("x")
            made[rel] = path
        got = sorted(build_datas.vault_datas(here))
        self.assertEqual(got, sorted([
            (made["vault/Fridae/Data/roblox_api.json"], os.path.join("vault", "Fridae", "Data")),
            (made["vault/Fridae/Guides/Core/s/SKILL.md"], os.path.join("vault", "Fridae", "Guides", "Core", "s")),
        ]))


if __name__ == "__main__":
    unittest.main()
