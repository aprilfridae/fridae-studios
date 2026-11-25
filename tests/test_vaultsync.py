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

    def cli(self, staged, manifest, *more):
        """Run `skills vault-apply` in-process on the made-up Knowledge folder: (exit code, stdout, stderr)."""
        import argparse
        import contextlib
        import io
        mf = os.path.join(self.root, "m.json")
        with open(mf, "w") as f:
            json.dump(manifest, f)
        ap = argparse.ArgumentParser()
        rbxskills.register(ap.add_subparsers(dest="cmd", required=True))
        a = ap.parse_args(["skills", "vault-apply", "--staged", staged, "--manifest", mf, "--json"] + list(more))
        out, err = io.StringIO(), io.StringIO()
        code = 0
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            try:
                a.fn(a)
            except SystemExit as e:
                code = e.code if isinstance(e.code, int) else 1
        return code, out.getvalue(), err.getvalue()

    def test_cli_success_prints_result(self):
        staged, man = self.stage({self.P: "v"})
        code, out, err = self.cli(staged, man)
        self.assertEqual(code, 0, err)
        self.assertEqual(json.loads(out)["added"], [self.P])
        self.assertEqual(self.get(self.P), "v")

    def test_cli_exit_2_on_bad_manifest(self):
        staged, man = self.stage({self.P: "v"})
        man["files"][0]["path"] = "Notes/a.md"
        code, out, err = self.cli(staged, man)
        self.assertEqual(code, 2)
        self.assertIn("This vault update looked wrong, so nothing was changed.", err)

    def test_cli_refuses_empty_staged_and_plain_errors(self):
        staged, man = self.stage({self.P: "v"})
        self.assertEqual(self.cli("", man)[0], 2)
        with mock.patch.object(rbxvaultsync, "apply", side_effect=OSError("disk full")):
            code, out, err = self.cli(staged, man)
        self.assertNotEqual(code, 0)
        self.assertIn("disk full", err)
        self.assertNotIn("Traceback", err)

    def test_unreadable_or_non_file_target_is_never_overwritten(self):
        self.put(self.P, "MY WORK")
        os.chmod(self.local(self.P), 0)
        self.addCleanup(os.chmod, self.local(self.P), 0o644)
        os.makedirs(self.local("Skills/x/dir.md"))
        r = self.apply({self.P: "theirs", "Skills/x/dir.md": "d"})
        os.chmod(self.local(self.P), 0o644)
        self.assertEqual(self.get(self.P), "MY WORK")
        self.assertTrue(os.path.isdir(self.local("Skills/x/dir.md")))
        self.assertEqual(len(r["errors"]), 2)
        base = rbxvaultsync.read_base()
        self.assertEqual(base["deleted"], [])
        self.assertEqual(base["files"], {})

    def test_unreadable_target_with_base_entry_is_not_marked_deleted(self):
        self.seed(self.P, "v1", "v1")
        os.chmod(self.local(self.P), 0)
        self.addCleanup(os.chmod, self.local(self.P), 0o644)
        r = self.apply({self.P: "v2"})
        os.chmod(self.local(self.P), 0o644)
        self.assertEqual(len(r["errors"]), 1)
        self.assertEqual(rbxvaultsync.read_base()["deleted"], [])
        self.assertEqual(self.get(self.P), "v1")

    def test_existing_beside_file_is_not_replaced(self):
        self.seed(self.P, "a\nb\nc\n", "a\nmine\nc\n")
        self.put("Skills/x/SKILL (Fridae update).md", "my hand merge")
        r = self.apply({self.P: "a\ntheirs\nc\n"})
        self.assertEqual(self.get("Skills/x/SKILL (Fridae update).md"), "my hand merge")
        self.assertEqual(self.get("Skills/x/SKILL (Fridae update 2).md"), "a\ntheirs\nc\n")
        self.assertEqual(r["kept"], [self.P])
        self.apply({self.P: "a\ntheirs\nc\n"}, release="vault-2026.02.01")  # same bytes: no third copy
        self.assertFalse(os.path.exists(self.local("Skills/x/SKILL (Fridae update 3).md")))

    def refused(self, files=None, mutate=None):
        staged, man = self.stage(files or {self.P: "v"})
        if mutate:
            mutate(man)
        before = self.snapshot()
        with self.assertRaises(ValueError):
            rbxvaultsync.apply(staged, man)
        self.assertEqual(self.snapshot(), before)

    def test_beside_named_manifest_path_is_refused(self):
        self.refused({"Skills/x/SKILL (Fridae update).md": "v"})

    def test_manifest_limits(self):
        """Each case has real, matching staged files, so only the limit can refuse it, and no staged file is read."""
        def boom(path):
            raise AssertionError("a staged file was read before the limits were checked")
        cases = [("MAX_FILES", 2, {"Skills/s/a.md": "a", "Skills/s/b.md": "b", "Skills/s/c.md": "c"}),
                 ("MAX_FILE", 5, {"Skills/s/a.md": "123456"}),
                 ("MAX_TOTAL", 10, {"Skills/s/a.md": "1234", "Skills/s/b.md": "1234", "Skills/s/c.md": "1234"})]
        for name, limit, files in cases:
            staged, man = self.stage(files)
            before = self.snapshot()
            with mock.patch.object(rbxvaultsync, name, limit), mock.patch.object(rbxvaultsync, "_read", boom):
                with self.assertRaises(ValueError, msg=name):
                    rbxvaultsync.apply(staged, man)
            self.assertEqual(self.snapshot(), before)
            with mock.patch.object(rbxvaultsync, name, limit * 10):  # the same files pass when the limit allows
                rbxvaultsync.apply(staged, man)
            shutil.rmtree(self.know)
        self.refused(mutate=lambda m: m.update(files=[]))

    def test_longest_accepted_name_really_writes(self):
        ok = "Skills/y/%s.md" % ("n" * (rbxvaultsync.NAME_MAX - 3))
        self.assertEqual(len(ok.rsplit("/", 1)[1].encode()), rbxvaultsync.NAME_MAX)
        self.seed(ok, "a\nb\nc\n", "a\nmine\nc\n")
        self.put(ok.replace(".md", " (Fridae update).md"), "hand merge")
        self.apply({ok: "a\ntheirs\nc\n"})
        self.assertTrue(os.path.exists(self.local(ok.replace(".md", " (Fridae update 2).md"))))
        self.refused({"Skills/z/%s.md" % ("n" * (rbxvaultsync.NAME_MAX - 2)): "v"})

    def test_removing_through_a_symlinked_base_copy_is_refused(self):
        outside = tempfile.mkdtemp(prefix="outside ")
        self.addCleanup(shutil.rmtree, outside, True)
        self.seed(self.P, "v1", "v1")
        self.seed("Skills/y/SKILL.md", "Y", "Y")
        folder = os.path.join(rbxvaultsync.state_dir(), "base", "Skills", "x")
        shutil.move(os.path.join(folder, "SKILL.md"), os.path.join(outside, "SKILL.md"))
        os.rmdir(folder)
        os.symlink(outside, folder)  # base/Skills/x is now a link to a folder outside the record
        self.apply({"Skills/y/SKILL.md": "Y"})
        self.assertTrue(os.path.exists(os.path.join(outside, "SKILL.md")))
        self.assertTrue(os.path.islink(folder))

    def test_upstream_paths_equal_after_casefold_are_refused(self):
        """The two upstream paths map to different local files, so only the upstream check can refuse them."""
        b = rbxvaultsync.read_base()
        b["renamed"] = {"X": "x-big"}
        os.makedirs(rbxvaultsync.state_dir(), exist_ok=True)
        rbxskills._write_json(rbxvaultsync.base_path(), b)
        # same bytes: they are one staged file on a case-insensitive disk
        self.refused({"Skills/X/a.md": "same", "Skills/x/a.md": "same"})

    def test_knowledge_itself_may_be_a_symlink(self):
        real = tempfile.mkdtemp(prefix="portable ")
        self.addCleanup(shutil.rmtree, real, True)
        os.symlink(real, self.know)
        r = self.apply({self.P: "v"})
        self.assertEqual(r["added"], [self.P])
        self.assertEqual(self.get(self.P), "v")
        self.assertTrue(os.path.exists(os.path.join(real, "Skills", "x", "SKILL.md")))

    def test_loose_file_under_skills_is_refused(self):
        self.assertFalse(rbxvaultsync.check_path("Skills/README.md"))
        self.refused({"Skills/README.md": "v"})

    def test_two_paths_for_one_local_file_are_refused(self):
        b = rbxvaultsync.read_base()
        b["renamed"] = {"x": "x-community"}
        os.makedirs(rbxvaultsync.state_dir(), exist_ok=True)
        rbxskills._write_json(rbxvaultsync.base_path(), b)
        self.refused({self.P: "a", "Skills/x-community/SKILL.md": "b"})
        self.refused({"Skills/y/SKILL.md": "a", "Skills/y/skill.md": "b"})

    def test_symlinked_vault_folders_are_refused(self):
        outside = tempfile.mkdtemp(prefix="outside ")
        self.addCleanup(shutil.rmtree, outside, True)
        os.makedirs(self.know)
        os.symlink(outside, os.path.join(self.know, "Skills"))
        with self.assertRaises(ValueError):
            self.apply({self.P: "v"})
        os.remove(os.path.join(self.know, "Skills"))
        os.symlink(outside, os.path.join(self.know, ".fridae-vault"))
        with self.assertRaises(ValueError):
            self.apply({self.P: "v"})
        self.assertEqual(os.listdir(outside), [])

    def test_base_is_saved_after_each_skill_folder(self):
        b = rbxvaultsync.read_base()
        b["declined"] = ["b"]
        os.makedirs(rbxvaultsync.state_dir(), exist_ok=True)
        rbxskills._write_json(rbxvaultsync.base_path(), b)
        with mock.patch.object(rbxvaultsync, "_save_base", wraps=rbxvaultsync._save_base) as sv:
            self.apply({"Skills/a/SKILL.md": "a", "Skills/b/SKILL.md": "b", "Skills/c/SKILL.md": "c"})
        self.assertEqual(sv.call_count, 4)  # a, b (declined), c, then the end

    def test_one_state_dir(self):
        self.assertEqual(rbxvaultsync.state_dir(), rbxskills.vault_state_dir())


if __name__ == "__main__":
    unittest.main()
