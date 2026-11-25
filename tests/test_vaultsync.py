"""rbxvaultsync.apply: a vault update never overrides the user. Runs in a made-up Knowledge folder."""
import hashlib
import json
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxpaths
import rbxskills
import rbxvaultsync


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


class VaultSyncTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="vaultsync é ")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.know = os.path.join(self.root, "Knowledge")
        p = mock.patch.object(rbxpaths, "KNOWLEDGE_DIR", self.know)
        p.start()
        self.addCleanup(p.stop)
        self.n = 0

    def stage(self, files, release="vault-2026.01.01", extra=None):
        """Write staged/<path> for each file and return (staged, manifest)."""
        self.n += 1
        staged = os.path.join(self.root, "staged%d" % self.n)
        entries = []
        for path, text in sorted(files.items()):
            full = os.path.join(staged, *path.split("/"))
            os.makedirs(os.path.dirname(full), exist_ok=True)
            with open(full, "w", encoding="utf-8", newline="") as f:
                f.write(text)
            entries.append({"path": path, "sha256": sha(text), "size": len(text.encode("utf-8"))})
        os.makedirs(staged, exist_ok=True)
        man = {"format": 1, "release": release, "made": "2026-01-01T00:00:00", "files": entries}
        man.update(extra or {})
        return staged, man

    def local(self, path):
        return os.path.join(self.know, *path.split("/"))

    def put(self, path, text):
        full = self.local(path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8", newline="") as f:
            f.write(text)

    def get(self, path):
        with open(self.local(path), encoding="utf-8", newline="") as f:
            return f.read()

    def snapshot(self):
        out = {}
        for r, ds, fs in os.walk(self.know):
            for d in ds:
                out[os.path.relpath(os.path.join(r, d), self.know)] = None
            for f in fs:
                p = os.path.join(r, f)
                with open(p, "rb") as h:
                    out[os.path.relpath(p, self.know)] = h.read()
        return out

    def apply(self, files, **kw):
        extra = {k: kw.pop(k) for k in list(kw) if k == "extra"}
        staged, man = self.stage(files, **kw)
        return rbxvaultsync.apply(staged, man)

    def seed(self, path, base_text, local_text, release="vault-2025.12.01"):
        """The base record knows path at base_text; the local file holds local_text (None: the user deleted it)."""
        b = rbxvaultsync.read_base()
        b["files"][path] = sha(base_text)
        b["release"] = release
        os.makedirs(rbxvaultsync.state_dir(), exist_ok=True)
        rbxskills._write_json(rbxvaultsync.base_path(), b)
        bc = os.path.join(rbxvaultsync.state_dir(), "base", *path.split("/"))
        os.makedirs(os.path.dirname(bc), exist_ok=True)
        with open(bc, "w", encoding="utf-8", newline="") as f:
            f.write(base_text)
        if local_text is not None:
            self.put(path, local_text)

    P = "Skills/x/SKILL.md"

    def test_first_download_adds_everything(self):
        files = {"Skills/a/SKILL.md": "A", "Skills/a/ref.md": "R", "Skills/b/SKILL.md": "B"}
        r = self.apply(files)
        self.assertEqual(sorted(r["added"]), sorted(files))
        self.assertEqual(r["release"], "vault-2026.01.01")
        self.assertEqual(r["errors"], [])
        base = rbxvaultsync.read_base()
        self.assertEqual(base["files"], {p: sha(t) for p, t in files.items()})
        self.assertEqual(base["release"], "vault-2026.01.01")
        for p, t in files.items():
            self.assertEqual(self.get(p), t)
            with open(os.path.join(rbxvaultsync.state_dir(), "base", *p.split("/")), encoding="utf-8") as f:
                self.assertEqual(f.read(), t)

    def test_unchanged_local_file_is_replaced(self):
        self.seed(self.P, "v1", "v1")
        r = self.apply({self.P: "v2"})
        self.assertEqual(self.get(self.P), "v2")
        self.assertEqual(r["updated"], [self.P])
        self.assertEqual(rbxvaultsync.read_base()["files"][self.P], sha("v2"))

    def test_local_edit_with_no_upstream_change_is_left(self):
        self.seed(self.P, "v1", "mine")
        r = self.apply({self.P: "v1"})
        self.assertEqual(self.get(self.P), "mine")
        for k in ("added", "updated", "merged", "kept", "removed", "skipped_deleted"):
            self.assertEqual(r[k], [], k)

    def test_both_changed_clean_merge(self):
        self.seed(self.P, "a\nb\nc\n", "A\nb\nc\n")
        r = self.apply({self.P: "a\nb\nC\n"})
        self.assertEqual(self.get(self.P), "A\nb\nC\n")
        self.assertEqual(r["merged"], [self.P])
        self.assertEqual(rbxvaultsync.read_base()["files"][self.P], sha("a\nb\nC\n"))

    def test_both_changed_conflict_keeps_mine_saves_beside(self):
        self.seed(self.P, "a\nb\nc\n", "a\nmine\nc\n")
        r = self.apply({self.P: "a\ntheirs\nc\n"})
        self.assertEqual(self.get(self.P), "a\nmine\nc\n")
        self.assertEqual(self.get("Skills/x/SKILL (Fridae update).md"), "a\ntheirs\nc\n")
        self.assertEqual(r["kept"], [self.P])
        self.assertEqual(rbxvaultsync.read_base()["files"][self.P], sha("a\nb\nc\n"))

    def test_user_deleted_file_never_returns(self):
        self.seed(self.P, "v1", None)
        r = self.apply({self.P: "v2"})
        self.assertFalse(os.path.exists(self.local(self.P)))
        self.assertIn(self.P, rbxvaultsync.read_base()["deleted"])
        self.assertEqual(r["skipped_deleted"], [self.P])
        self.apply({self.P: "v3"}, release="vault-2026.02.01")
        self.assertFalse(os.path.exists(self.local(self.P)))

    def test_removed_upstream_unchanged_moves_aside(self):
        self.seed(self.P, "v1", "v1")
        self.seed("Skills/y/SKILL.md", "Y", "Y")
        r = self.apply({"Skills/y/SKILL.md": "Y"}, release="vault-2026.03.01")
        self.assertFalse(os.path.exists(self.local(self.P)))
        moved = os.path.join(rbxvaultsync.state_dir(), "removed", "vault-2026.03.01", "Skills", "x", "SKILL.md")
        with open(moved, encoding="utf-8") as f:
            self.assertEqual(f.read(), "v1")
        self.assertEqual(r["removed"], [self.P])
        self.assertNotIn(self.P, rbxvaultsync.read_base()["files"])

    def test_removed_upstream_edited_is_kept(self):
        self.seed(self.P, "v1", "mine")
        self.seed("Skills/y/SKILL.md", "Y", "Y")
        self.apply({"Skills/y/SKILL.md": "Y"})
        self.assertEqual(self.get(self.P), "mine")
        self.assertNotIn(self.P, rbxvaultsync.read_base()["files"])

    def test_new_upstream_file_where_user_has_one(self):
        self.put(self.P, "mine")
        r = self.apply({self.P: "theirs"})
        self.assertEqual(self.get(self.P), "mine")
        self.assertEqual(self.get("Skills/x/SKILL (Fridae update).md"), "theirs")
        self.assertEqual(r["kept"], [self.P])
        self.assertNotIn(self.P, rbxvaultsync.read_base()["files"])

    def test_bad_paths_refuse_everything(self):
        self.put("Skills/keep/SKILL.md", "k")
        for bad in ("Skills/../x", "/etc/x", "Skills/a/.git/config", "Skills\\a", "Notes/a.md", "Skills//a"):
            before = self.snapshot()
            staged, man = self.stage({"Skills/ok/SKILL.md": "ok"})
            man["files"].append({"path": bad, "sha256": sha("z"), "size": 1})
            with self.assertRaises(ValueError, msg=bad) as cm:
                rbxvaultsync.apply(staged, man)
            self.assertEqual(str(cm.exception), "This vault update looked wrong, so nothing was changed.")
            self.assertEqual(self.snapshot(), before, bad)

    def test_bad_manifest_and_tampered_staged_file(self):
        staged, man = self.stage({self.P: "v"})
        for broken in (dict(man, format=2), dict(man, release=""), dict(man, files="x"),
                       dict(man, files=[{"path": self.P}])):
            with self.assertRaises(ValueError):
                rbxvaultsync.apply(staged, broken)
        with open(os.path.join(staged, "Skills", "x", "SKILL.md"), "w") as f:
            f.write("tampered")
        with self.assertRaises(ValueError):
            rbxvaultsync.apply(staged, man)
        os.remove(os.path.join(staged, "Skills", "x", "SKILL.md"))
        with self.assertRaises(ValueError):
            rbxvaultsync.apply(staged, man)
        self.assertFalse(os.path.exists(self.local(self.P)))

    def test_symlinked_skill_folder_is_refused(self):
        outside = tempfile.mkdtemp(prefix="outside ")
        self.addCleanup(shutil.rmtree, outside, True)
        os.makedirs(os.path.join(self.know, "Skills"))
        os.symlink(outside, os.path.join(self.know, "Skills", "x"))
        with self.assertRaises(ValueError):
            self.apply({self.P: "v", "Skills/z/SKILL.md": "z"})
        self.assertEqual(os.listdir(outside), [])
        self.assertFalse(os.path.exists(self.local("Skills/z/SKILL.md")))

    def test_declined_and_renamed(self):
        b = rbxvaultsync.read_base()
        b["declined"] = ["no"]
        b["renamed"] = {"x": "x-community"}
        os.makedirs(rbxvaultsync.state_dir(), exist_ok=True)
        rbxskills._write_json(rbxvaultsync.base_path(), b)
        r = self.apply({"Skills/no/SKILL.md": "n", self.P: "x"})
        self.assertFalse(os.path.exists(self.local("Skills/no")))
        self.assertEqual(self.get("Skills/x-community/SKILL.md"), "x")
        self.assertFalse(os.path.exists(self.local("Skills/x")))
        self.assertEqual(r["declined"], ["no"])

    def test_newcomer_seam_is_asked_only_for_new_skills(self):
        self.seed(self.P, "v1", "v1")
        with mock.patch.object(rbxvaultsync, "newcomer", return_value="hold") as nc:
            self.apply({self.P: "v2", "Skills/new/SKILL.md": "n"})
        nc.assert_called_once()
        self.assertEqual(nc.call_args[0][0], "new")
        self.assertFalse(os.path.exists(self.local("Skills/new")))
        self.assertEqual(self.get(self.P), "v2")

    def test_cli_exit_2_on_bad_manifest(self):
        import subprocess
        import sys
        staged, man = self.stage({self.P: "v"})
        man["files"][0]["path"] = "Notes/a.md"
        mf = os.path.join(self.root, "m.json")
        with open(mf, "w") as f:
            json.dump(man, f)
        here = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        env = dict(os.environ, FRIDAE_HOME=self.root)
        r = subprocess.run([sys.executable, "rbxskills.py", "skills", "vault-apply", "--staged", staged,
                            "--manifest", mf, "--json"], cwd=here, capture_output=True, text=True, env=env)
        self.assertEqual(r.returncode, 2)
        self.assertIn("This vault update looked wrong, so nothing was changed.", r.stderr)


if __name__ == "__main__":
    unittest.main()
