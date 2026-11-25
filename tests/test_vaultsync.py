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
import rbxvaultnew
import rbxvaultsync


def sha(text):
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


# Names Windows would read as a drive, a stream or a device, or would change: the same table as vault_sync.rs's
# WINDOWS_BAD.
WINDOWS_BAD = (
    "Skills/a/C:x.md",
    "Skills/C:/x.md",
    "Skills/a/SKILL.md:s",
    "Skills/CON/x.md",
    "Skills/a/nul.txt",
    "Skills/a/com1",
    "Skills/a./x.md",
    "Skills/a /x.md",
    "Skills/a/x.md.",
    "Skills/a/CONIN$",
    "Skills/a/conout$.md",
    "Skills/a/COM\u00b9",
    "Skills/a/lpt\u00b3.md",
    "Skills/a/LPT0",
    "Skills/a/aux .md",
    "Skills/ a/x.md",
    "Skills/a/x<y.md",
    "Skills/a/x?.md",
    "Skills/a/x|y.md",
    "Skills/a/x\x01y.md",
    "Roblox/CON.md",
    "Roblox/a:b.md",
)
WINDOWS_GOOD = ("Skills/a/console.md", "Skills/a/com10", "Skills/con-tips/SKILL.md", "Skills/a/nul-thing.md",
                "Skills/a/x.y.md", "Skills/a/comb", "Roblox/con-tips.md")


class VaultSyncTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="vaultsync é ")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.know = os.path.join(self.root, "Knowledge")
        p = mock.patch.object(rbxpaths, "KNOWLEDGE_DIR", self.know)
        p.start()
        self.addCleanup(p.stop)
        self.home = os.path.join(self.root, "home")
        self.kit = os.path.join(self.root, "kit")
        for q in (mock.patch.object(rbxpaths, "HOME_DIR", self.home), mock.patch.object(rbxskills, "KIT", self.kit)):
            q.start()
            self.addCleanup(q.stop)
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
        for bad in ("Skills/../x", "/etc/x", "Skills/a/.git/config", "Skills\\a", "Skills//a", "House rules.md",
                    "Glossary.md", ".fridae-vault/base.json", "Roblox/.obsidian/x.json", "Roblox/_tools/roblox_api.json",
                    "Roblox/_data/roblox_facts_2026.json", "Roblox", "Roblox/"):
            before = self.snapshot()
            staged, man = self.stage({"Skills/ok/SKILL.md": "ok"})
            man["files"].append({"path": bad, "sha256": sha("z"), "size": 1})
            with self.assertRaises(ValueError, msg=bad) as cm:
                rbxvaultsync.apply(staged, man)
            self.assertEqual(str(cm.exception), "This vault update looked wrong, so nothing was changed.")
            self.assertEqual(self.snapshot(), before, bad)

    def test_guides_and_topic_folders_travel(self):
        for good in ("Roblox/Glossary.md", "Roblox/02 Luau Scripting/Buffers and Binary Data.md",
                     "Roblox/18 Rigs and Skeletons/Spider_8Leg/Spider_8Leg.fbx", "Notes/a.md", "Taste/Softer.md",
                     "Inbox/x.md"):
            self.assertTrue(rbxvaultsync.check_path(good), good)
        self.assertEqual(rbxvaultsync.unit_of("Skills/a/ref/x.md"), "Skills/a")
        self.assertEqual(rbxvaultsync.unit_of("Roblox/Glossary.md"), "Roblox/Glossary.md")

    def test_renamed_and_declined_touch_only_skills(self):
        self.assertEqual(rbxvaultsync._local_path("Roblox/x/y.md", {"x": "z"}), "Roblox/x/y.md")
        self.assertEqual(rbxvaultsync._skill("Roblox/x/y.md"), "")
        self.assertEqual(rbxvaultsync._local_path("Skills/x/y.md", {"x": "z"}), "Skills/z/y.md")
        self.assertEqual(rbxvaultsync._skill("Skills/x/y.md"), "x")

    def test_windows_names_refuse_everything(self):
        for bad in WINDOWS_BAD:
            self.assertFalse(rbxvaultsync.check_path(bad), bad)
        for good in WINDOWS_GOOD:
            self.assertTrue(rbxvaultsync.check_path(good), good)
        self.put("Skills/keep/SKILL.md", "k")
        for bad in WINDOWS_BAD[:3]:
            before = self.snapshot()
            staged, man = self.stage({"Skills/ok/SKILL.md": "ok"})
            man["files"].append({"path": bad, "sha256": sha("z"), "size": 1})
            with self.assertRaises(ValueError, msg=bad):
                rbxvaultsync.apply(staged, man)
            self.assertEqual(self.snapshot(), before, bad)

    def test_merge_runs_git_without_a_console_window(self):
        done = mock.Mock(returncode=0, stdout=b"merged")
        with mock.patch.object(rbxvaultsync.subprocess, "run", return_value=done) as run:
            self.assertEqual(rbxvaultsync._merge(b"a", b"b", b"c"), b"merged")
        self.assertEqual(run.call_args.kwargs.get("creationflags"), rbxpaths.no_window())

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
        r = self.apply({"Skills/no/SKILL.md": "n", self.P: "---\nname: x\n---\nx"})
        self.assertFalse(os.path.exists(self.local("Skills/no")))
        self.assertEqual(self.get("Skills/x-community/SKILL.md"), "---\nname: x-community\n---\nx")
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
        man["files"][0]["path"] = "House rules.md"
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
        """The real limits, from the manifest alone: a refused manifest reads no staged file, and one within the
        limits goes on to read them (and finds none)."""
        self.assertEqual((rbxvaultsync.MAX_FILES, rbxvaultsync.MAX_FILE, rbxvaultsync.MAX_TOTAL),
                         (20_000, 8 << 20, 256 << 20))

        def boom(path):
            raise AssertionError("a staged file was read")

        def passes(entries):
            staged = os.path.join(self.root, "nothing staged")
            os.makedirs(staged, exist_ok=True)
            man = {"format": 1, "release": "vault-2026.01.01", "files": entries}
            with mock.patch.object(rbxvaultsync, "_read", boom):
                try:
                    rbxvaultsync.apply(staged, man)
                except AssertionError:
                    return True  # past the limits: it went on to read a staged file
                except ValueError:
                    return False
            raise AssertionError("neither refused nor read")

        def files(n, size=1):
            return [{"path": "Skills/s%d/SKILL.md" % i, "sha256": sha("x"), "size": size} for i in range(n)]

        before = self.snapshot()
        self.assertFalse(passes(files(20_001)), "20,001 files")
        self.assertTrue(passes(files(20_000)), "20,000 files")
        self.assertTrue(passes(files(1, 8 << 20)), "8 MiB")
        self.assertFalse(passes(files(1, (8 << 20) + 1)), "8 MiB and a byte")
        self.assertFalse(passes(files(33, 8 << 20)), "over 256 MiB in all")
        self.assertTrue(passes(files(32, 8 << 20)), "256 MiB in all")
        self.assertEqual(self.snapshot(), before)
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

    # ---- a new skill that meets the library
    DESC = "Find the root cause of a bug before fixing it. Use when a test fails or something breaks."

    def user_skill(self, name, desc=DESC, body="Mine.", extra=None):
        d = os.path.join(rbxskills.user_dir(), name)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: %s\ndescription: %s\n---\n\n%s\n" % (name, desc, body))
        for rel, text in (extra or {}).items():
            with open(os.path.join(d, rel), "w", encoding="utf-8") as f:
                f.write(text)
        return d

    def kit_skill(self, name, owner, desc="Something else entirely about crochet."):
        d = os.path.join(self.kit, "vault", "Fridae", "Guides", "Core", name)
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, "SKILL.md"), "w", encoding="utf-8") as f:
            f.write("---\nname: %s\ndescription: %s\nowner: %s\n---\n\nKit.\n" % (name, desc, owner))
        return d

    def vfiles(self, name, desc=DESC, body="Theirs."):
        return {"Skills/%s/SKILL.md" % name: "---\nname: %s\ndescription: %s\n---\n\n%s\n" % (name, desc, body),
                "Skills/%s/ref.md" % name: "ref of " + name}

    def waiting_one(self, **kw):
        """A user debugging and a vault debug-helper that overlaps it; returns the one open decision."""
        self.user_skill("debugging")
        r = self.apply(self.vfiles("debug-helper", **kw))
        self.assertEqual(r["incoming"], ["debug-helper"])
        (dec,) = rbxskills.pending()
        return dec

    def test_overlapping_newcomer_waits(self):
        dec = self.waiting_one()
        self.assertFalse(os.path.exists(self.local("Skills/debug-helper")))
        self.assertEqual(dec["from"], "vault")
        self.assertEqual((dec["a"], dec["b"], dec["reason"]), ("debugging", "debug-helper", "overlap"))
        self.assertEqual(dec["source"], "Fridae vault vault-2026.01.01")
        self.assertEqual(dec["b_dir"], os.path.join(rbxvaultsync.incoming_dir(), "debug-helper"))
        self.assertTrue(os.path.isfile(os.path.join(dec["b_dir"], "ref.md")))
        self.assertEqual([e["id"] for e in rbxskills.library()], ["debugging"])
        self.assertEqual(rbxvaultsync.read_base()["waiting"], {"debug-helper": "vault-2026.01.01"})
        self.assertEqual(rbxvaultsync.read_base()["files"], {})

    def test_replace_moves_it_in_and_sets_mine_aside(self):
        dec = self.waiting_one()
        rbxvaultsync.decide_vault(dec, "replace", None)
        lib = {e["id"]: e for e in rbxskills.library()}
        self.assertEqual(list(lib), ["debug-helper"])
        self.assertEqual(lib["debug-helper"]["origin"], "vault")
        self.assertTrue(os.path.isdir(os.path.join(rbxskills.user_dir(), ".replaced", "debugging")))
        base = rbxvaultsync.read_base()
        for f in ("SKILL.md", "ref.md"):
            p = "Skills/debug-helper/" + f
            self.assertEqual(base["files"][p], sha(self.get(p)))
            with open(os.path.join(rbxvaultsync.state_dir(), "base", *p.split("/")), encoding="utf-8") as h:
                self.assertEqual(h.read(), self.get(p))
        self.assertEqual(base["waiting"], {})
        self.assertFalse(os.path.exists(dec["b_dir"]))
        self.assertEqual(rbxskills.pending(), [])
        self.apply(self.vfiles("debug-helper", body="Theirs v2."), release="vault-2026.02.01")  # follows upstream
        self.assertIn("Theirs v2.", self.get("Skills/debug-helper/SKILL.md"))

    def test_accept_all_settles_vault_decisions(self):
        dec = self.waiting_one()
        done = rbxskills.decide(dec["id"], "replace")
        self.assertEqual(done["state"], "done")
        self.assertEqual([e["id"] for e in rbxskills.library()], ["debug-helper"])

    def test_keep_declines_future_versions(self):
        dec = self.waiting_one()
        rbxskills.decide(dec["id"], "keep")
        base = rbxvaultsync.read_base()
        self.assertIn("debug-helper", base["declined"])
        self.assertEqual(base["waiting"], {})
        self.assertFalse(os.path.exists(dec["b_dir"]))
        self.assertTrue(os.path.isdir(os.path.join(rbxvaultsync.state_dir(), "removed")))
        self.assertTrue(os.path.isdir(os.path.join(rbxskills.user_dir(), "debugging")))
        before = self.snapshot()
        r = self.apply(self.vfiles("debug-helper", body="v2"), release="vault-2026.02.01")
        self.assertEqual(r["declined"], ["debug-helper"])
        after = self.snapshot()
        after.pop(os.path.join(".fridae-vault", "base.json"))
        before.pop(os.path.join(".fridae-vault", "base.json"))
        self.assertEqual(after, before)

    def test_keep_both_renames(self):
        dec = self.waiting_one()
        rbxskills.decide(dec["id"], "keep_both")
        self.assertEqual(rbxvaultsync.read_base()["renamed"], {"debug-helper": "debug-helper-community"})
        self.assertIn("name: debug-helper-community", self.get("Skills/debug-helper-community/SKILL.md"))
        self.assertEqual(sorted(e["id"] for e in rbxskills.library()), ["debug-helper-community", "debugging"])
        self.assertEqual(rbxvaultsync.read_base()["files"]["Skills/debug-helper/ref.md"], sha("ref of debug-helper"))
        self.apply({"Skills/debug-helper/SKILL.md": self.get("Skills/debug-helper-community/SKILL.md"),
                    "Skills/debug-helper/ref.md": "ref v2"}, release="vault-2026.02.01")
        self.assertEqual(self.get("Skills/debug-helper-community/ref.md"), "ref v2")
        self.assertFalse(os.path.exists(self.local("Skills/debug-helper")))

    def test_keep_both_name_taken_goes_to_next_free(self):
        dec = self.waiting_one()
        self.put("Skills/debug-helper-community/SKILL.md", "x")
        rbxskills.decide(dec["id"], "keep_both")
        self.assertEqual(rbxvaultsync.read_base()["renamed"], {"debug-helper": "debug-helper-community-2"})

    def test_own_share_comes_back_without_a_card(self):
        d = self.user_skill("tidy", "Tidy up a project folder.", extra={"notes.md": "n"})
        h = rbxskills.folder_hash(d)
        rbxskills._write_json(os.path.join(d, rbxskills.SHARED_FILE),
                              {"sent": [{"name": "Tidy", "hash": h, "pr": "https://x/1", "at": 1}]})
        r = self.apply(self.vfiles("tidy", "Tidy up a project folder."))
        self.assertEqual(r["settled"], ["tidy"])
        self.assertEqual(len(r["added"]), 2)
        self.assertTrue(os.path.isdir(os.path.join(rbxskills.user_dir(), ".replaced", "tidy")))
        self.assertFalse(os.path.exists(d))
        self.assertEqual(rbxskills.pending(), [])
        self.assertEqual([e["origin"] for e in rbxskills.library()], ["vault"])

    def test_edited_share_opens_a_decision(self):
        d = self.user_skill("tidy", "Tidy up a project folder.")
        h = rbxskills.folder_hash(d)
        rbxskills._write_json(os.path.join(d, rbxskills.SHARED_FILE), {"sent": [{"name": "tidy", "hash": h}]})
        with open(os.path.join(d, "extra.md"), "w") as f:
            f.write("edited after sending")
        r = self.apply(self.vfiles("tidy", "Tidy up a project folder."))
        self.assertEqual(r["incoming"], ["tidy"])
        self.assertEqual(r["settled"], [])
        self.assertTrue(os.path.isdir(d))
        self.assertEqual(len(rbxskills.pending()), 1)

    def test_app_name_is_declined(self):
        self.kit_skill("critic", "app")
        r = self.apply({"Skills/critic/SKILL.md": "---\nname: critic\n---\nx", "Skills/fine/SKILL.md": "f"})
        self.assertEqual(r["declined"], ["critic"])
        self.assertFalse(os.path.exists(self.local("Skills/critic")))
        self.assertIn("critic", rbxvaultsync.read_base()["declined"])
        self.assertTrue(os.path.exists(self.local("Skills/fine/SKILL.md")))

    def test_app_only_name_is_declined_and_never_listed(self):
        r = self.apply({"Skills/docs/SKILL.md": "---\nname: docs\n---\nx"})
        self.assertEqual(r["declined"], ["docs"])
        self.assertIn("docs", rbxvaultsync.read_base()["declined"])
        self.assertFalse(os.path.exists(self.local("Skills/docs")))
        self.put("Skills/computer-use/SKILL.md", "---\nname: computer-use\ndescription: d\n---\nx")
        self.assertEqual(rbxskills.library(), [])

    def test_identical_body_is_declined_as_already_had(self):
        self.user_skill("mine", "Totally different words here.", body="Same body.\nTwo lines.  ")
        files = {"Skills/theirs/SKILL.md": "---\nname: theirs\ndescription: Other words about crochet.\n---\n"
                                           "\nSame body.  \nTwo lines.\n\n"}
        r = self.apply(files)
        self.assertEqual(r["declined"], ["theirs"])
        self.assertFalse(os.path.exists(self.local("Skills/theirs")))
        self.assertNotIn("theirs", rbxvaultsync.read_base()["declined"])
        self.assertEqual(rbxskills.pending(), [])

    def test_unrelated_newcomer_installs(self):
        self.user_skill("debugging")
        r = self.apply(self.vfiles("crochet", "Stitch a granny square blanket with wool."))
        self.assertEqual(len(r["added"]), 2)
        self.assertEqual(rbxskills.pending(), [])

    def test_newer_release_replaces_the_waiting_copy_and_clears_the_recommendation(self):
        dec = self.waiting_one()
        decs = rbxskills._decisions()
        decs[0]["recommendation"] = {"choice": "replace"}
        rbxskills._save_decisions(decs)
        r = self.apply(self.vfiles("debug-helper", body="Theirs v2."), release="vault-2026.02.01")
        self.assertEqual(r["incoming"], ["debug-helper"])
        self.assertFalse(os.path.exists(self.local("Skills/debug-helper")))
        with open(os.path.join(dec["b_dir"], "SKILL.md"), encoding="utf-8") as f:
            self.assertIn("Theirs v2.", f.read())
        (again,) = rbxskills.pending()
        self.assertEqual(again["id"], dec["id"])
        self.assertIsNone(again["recommendation"])
        self.assertEqual(rbxvaultsync.read_base()["waiting"], {"debug-helper": "vault-2026.02.01"})
        self.assertEqual(len(rbxskills._decisions()), 1)

    def test_replace_a_shipped_skill_supersedes_it(self):
        self.kit_skill("debugging", "shipped", self.DESC)
        self.apply(self.vfiles("debug-helper"))
        (dec,) = rbxskills.pending()
        rbxskills.decide(dec["id"], "replace")
        self.assertEqual(rbxvaultsync.read_base()["patch"]["debug-helper"]["supersedes"], ["debugging"])
        self.assertEqual([e["id"] for e in rbxskills.library()], ["debug-helper"])

    def test_replace_an_app_skill_is_refused(self):
        self.kit_skill("debugging", "app", self.DESC)
        self.apply(self.vfiles("debug-helper"))
        (dec,) = rbxskills.pending()
        with self.assertRaises(ValueError) as cm:
            rbxskills.decide(dec["id"], "replace")
        self.assertEqual(str(cm.exception), rbxskills.APP_SKILL)
        self.assertTrue(os.path.isdir(dec["b_dir"]))
        self.assertEqual(len(rbxskills.pending()), 1)

    def test_merge_edits_the_one_in_use_and_declines(self):
        dec = self.waiting_one()
        rbxskills.decide(dec["id"], "merge", "---\nname: debugging\ndescription: Merged.\n---\n\nBoth.\n")
        mine = os.path.join(rbxskills.user_dir(), "debugging", "SKILL.md")
        with open(mine, encoding="utf-8") as f:
            self.assertIn("Both.", f.read())
        self.assertTrue(os.path.isdir(os.path.join(rbxskills.user_dir(), ".replaced", "debugging")))
        base = rbxvaultsync.read_base()
        self.assertIn("debug-helper", base["declined"])
        self.assertEqual(base["waiting"], {})
        self.assertFalse(os.path.exists(dec["b_dir"]))
        self.assertFalse(os.path.exists(self.local("Skills/debug-helper")))

    def test_decide_vault_refuses_a_folder_outside_the_known_places(self):
        dec = self.waiting_one()
        outside = os.path.join(self.root, "elsewhere")
        os.makedirs(outside)
        decs = rbxskills._decisions()
        decs[0]["b_dir"] = outside
        rbxskills._save_decisions(decs)
        for choice in ("replace", "keep", "keep_both"):
            with self.assertRaises(ValueError):
                rbxskills.decide(dec["id"], choice)
        self.assertTrue(os.path.isdir(outside))

    def test_locks_are_taken_vault_first_then_library(self):
        dec = self.waiting_one()
        order = []
        real = rbxskills._locked

        import contextlib

        @contextlib.contextmanager
        def spy(root):
            order.append(os.path.basename(root))
            with real(root):
                yield
        with mock.patch.object(rbxskills, "_locked", spy):
            rbxskills.decide(dec["id"], "keep")
        self.assertEqual(order, [".fridae-vault", "skills"])

    def test_upstream_change_reaches_a_keep_both_copy_cleanly(self):
        dec = self.waiting_one()
        rbxskills.decide(dec["id"], "keep_both")
        base = rbxvaultsync.read_base()
        up = self.vfiles("debug-helper")["Skills/debug-helper/SKILL.md"]
        self.assertEqual(base["files"]["Skills/debug-helper/SKILL.md"], sha(up))  # upstream's hash, for the fetcher
        r = self.apply(self.vfiles("debug-helper", desc=self.DESC + " Also for crashes."), release="vault-2026.02.01")
        p = "Skills/debug-helper/SKILL.md"
        self.assertTrue(p in r["updated"] or p in r["merged"], r)
        self.assertEqual(r["kept"], [])
        text = self.get("Skills/debug-helper-community/SKILL.md")
        self.assertIn("name: debug-helper-community", text)
        self.assertIn("Also for crashes.", text)
        self.assertFalse(os.path.exists(self.local("Skills/debug-helper-community/SKILL (Fridae update).md")))

    def test_replace_a_vault_skill_moves_it_to_removed_and_declines_it(self):
        OLD = self.vfiles("old-way", "Stitch a granny square blanket with wool.")
        self.apply(OLD)
        self.apply({**OLD, **self.vfiles("debug-helper", "Stitch a granny square blanket with wool. Use it for wool.",
                                         body="Newer.")}, release="vault-2026.02.01")
        (dec,) = rbxskills.pending()
        self.assertEqual(dec["a"], "old-way")
        rbxskills.decide(dec["id"], "replace")
        self.assertEqual([e["id"] for e in rbxskills.library()], ["debug-helper"])
        self.assertTrue(os.path.isdir(os.path.join(rbxvaultsync.state_dir(), "removed", "vault-2026.02.01",
                                                   "Skills", "old-way")))
        self.assertFalse(os.path.exists(self.local("Skills/old-way")))
        base = rbxvaultsync.read_base()
        self.assertIn("old-way", base["declined"])
        self.assertEqual([p for p in base["files"] if "old-way" in p], [])
        self.assertIn("Skills/debug-helper/SKILL.md", base["files"])

    def test_library_is_read_once_for_thirty_newcomers(self):
        words = ["alpha", "bravo", "cobalt", "delta", "ember", "fjord", "garnet", "harbor", "indigo", "juniper"]
        files = {}
        for i in range(30):
            n = "s%02d%s" % (i, words[i % 10])
            files.update(self.vfiles(n, "%s %s %s unique%d tale" % (words[i % 10], words[(i * 3 + 1) % 10], n, i * 7919),
                                     body="Body number %d about %s." % (i, n)))
        with mock.patch.object(rbxskills, "library", wraps=rbxskills.library) as lib:
            r = self.apply(files)
        self.assertEqual(len(r["added"]), 60)
        self.assertEqual(lib.call_count, 1)

    def test_a_second_newcomer_sees_the_first_one_installed(self):
        files = {**self.vfiles("first-one", body="One."), **self.vfiles("first-two", body="Two.")}
        r = self.apply(files)
        self.assertEqual(r["incoming"], ["first-two"])
        self.assertTrue(os.path.exists(self.local("Skills/first-one/SKILL.md")))

    def test_upstream_rename_same_body_installs_the_new_name(self):
        self.apply(self.vfiles("old-name", "Stitch a granny square blanket with wool."))
        r = self.apply(self.vfiles("new-name", "Stitch a granny square blanket with wool.", body="Theirs."),
                       release="vault-2026.02.01")
        self.assertEqual(r["declined"], [])
        self.assertEqual([e["id"] for e in rbxskills.library()], ["new-name"])
        self.assertEqual(rbxskills.pending(), [])

    def test_upstream_rename_edited_body_asks_no_card(self):
        self.apply(self.vfiles("old-name", "Stitch a granny square blanket with wool.", body="Old."))
        r = self.apply(self.vfiles("new-name", "Stitch a granny square blanket with wool.", body="Edited."),
                       release="vault-2026.02.01")
        self.assertEqual(r["incoming"], [])
        self.assertEqual([e["id"] for e in rbxskills.library()], ["new-name"])
        self.assertEqual(rbxskills.pending(), [])

    def failing_second_record(self):
        real, calls = rbxvaultnew._record_base, []

        def fake(*a, **k):
            calls.append(1)
            if len(calls) == 2:
                raise OSError("disk full")
            return real(*a, **k)
        return mock.patch.object(rbxvaultnew, "_record_base", fake)

    def test_failed_replace_is_rolled_back_whole(self):
        dec = self.waiting_one()
        before = [e["id"] for e in rbxskills.library()]
        with self.failing_second_record(), self.assertRaises(OSError):
            rbxskills.decide(dec["id"], "replace")
        self.assertEqual([e["id"] for e in rbxskills.library()], before)
        self.assertEqual([d["id"] for d in rbxskills.pending()], [dec["id"]])
        self.assertTrue(os.path.isfile(os.path.join(dec["b_dir"], "ref.md")))
        self.assertFalse(os.path.exists(self.local("Skills/debug-helper")))
        self.assertEqual(rbxvaultsync.read_base()["files"], {})
        self.assertTrue(os.path.isdir(os.path.join(rbxskills.user_dir(), "debugging")))
        rbxskills.decide(dec["id"], "replace")  # and it can be tried again
        self.assertEqual([e["id"] for e in rbxskills.library()], ["debug-helper"])

    def test_failed_keep_both_is_rolled_back_whole(self):
        dec = self.waiting_one()
        with self.failing_second_record(), self.assertRaises(OSError):
            rbxskills.decide(dec["id"], "keep_both")
        self.assertEqual([e["id"] for e in rbxskills.library()], ["debugging"])
        self.assertTrue(os.path.isdir(dec["b_dir"]))
        self.assertEqual(rbxvaultsync.read_base()["renamed"], {})

    def test_library_lock_timeout_on_a_waiting_skill_does_not_abort_the_update(self):
        self.waiting_one()
        with mock.patch.object(rbxvaultnew, "_renew", side_effect=rbxpaths.KitError("busy")):
            r = self.apply({**self.vfiles("debug-helper", body="v2"), **self.vfiles("crochet", "Stitch wool.")},
                           release="vault-2026.02.01")
        self.assertEqual(len(r["errors"]), 1)
        self.assertTrue(os.path.exists(self.local("Skills/crochet/SKILL.md")))

    def test_merge_into_a_vault_skill_makes_a_user_skill_and_removes_the_vault_one(self):
        OLD = self.vfiles("old-way", "Stitch a granny square blanket with wool.", body="Old.")
        self.apply(OLD)
        self.apply({**OLD, **self.vfiles("debug-helper", "Stitch a granny square blanket with wool.", body="Newer.")},
                   release="vault-2026.02.01")
        (dec,) = rbxskills.pending()
        rbxskills.decide(dec["id"], "merge", "---\nname: old-way\ndescription: Merged.\n---\n\nBoth.\n")
        (e,) = rbxskills.library()
        self.assertEqual((e["id"], e["origin"]), ("old-way", "user"))
        self.assertFalse(os.path.exists(self.local("Skills/old-way")))
        self.assertTrue(os.path.isdir(os.path.join(rbxvaultsync.state_dir(), "removed", "vault-2026.02.01",
                                                   "Skills", "old-way")))
        base = rbxvaultsync.read_base()
        self.assertIn("debug-helper", base["declined"])
        self.assertIn("old-way", base["declined"])
        self.assertEqual([p for p in base["files"] if "old-way" in p], [])
        self.apply(self.vfiles("old-way", "Stitch a granny square blanket with wool.", body="Old v2."),
                   release="vault-2026.03.01")  # an update never touches the merged user skill
        self.assertFalse(os.path.exists(self.local("Skills/old-way")))
        self.assertIn("Both.", self.get_text(e["dir"]))

    def get_text(self, d):
        with open(os.path.join(d, "SKILL.md"), encoding="utf-8") as f:
            return f.read()

    def test_replace_a_hand_made_vault_folder_is_not_declined(self):
        self.put("Skills/mine/SKILL.md", "---\nname: mine\ndescription: Stitch a granny square blanket with wool.\n---\n\nB\n")
        self.apply(self.vfiles("debug-helper", "Stitch a granny square blanket with wool.", body="Newer."))
        (dec,) = rbxskills.pending()
        rbxskills.decide(dec["id"], "replace")
        self.assertEqual(rbxvaultsync.read_base()["declined"], [])
        self.assertTrue(os.path.isdir(os.path.join(rbxvaultsync.state_dir(), "removed", "vault-2026.01.01", "Skills", "mine")))

    def test_keep_both_avoids_a_name_a_library_skill_has(self):
        self.user_skill("debug-helper-community", "Something about crochet wool.")
        dec = self.waiting_one()
        rbxskills.decide(dec["id"], "keep_both")
        self.assertEqual(rbxvaultsync.read_base()["renamed"], {"debug-helper": "debug-helper-community-2"})

    def test_symlinked_vault_folder_on_decide_gives_a_plain_message(self):
        dec = self.waiting_one()
        outside = tempfile.mkdtemp(prefix="outside ")
        self.addCleanup(shutil.rmtree, outside, True)
        os.symlink(outside, os.path.join(self.know, "Skills"))
        with self.assertRaises(ValueError) as cm:
            rbxskills.decide(dec["id"], "replace")
        self.assertEqual(str(cm.exception), rbxvaultsync.BAD)

    def test_a_vault_decision_waits_only_while_its_folder_is_in_incoming(self):
        dec = self.waiting_one()
        outside = os.path.join(self.root, "elsewhere")
        os.makedirs(outside)
        decs = rbxskills._decisions()
        decs[0]["b_dir"] = outside
        rbxskills._save_decisions(decs)
        self.assertEqual(rbxskills.pending(), [])
        self.assertEqual(rbxskills._decisions()[0]["state"], "gone")

    def test_one_state_dir(self):
        self.assertEqual(rbxvaultsync.state_dir(), rbxskills.vault_state_dir())


if __name__ == "__main__":
    unittest.main()
