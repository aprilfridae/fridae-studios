"""The knowledge exam's own pieces (tests/scale_bench/exam_lib.py, corpus.py's exam additions, exam.py's planning):
the judge's verdicts read strictly, the stage cache resumes, the failure classes, the not-found threshold and the
stale pairs in the corpus."""
import json
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "scale_bench"))

import corpus  # noqa: E402
import exam_lib as L  # noqa: E402


def item(key="a|P01", facts=("f1", "f2"), wrong=("w1",), expect="answer"):
    return {"key": key, "question": "q", "expect": expect, "facts": list(facts), "wrong": list(wrong), "guide": "",
            "answer": "x"}


class Verdicts(unittest.TestCase):
    def test_a_fenced_reply_is_read(self):
        text = 'Here:\n```json\n{"verdicts": [{"key": "a|P01", "facts": ["present", "Absent"], "wrong": [false], ' \
               '"abstained": false, "guide_pointer": false, "note": ""}]}\n```'
        v = L.parse_verdicts(text, [item()])
        self.assertEqual(v["a|P01"]["facts"], ["present", "absent"])
        self.assertEqual(v["a|P01"]["wrong"], [False])

    def test_a_verdict_with_the_wrong_number_of_facts_is_dropped(self):
        text = json.dumps({"verdicts": [{"key": "a|P01", "facts": ["present"], "wrong": [False]},
                                        {"key": "a|P02", "facts": ["present", "present"], "wrong": [True]}]})
        v = L.parse_verdicts(text, [item(), item("a|P02")])
        self.assertNotIn("a|P01", v)
        self.assertIn("a|P02", v)

    def test_unknown_states_keys_and_non_booleans_are_dropped(self):
        text = json.dumps({"verdicts": [{"key": "a|P01", "facts": ["present", "maybe"], "wrong": [False]},
                                        {"key": "zz", "facts": ["present", "present"], "wrong": [False]},
                                        {"key": "a|P02", "facts": ["present", "present"], "wrong": ["no"]}]})
        self.assertEqual(L.parse_verdicts(text, [item(), item("a|P02")]), {})

    def test_no_json_means_no_verdicts(self):
        self.assertEqual(L.parse_verdicts("I cannot grade these.", [item()]), {})
        self.assertEqual(L.parse_verdicts(None, [item()]), {})

    def test_scores(self):
        e = {"expect": "answer", "facts": ["a", "b"], "wrong": ["w"]}
        ok = {"facts": ["present", "present"], "wrong": [False], "abstained": False, "guide_pointer": False}
        half = {"facts": ["present", "absent"], "wrong": [False], "abstained": False, "guide_pointer": False}
        bad = {"facts": ["present", "contradicted"], "wrong": [False], "abstained": False, "guide_pointer": False}
        self.assertTrue(L.score(e, ok)["correct"])
        self.assertEqual(L.score(e, half)["recall"], 0.5)
        self.assertFalse(L.score(e, half)["correct"])
        self.assertTrue(L.score(e, bad)["wrong"])
        neg = {"expect": "not_found", "facts": [], "wrong": []}
        self.assertTrue(L.score(neg, {"facts": [], "wrong": [], "abstained": True, "guide_pointer": False})["correct"])
        s = L.score(neg, {"facts": [], "wrong": [], "abstained": False, "guide_pointer": False})
        self.assertFalse(s["correct"])
        self.assertTrue(s["wrong"])
        nf = L.score(e, {"facts": ["absent", "absent"], "wrong": [False], "abstained": True, "guide_pointer": False})
        self.assertTrue(nf["false_not_found"])

    def test_the_judge_prompt_carries_every_item(self):
        t = L.judge_items_text([item("k1"), item("k2")])
        self.assertEqual([json.loads(x)["key"] for x in t.splitlines()], ["k1", "k2"])


class Cache(unittest.TestCase):
    def test_units_and_records_resume(self):
        with tempfile.TemporaryDirectory() as d:
            c = L.StageCache(d)
            self.assertIsNone(c.get("retrieval", "k1", "light-1x"))
            c.put("retrieval", "k1", "light-1x", {"x": 1})
            self.assertEqual(c.get("retrieval", "k1", "light-1x"), {"x": 1})
            self.assertIsNone(c.get("retrieval", "k2", "light-1x"))  # another code version starts afresh
            c.append("answers", "k1", "lib", {"id": "P01", "a": 1})
            c.append("answers", "k1", "lib", {"id": "P02", "a": 2})
            with open(c.path("answers", "k1", "lib", ".jsonl"), "a") as f:
                f.write('{"id": "P03", "a"')  # a run killed mid-line
            self.assertEqual(sorted(c.records("answers", "k1", "lib")), ["P01", "P02"])

    def test_every_unit_carries_the_commit_it_measured(self):
        with tempfile.TemporaryDirectory() as d:
            c = L.StageCache(d, {"commit": "abc123", "branch": "b"})
            c.put("retrieval", "k", "u", {"x": 1})
            c.append("answers", "k", "lib", {"id": "P01"})
            self.assertEqual(c.get("retrieval", "k", "u")["_measured"]["commit"], "abc123")
            self.assertEqual(c.records("answers", "k", "lib")["P01"]["_measured"], "abc123")

    def test_file_hash_follows_content(self):
        with tempfile.TemporaryDirectory() as d:
            p = os.path.join(d, "a.py")
            with open(p, "w") as f:
                f.write("1")
            h1 = L.file_hash([p])
            with open(p, "w") as f:
                f.write("2")
            self.assertNotEqual(h1, L.file_hash([p]))
            self.assertNotEqual(L.file_hash([p], "x"), L.file_hash([p], "y"))


class Numbers(unittest.TestCase):
    def test_hit_rates(self):
        h = L.hit_rates([1, 3, None, 6])
        self.assertEqual(h["hit@1"], 0.25)
        self.assertEqual(h["hit@3"], 0.5)
        self.assertEqual(h["hit@5"], 0.5)
        self.assertAlmostEqual(h["mrr"], (1 + 1 / 3 + 1 / 6) / 4)

    def test_first_rank_matches_titles_and_guide_paths(self):
        self.assertEqual(L.first_rank(["X", "roblox-testing/cloud-test-runner"], ["cloud-test-runner"]), 2)
        self.assertIsNone(L.first_rank(["X", "Y", "Z"], ["Z"], k=2))

    def test_threshold_separates(self):
        b = L.best_threshold([0.8, 0.9, 0.7], [0.2, 0.3, 0.75])
        self.assertEqual(b["threshold"], 0.7)  # 0.7 and 0.8 tie: the lower one
        self.assertAlmostEqual(b["balanced"], (2 / 3 + 1) / 2)
        self.assertEqual(L.best_threshold([0.5, 0.6], [0.1])["balanced"], 1.0)
        self.assertIsNone(L.best_threshold([0.5], []))


class Failures(unittest.TestCase):
    entry = {"expect": "answer", "facts": ["Autosave every 90 seconds", "Lease of 5 minutes"], "wrong": ["every 30 s"]}
    v_missing = {"facts": ["present", "absent"], "wrong": [False], "abstained": False, "guide_pointer": False}

    def rec(self, titles, **kw):
        r = {"answer": "Autosave every 90 seconds.", "passages": [[{"title": t} for t in titles]]}
        r.update(kw)
        return r

    def test_correct_answers_have_no_class(self):
        v = dict(self.v_missing, facts=["present", "present"])
        self.assertIsNone(L.classify(self.entry, v, self.rec(["Autosave Rules"]), ["Autosave Rules"]))
        self.assertIsNone(L.classify(self.entry, None, self.rec([]), ["Autosave Rules"]))

    def test_retrieval_miss(self):
        self.assertEqual(L.classify(self.entry, self.v_missing, self.rec(["Other"]), ["Autosave Rules"]),
                         "retrieval miss")

    def test_writer_omission(self):
        r = self.rec(["Autosave Rules"], writer_raw="...", format_ok=True,
                     claims_raw=[{"claim": "Autosave every 90 seconds", "quote": "q"}],
                     claims_quoted=[{"claim": "Autosave every 90 seconds", "quote": "q"}])
        self.assertEqual(L.classify(self.entry, self.v_missing, r, ["Autosave Rules"]), "writer omission")

    def test_a_check_dropped_the_claim(self):
        r = self.rec(["Autosave Rules"], writer_raw="...", format_ok=True,
                     claims_raw=[{"claim": "Autosave every 90 seconds", "quote": "q"},
                                 {"claim": "The session lock lease is 5 minutes", "quote": "lease of 5 minutes"}],
                     claims_quoted=[{"claim": "Autosave every 90 seconds", "quote": "q"}])
        self.assertEqual(L.classify(self.entry, self.v_missing, r, ["Autosave Rules"]), "check dropped a true claim")

    def test_format_failure_and_guides_and_negatives_and_stale(self):
        r = self.rec(["Autosave Rules"], writer_raw="not json at all", format_ok=False)
        self.assertEqual(L.classify(self.entry, self.v_missing, r, ["Autosave Rules"]), "format failure")
        g = {"expect": "guide", "facts": [], "wrong": [], "guide": "roblox-testing"}
        self.assertEqual(L.classify(g, {"facts": [], "wrong": [], "abstained": False, "guide_pointer": False},
                                    self.rec([]), ["roblox-testing"]), "guide handling")
        n = {"expect": "not_found", "facts": [], "wrong": []}
        self.assertEqual(L.classify(n, {"facts": [], "wrong": [], "abstained": False, "guide_pointer": False},
                                    self.rec(["X"]), []), "distractor or stale won")
        v = dict(self.v_missing, wrong=[True])
        self.assertEqual(L.classify(self.entry, v, self.rec(["Other"]), ["Autosave Rules"], stale_old="Old"),
                         "distractor or stale won")


class StalePairs(unittest.TestCase):
    spec = {"id": "S01", "old": "Light Sources and Shadows",
            "new": {"folder": "07 Lighting", "title": "Light Range Cap Raised", "date": "2026-11-12",
                    "body": "Range is now capped at 240 studs."}}

    def test_the_marked_half_says_so_in_front_matter_and_the_other_in_text(self):
        marked = corpus.stale_note(self.spec, True)
        plain = corpus.stale_note(self.spec, False)
        self.assertIn('supersedes: "Light Sources and Shadows"', marked)
        self.assertNotIn("replaces the older note", marked)
        self.assertNotIn("supersedes", plain)
        self.assertIn("replaces the older note [[Light Sources and Shadows]]", plain)
        for t in (marked, plain):
            self.assertIn("date: 2026-11-12", t)
            self.assertIn("# Light Range Cap Raised", t)
        self.assertEqual([corpus.stale_marked(i) for i in range(4)], [True, False, True, False])

    def test_extras_are_written_and_the_old_note_points_onward(self):
        with tempfile.TemporaryDirectory() as d:
            os.makedirs(os.path.join(d, "vault", "07 Lighting"))
            old = os.path.join(d, "vault", "07 Lighting", "Light Sources and Shadows.md")
            with open(old, "w") as f:
                f.write("---\ntags: [light]\n---\n# Light Sources and Shadows\n\nRange is capped at 120 studs.\n")
            spec2 = json.loads(json.dumps(self.spec))
            spec2["id"], spec2["old"] = "S02", "Missing Note"
            spec2["new"]["title"] = "Second"
            exam = {"stale": [self.spec, spec2],
                    "game": {"PROJECT.md": "# G\n", "notes": {"Autosave Rules.md": "# Autosave Rules\n\nx\n"}}}
            corpus.write_exam_extras(d, exam)
            with open(old) as f:
                self.assertTrue(f.read().startswith('---\nsuperseded_by: "Light Range Cap Raised"\ntags'))
            self.assertTrue(os.path.isfile(os.path.join(d, "vault", "07 Lighting", "Light Range Cap Raised.md")))
            with open(os.path.join(d, "vault", "07 Lighting", "Second.md")) as f:
                self.assertIn("[[Missing Note]]", f.read())  # the unmarked half: text only
            self.assertTrue(os.path.isfile(os.path.join(d, "game", "PROJECT.md")))
            self.assertTrue(os.path.isfile(os.path.join(d, "game", ".fridae", "notes", "Autosave Rules.md")))


class ContentRight(unittest.TestCase):
    """Scoring with copies: a distractor made from the gold note counts as content-right, an echo never does."""

    class Ex:
        def derived(self, scale):
            return {"Gold - short version": {"kind": "summary", "from": ["Gold"]},
                    "Working notes - Gold (3)": {"kind": "echo", "from": ["Other"], "looks_like": "Gold"},
                    "Mixed bag": {"kind": "mix", "from": ["Other", "Gold"]}}

    def test_expander_and_drop(self):
        import exam_report as R
        exp = R.expander(self.Ex(), 10)
        self.assertEqual(sorted(exp(["Gold"])), ["Gold", "Gold - short version", "Mixed bag"])
        qs = {"P1": {"gold": ["Gold"]}, "P2": {"gold": ["Gold"]}, "P3": {"gold": ["Gold"]}}
        small = [{"id": i, "set": "paraphrase", "titles": ["Gold"]} for i in qs]
        large = [{"id": "P1", "set": "paraphrase", "titles": ["Gold"]},
                 {"id": "P2", "set": "paraphrase", "titles": ["Gold - short version"]},
                 {"id": "P3", "set": "paraphrase", "titles": ["Working notes - Gold (3)"]}]
        self.assertEqual(R.drop(self.Ex(), small, (10, large), qs), {"kept": 1, "copy": 1, "real": 1})


class TheQuestions(unittest.TestCase):
    def test_every_answer_entry_has_a_question_and_a_valid_key(self):
        import exam
        base, ex = exam.load_sets()
        qs = exam.all_questions(base, ex)
        entries = exam.answer_entries(ex, qs)
        self.assertGreaterEqual(len(entries), 55)
        for e in entries:
            self.assertIn(e["expect"], ("answer", "not_found", "guide"))
            if e["expect"] == "answer":
                self.assertTrue(2 <= len(e["facts"]) <= 4, e["id"])
                self.assertTrue(e["gold"], e["id"])
            if e["expect"] == "guide":
                self.assertTrue(e.get("guide"), e["id"])
        halves = [q["marked"] for q in qs.values() if q["set"] == "stale"]
        self.assertEqual(sum(halves), len(halves) - sum(halves))
        quick = exam.answer_entries(ex, qs, 6)
        self.assertEqual(len(quick), 6)
        self.assertEqual({e["expect"] for e in quick}, {"answer", "not_found", "guide"})
        self.assertTrue(any(e.get("stale_old") for e in quick))

    def test_vault_conflicts_are_listed_and_the_judge_is_told_the_rules(self):
        import exam
        import exam_report as R
        _, ex = exam.load_sets()
        self.assertIn("P11", [qid for qid, _ in R.vault_conflicts(ex)])
        self.assertIn("either one counts", L.JUDGE_PROMPT)
        self.assertIn("mark that wrong fact true as well", L.JUDGE_PROMPT)

    def test_the_reader_prompt_numbers_every_passage(self):
        import exam
        p = exam.reader_prompt("q?", [{"hits": [{"shelf": "vault", "title": "A", "heading": "H", "text": "x"}]},
                                      {"hits": [{"shelf": "guides", "title": "g", "heading": "", "text": "y"}]}])
        self.assertIn("[1] (vault: A › H) x", p)
        self.assertIn("[2] (guides: g) y", p)


if __name__ == "__main__":
    unittest.main()
