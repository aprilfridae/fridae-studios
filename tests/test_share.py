"""rbxshare: which of the user's skills may be shared, and packing a batch. Built on a made-up home folder."""
import os
import shutil
import tempfile
import unittest
from unittest import mock

import rbxpaths
import rbxshare
import rbxskills

FRESH = """# Chasing a fault

When something misbehaves, resist the urge to poke at it. First make it happen again on demand; a failure you
cannot trigger cannot be fixed with any confidence.

Then shrink the situation. Strip away every part that is not needed until the smallest case that still goes wrong
remains. What is left usually points straight at the cause.

Write down what you expected next to what you observed. The gap between the two is the real question. Guess one
cause at a time, test that guess with a single change, and undo the change if it did not matter.

Once the cause is found, add a check that fails because of it, make the fix, and watch the check turn green. Keep
the check so the same fault cannot return unnoticed.
"""


NEW_PARAGRAPH = [
    "Start by listing what the person asked for in their own words.", "Keep that list short and visible while you work.",
    "When a step feels unclear, ask one plain question before going on.", "Prefer a small change that can be checked today.",
    "Name each file you touched and say why you touched it.", "If two choices look equal, pick the one that is easier to undo.",
    "Leave notes for whoever comes next, even if that is you.", "Avoid clever tricks that need an explanation.",
    "Read the error message slowly and completely.", "Share the result as soon as something useful exists.",
    "Thank the person for waiting, then show the outcome.", "Offer one next step, not five.",
    "Close the loop by asking whether it matches what they wanted.", "Tidy any scratch files you made along the way.",
    "Mention anything you chose not to do, and why.", "End with a sentence about what to try next."]

THEIRS = "Find the root cause before proposing fixes. Reproduce the failure, read the error, check recent changes, form one hypothesis and test it with the smallest change."


def skill(folder, name, body="Do the thing.", extra=None, front=""):
    os.makedirs(folder, exist_ok=True)
    with open(os.path.join(folder, "SKILL.md"), "w", encoding="utf-8", newline="") as f:
        f.write("---\nname: %s\ndescription: About %s.\n%s---\n\n%s\n" % (name, name, front, body))
    for rel, data in (extra or {}).items():
        path = os.path.join(folder, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data if isinstance(data, bytes) else data.encode("utf-8"))


class ShareTests(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="share é ")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.home = os.path.join(self.root, "home")
        self.user = os.path.join(self.home, "skills")
        self.claude = os.path.join(self.root, "claude")
        os.makedirs(self.claude)
        for p in (mock.patch.object(rbxpaths, "HOME_DIR", self.home),
                  mock.patch.object(rbxpaths, "KNOWLEDGE_DIR", os.path.join(self.root, "Knowledge")),
                  mock.patch.object(rbxskills, "HOME", self.root),
                  mock.patch.dict(os.environ, {"FRIDAE_USER_CLAUDE": self.claude, "CLAUDE_CONFIG_DIR": ""})):
            p.start()
            self.addCleanup(p.stop)

    def mine(self, name, **kw):
        d = os.path.join(self.user, name)
        skill(d, name, **kw)
        return d

    def ready(self):
        return {e["id"]: e for e in rbxshare.shareable()["ready"]}

    def held(self):
        return {e["id"]: e["why"] for e in rbxshare.shareable()["held"]}

    def test_own_skill_is_ready(self):
        self.mine("tidy", extra={"scripts/go.sh": "echo hi\n"})
        e = self.ready()["tidy"]
        self.assertIs(e["scripts"], True)
        self.assertEqual(e["files"], 2)
        self.assertIs(e["changed"], False)
        self.assertEqual(e["hash"], rbxskills.folder_hash(e["dir"]))
        self.mine("plain")
        self.assertIs(self.ready()["plain"]["scripts"], False)

    def test_import_is_held_with_credit_reason(self):
        d = self.mine("borrowed")
        rbxskills._write_json(os.path.join(d, rbxskills.SOURCE_FILE), {"source": "/x/superpowers"})
        self.assertEqual(self.held()["borrowed"], "Imported from /x/superpowers: it is credited there, not shared.")
        self.assertNotIn("borrowed", self.ready())

    def test_app_only_names_are_not_listed(self):
        self.mine("docs")
        out = rbxshare.shareable()
        self.assertNotIn("docs", [e["id"] for e in out["ready"] + out["held"]])

    def test_kit_name_is_held(self):
        self.mine("critic", body="Totally different words about gardening and soil.")
        self.assertEqual(self.held()["critic"], "Has the same name as one of Fridae's skills.")

    def kit_critic(self):
        return rbxskills._read(os.path.join(rbxskills.KIT, "vault", "Fridae", "Guides", "Core", "critic", "SKILL.md"))

    def test_edited_copy_is_held_and_fresh_topic_is_ready(self):
        body = rbxskills.front(self.kit_critic())[1].strip("\n")
        lines = body.split("\n")
        n = max(1, len(lines) // 10)
        at = len(lines) // 3
        lines[at:at + n] = (NEW_PARAGRAPH * 2)[:n]  # one paragraph of the copy rewritten, about 10% of its lines
        copy = self.mine("my-critic", body="\n".join(lines))
        fresh = self.mine("fresh-debug", body=FRESH)
        a, b = rbxshare.best_copy(copy), rbxshare.best_copy(fresh)
        msg = "edited copy scored %.4f, fresh debugging skill scored %.4f, COPY_MIN %.2f" % (a[0], b[0], rbxshare.COPY_MIN)
        print(msg)
        self.assertGreaterEqual(a[0], rbxshare.COPY_MIN, msg)
        self.assertLess(b[0], rbxshare.COPY_MIN, msg)
        self.assertEqual(self.held()["my-critic"], "Looks like a copy of critic (built into Fridae).")

    def test_edited_copy_of_an_import_is_held_though_its_source_is_gone(self):
        d = self.mine("borrowed-debug", body=FRESH)
        rbxskills._write_json(os.path.join(d, rbxskills.SOURCE_FILE), {"source": "/gone/superpowers"})
        lines = FRESH.strip("\n").split("\n")
        lines[5:6] = NEW_PARAGRAPH[:1]  # a little of it rewritten
        self.mine("my-debug", body="\n".join(lines))
        a = rbxshare.best_copy(os.path.join(self.user, "my-debug"))
        self.assertGreaterEqual(a[0], rbxshare.COPY_MIN, "scored %.4f" % a[0])
        self.assertEqual(self.held()["my-debug"], "Looks like a copy of borrowed-debug (your library).")
        self.assertEqual(self.held()["borrowed-debug"],
                         "Imported from /gone/superpowers: it is credited there, not shared.")

    def test_same_topic_written_fresh_is_ready(self):
        skill(os.path.join(self.claude, "skills", "systematic-debugging"), "systematic-debugging", THEIRS)
        self.mine("fresh-debug", body=FRESH)
        self.assertIn("systematic-debugging", [c["id"] for c in rbxskills._copies()])
        self.assertIn("fresh-debug", self.ready())

    def test_binary_and_size_rules(self):
        self.mine("withlogo", extra={"logo.png": b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"})
        self.mine("bigfile", extra={"notes.md": "x" * (300 * 1024)})
        self.mine("many", extra={"f%d.md" % i: "t" for i in range(40)})
        h = self.held()
        self.assertEqual(h["withlogo"], "Has a file that is not text: logo.png.")
        self.assertEqual(h["bigfile"], "notes.md is over 200 KB.")
        self.assertEqual(h["many"], "Too big to share (over 1 MB or 40 files).")

    def test_other_licence_is_held(self):
        self.mine("apache", front="licence: Apache-2.0\n")
        self.mine("mine", front="licence: %s\n" % rbxshare.LICENCE)
        self.assertEqual(self.held()["apache"], "Says its licence is Apache-2.0; only skills you made, under CC BY 4.0 "
                                                "and MIT, can be shared.")
        self.assertIn("mine", self.ready())

    def test_sent_skill_is_ready_again_only_when_changed(self):
        d = self.mine("tidy")
        h = rbxskills.folder_hash(d)
        rbxshare.mark_shared("tidy", h, "https://example.test/pr/1")
        out = rbxshare.shareable()
        self.assertNotIn("tidy", [e["id"] for e in out["ready"] + out["held"]])
        self.assertEqual([e["id"] for e in out["sent"]], ["tidy"])
        self.assertEqual(out["sent"][0]["pr"], "https://example.test/pr/1")
        with open(os.path.join(d, "SKILL.md"), "a", encoding="utf-8", newline="") as f:
            f.write("\nOne more line.\n")
        e = self.ready()["tidy"]
        self.assertIs(e["changed"], True)
        self.assertEqual(rbxshare.shareable()["sent"], [])

    def test_pack_sets_front_matter_and_strips_pins(self):
        d = self.mine("pinned", front="core: true\nowner: user\nsupersedes: [x]\nexamples:\n  - one\n",
                      extra={"scripts/go.sh": "echo\n", ".venv/lib/x.py": "no", "__pycache__/a.pyc": "no",
                             ".hidden": "no"})
        out = os.path.join(self.root, "out")
        hashes = rbxshare.pack(["pinned"], out, "ada")
        self.assertEqual(hashes, {"pinned": rbxskills.folder_hash(d)})
        base = os.path.join(out, "Skills", "pinned")
        text = rbxskills._read(os.path.join(base, "SKILL.md"))
        fm, body = rbxskills.front(text)
        self.assertEqual(fm["licence"], "CC-BY-4.0 (text), MIT (scripts)")
        self.assertEqual(fm["source"], "shared from Fridae by ada")
        self.assertEqual(fm["credit"], "ada")
        self.assertRegex(fm["updated"], r"^\d{4}-\d\d-\d\d$")
        for k in ("core", "owner", "supersedes"):
            self.assertNotIn(k, fm)
        self.assertEqual(fm["examples"], ["one"])
        self.assertIn("Do the thing.", body)
        with open(os.path.join(base, "SKILL.md"), "rb") as f:
            self.assertNotIn(b"\r", f.read())
        listed = sorted(os.path.relpath(os.path.join(r, f), base).replace(os.sep, "/")
                        for r, ds, fs in os.walk(base) for f in fs)
        self.assertEqual(listed, ["SKILL.md", "scripts/go.sh"])
        self.assertFalse([n for n in os.listdir(os.path.join(out, "Skills")) if n.startswith(".")])
        # the user's own folder is unchanged
        self.assertEqual(rbxskills.folder_hash(d), hashes["pinned"])

    def test_pack_refuses_what_is_not_ready(self):
        self.mine("critic", body="Totally different words.")
        with self.assertRaises(ValueError):
            rbxshare.pack(["critic"], os.path.join(self.root, "out"), "ada")
        with self.assertRaises(ValueError):
            rbxshare.pack(["nothing"], os.path.join(self.root, "out"), "ada")
        self.assertFalse(os.path.exists(os.path.join(self.root, "out", "Skills", "critic")))

    def test_mark_shared_is_a_record_beside_the_skill(self):
        d = self.mine("tidy")
        rbxshare.mark_shared("tidy", "abc", "https://example.test/pr/2")
        rec = rbxskills._read_json(os.path.join(d, rbxskills.SHARED_FILE), {})
        self.assertEqual([(e["name"], e["hash"], e["pr"]) for e in rec["sent"]],
                         [("tidy", "abc", "https://example.test/pr/2")])
        self.assertTrue(rec["sent"][0]["at"])
        with self.assertRaises(LookupError):
            rbxshare.mark_shared("nothing", "abc", "u")

    def test_compare_same_name_and_closest(self):
        import argparse
        import contextlib
        import io
        import json
        vault = os.path.join(self.root, "vault copy")
        skill(os.path.join(vault, "tidy"), "tidy", "Put things back where they live.\nWipe the bench.")
        skill(os.path.join(vault, "debug-vault"), "debug-vault", FRESH)
        new = os.path.join(self.root, "incoming")
        skill(os.path.join(new, "tidy"), "tidy", "Put things back where they live.\nWipe the bench twice.")
        out = rbxshare.compare(os.path.join(new, "tidy"), vault)
        self.assertTrue(out["same_name"])
        self.assertIn("-Wipe the bench.", out["diff"])
        self.assertIn("+Wipe the bench twice.", out["diff"])
        self.assertEqual(out["closest"]["id"], "tidy")
        # a near-duplicate under another name: no same name, no diff, and its closest is the vault's skill
        lines = FRESH.strip("\n").split("\n")
        lines[5:6] = NEW_PARAGRAPH[:1]
        skill(os.path.join(new, "chasing-faults"), "chasing-faults", "\n".join(lines))
        out = rbxshare.compare(os.path.join(new, "chasing-faults"), vault)
        self.assertFalse(out["same_name"])
        self.assertIsNone(out["diff"])
        self.assertEqual(out["closest"]["id"], "debug-vault")
        self.assertGreaterEqual(out["closest"]["score"], rbxskills.OVERLAP)
        self.assertEqual(os.path.realpath(out["closest"]["dir"]), os.path.realpath(os.path.join(vault, "debug-vault")))
        # something else entirely: nothing near
        skill(os.path.join(new, "gardening"), "gardening", "Water the soil early. Prune roses in late winter.")
        far = rbxshare.compare(os.path.join(new, "gardening"), vault)
        self.assertLess((far["closest"] or {"score": 0})["score"], rbxskills.OVERLAP)
        # the program
        ap = argparse.ArgumentParser()
        rbxskills.register(ap.add_subparsers(dest="cmd", required=True))
        buf = io.StringIO()
        a = ap.parse_args(["skills", "compare", "--new", os.path.join(new, "tidy"), "--against", vault, "--json"])
        with contextlib.redirect_stdout(buf):
            a.fn(a)
        self.assertTrue(json.loads(buf.getvalue())["same_name"])

    def test_cli_prints_json(self):
        import argparse
        import contextlib
        import io
        import json
        ap = argparse.ArgumentParser()
        rbxskills.register(ap.add_subparsers(dest="cmd", required=True))

        def run(*argv):
            buf = io.StringIO()
            a = ap.parse_args(["skills"] + list(argv))
            with contextlib.redirect_stdout(buf):
                a.fn(a)
            return json.loads(buf.getvalue())

        d = self.mine("tidy")
        self.mine("neat")
        self.assertEqual(sorted(e["id"] for e in run("shareable", "--json")["ready"]), ["neat", "tidy"])
        out = os.path.join(self.root, "cli out")
        hashes = run("pack", "--out", out, "--login", "ada", "tidy", "neat")
        self.assertEqual(hashes["tidy"], rbxskills.folder_hash(d))
        self.assertTrue(os.path.isfile(os.path.join(out, "Skills", "neat", "SKILL.md")))
        run("mark-shared", "tidy", "--hash", hashes["tidy"], "--pr", "https://example.test/pr/3")
        self.assertEqual([e["id"] for e in run("shareable", "--json")["sent"]], ["tidy"])


if __name__ == "__main__":
    unittest.main()
