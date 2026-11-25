"""The skills quick check: how well the per-message suggestions pick skills, over the real library.

Run (about 3 minutes; the first Rust test build can add a few more):
    .venv/bin/python tests/skills_bench.py --models <the Light model folder> [--work <scratch folder>]
A measuring script, not a unit test. It never writes to the real home folders: the library is made in --work by
rbxskills.import_external() with user_dir pointed there (the import only READS the Mac's skill folders: ~/.claude,
its plugins, ~/.agents ...). The Light model is read from --models only (no download).

What it measures, both paths through the code that ships:
- dense (the index service's path): rbxrag Engine.suggest exactly as `POST /suggest` ranks (BAAI/bge-small-en-v1.5,
  dense cosine of the message against each skill's summary), then Rust's suggest rules (the gate, SUGGEST_MIN,
  SUGGEST_REL, at most 3, pinned skills dropped);
- keyword (the fallback when the service is off): Rust's own scorer and rules.
The Rust half is skills::tests::skills_bench (cargo test ... -- --ignored), fed the index and the service's hits; the
threshold tables re-apply Rust's `within` rule to the scores Rust gave, and the shipped thresholds' rows are checked
against the lines Rust itself made.

Cases: tests/skills_bench_cases.json, 40 messages (34 asking for skill-shaped work, 6 short follow-ups with want []),
each {q, kind, want}: want is the skills a good engineer would load. A suggestion outside want is off-target.

Also: the token count of the instructions' skills part before (fb64721's catalog of every skill) and after (the
pinned core), and the OVERLAP check (the similar() scores the import computed: its decisions against the closest
pairs it let in).
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)
os.environ.setdefault("HF_HUB_OFFLINE", "1")

CASES = os.path.join(HERE, "skills_bench_cases.json")
OLD_CATALOG = "fb64721"  # the commit whose instructions listed every skill


def within(hits, lo, rel, most=3, words=0):
    """Rust's skills::within: the hits at lo or more and within rel of the best, at most `most` (best first). hits:
    [id, score] or (keyword) [id, score, words held]; words > 0 keeps only keyword hits holding that many words
    first (Rust's KEYWORD_WORDS, applied before the best is taken)."""
    if words:
        hits = [h for h in hits if h[2] >= words]
    if not hits:
        return []
    top = hits[0][1]
    return [h[0] for h in hits if h[1] >= lo and h[1] >= top - rel - 1e-9][:most]


def line_ids(line):
    if not line:
        return []
    out, depth, cur = [], 0, ""
    for ch in line.split(": ", 1)[1] + ",":  # "a (why), b (why, more)": split at the commas outside brackets
        depth += (ch == "(") - (ch == ")")
        if ch == "," and depth == 0:
            out.append(cur.strip().split(" (", 1)[0])
            cur = ""
        else:
            cur += ch
    return out


def stats(cases, picks):
    """picks: one list of ids per case. (hits, skill turns, shown/turn, off-target/turn, follow-ups with any)."""
    skill = [(c, p) for c, p in zip(cases, picks) if c["want"]]
    follow = [(c, p) for c, p in zip(cases, picks) if not c["want"]]
    hit = sum(1 for c, p in skill if set(p) & set(c["want"]))
    shown = sum(len(p) for _, p in skill) / len(skill)
    off = sum(sum(1 for i in p if i not in c["want"]) for c, p in skill) / len(skill)
    return hit, len(skill), shown, off, sum(1 for _, p in follow if p), len(follow)


def row(label, st):
    hit, n, shown, off, fu, nf = st
    return "%-30s | hit %2d/%d | shown/turn %.2f | off-target/turn %.2f | follow-ups %d/%d" % (label, hit, n, shown, off, fu, nf)


def make_library(work):
    """The real library, imported into work/lib; [(newcomer, its closest library skill, similar())] the import saw."""
    import rbxskills
    lib = os.path.join(work, "lib")
    os.makedirs(lib, exist_ok=True)
    rbxskills.user_dir = lambda: lib
    seen = []
    plain = rbxskills.similar

    def logged(a, b, idf=None):
        s = plain(a, b, idf)
        seen.append((b.get("id") or b.get("name"), a.get("id") or a.get("name"), s))
        return s

    rbxskills.similar = logged
    try:
        res = rbxskills.import_external()
    finally:
        rbxskills.similar = plain
    best = {}
    for b, a, s in seen:
        if s > best.get(b, ("", -1.0))[1]:
            best[b] = (a, s)
    return res, best


def dense_hits(cases, models):
    """[(model, [[id, cosine]], seconds)] per case from Engine.suggest over the library, as /suggest answers."""
    from rbxrag import chunks
    from rbxrag.engine import Engine, PLANS
    from rbxrag.models import LightEmbedders
    from rbxrag.stores import MemStore
    tmp = tempfile.mkdtemp(prefix="skbench-rag-")
    emb = LightEmbedders(models)
    files = [("skills", t, k) for t, k, _ in chunks._skills(None)]
    eng = Engine(MemStore(tmp), emb, os.path.join(tmp, "manifest.json"), corpus=lambda p: files,
                 project_files=lambda p: set(), plan=PLANS["light"])
    t0 = time.time()
    eng.sync()
    print("index: %d skills embedded in %.1f s" % (len(files), time.time() - t0))
    eng.suggest("warm up the model", "", 10)
    out = []
    for c in cases:
        t0 = time.time()
        hits = eng.suggest(c["q"], c["kind"], 10)
        out.append((emb.model, [[h["id"], h["score"]] for h in hits], time.time() - t0))
    return out, emb


def run_rust(work, index, cases, dense):
    inp, outp = os.path.join(work, "rust_in.json"), os.path.join(work, "rust_out.json")
    json.dump({"index": index, "ai": "cursor", "cases": [
        {"q": c["q"], "kind": c["kind"], "dense": {"model": m, "hits": h}} for c, (m, h, _) in zip(cases, dense)]},
        open(inp, "w"))
    env = dict(os.environ, FRIDAE_SKILLS_BENCH=inp, FRIDAE_SKILLS_BENCH_OUT=outp)
    cmd = ["cargo", "test", "--manifest-path", os.path.join(ROOT, "app-rs", "Cargo.toml"), "--bin", "fridae",
           "skills::tests::skills_bench", "--", "--ignored", "--exact"]
    r = subprocess.run(cmd, env=env, capture_output=True, text=True)
    if r.returncode != 0 or not os.path.isfile(outp):
        sys.exit("the Rust half failed:\n" + r.stdout[-3000:] + r.stderr[-3000:])
    return json.load(open(outp))


def tokens_part(work, count):
    """The skills part of the instructions: before (fb64721's catalog) and after (the pinned core), per AI and kind."""
    import rbxskills
    old_src = subprocess.run(["git", "-C", ROOT, "show", OLD_CATALOG + ":rbxskills.py"], capture_output=True,
                             text=True, check=True).stdout
    path = os.path.join(work, "rbxskills_fb64721.py")
    open(path, "w").write(old_src)
    spec = importlib.util.spec_from_file_location("rbxskills_fb64721", path)
    old = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(old)
    rows = []
    for ai in ("claude", "gemini"):
        before = old.catalog(ai)
        for kind in ("game", "plain"):
            after = rbxskills.catalog(ai, kind=kind)
            rows.append((ai, kind, count(before), len(before), count(after), len(after)))
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--models", default=os.environ.get("FRIDAE_RAG_MODELS", ""), help="the Light model folder")
    ap.add_argument("--work", default="", help="scratch folder (default: a new temporary one)")
    a = ap.parse_args()
    if not a.models:
        sys.exit("--models: the folder holding BAAI/bge-small-en-v1.5 (fastembed's cache)")
    work = a.work or tempfile.mkdtemp(prefix="skbench-")
    os.makedirs(work, exist_ok=True)
    began = time.time()
    import rbxskills
    cases = json.load(open(CASES))

    res, best = make_library(work)
    lib = {s["id"] for s in rbxskills.library()}
    print("library: %d skills (%s)" % (len(lib), ", ".join("%s %d" % (k, len(v)) for k, v in res.items())))
    missing = sorted({w for c in cases for w in c["want"] if w not in lib})
    if missing:
        print("want ids not in this library, dropped:", ", ".join(missing))
        for c in cases:
            c["want"] = [w for w in c["want"] if w in lib]
    pinned_only = [c["q"] for c in cases if c["want"] and all(rbxskills.pinned({"id": w}) for w in c["want"])]

    index = rbxskills.index()
    dense, emb = dense_hits(cases, a.models)
    rust = run_rust(work, index, cases, dense)
    rc = rust["cases"]

    print("\n== shipped thresholds: dense min %.2f rel %.2f; keyword min %.2f rel %.2f (>= %d words); gate >= %d "
          "content words; at most %d" % (rust["dense_min"], rust["dense_rel"], rust["keyword_min"], rust["keyword_rel"],
                                          rust["keyword_words"], rust["gate_words"], rust["max"]))
    shipped = {"dense": [line_ids(r["dense_line"]) for r in rc], "keyword": [line_ids(r["keyword_line"]) for r in rc]}
    for path, lo, rel in (("dense", rust["dense_min"], rust["dense_rel"]), ("keyword", rust["keyword_min"], rust["keyword_rel"])):
        again = [within(r[path], lo, rel) if r["gate"] else [] for r in rc]
        assert again == shipped[path], "%s: the table's rule differs from Rust's lines" % path
        print(row(path, stats(cases, shipped[path])))
    print("cases whose every wanted skill is pinned (already in the instructions; never suggested): %d" % len(pinned_only))
    d_ms = 1000 * sum(t for _, _, t in dense) / len(dense) + sum(r["dense_rust_us"] for r in rc) / len(rc) / 1000
    k_ms = sum(r["keyword_us"] for r in rc) / len(rc) / 1000
    print("ms per message: dense %.1f (embedding the message + Rust), keyword %.2f (Rust, debug build)" % (d_ms, k_ms))
    print("top-3 by score alone (no floor; pinned dropped; keyword hits hold >= %d words): dense %d/%d, keyword %d/%d" % (
        rust["keyword_words"],
        sum(1 for c, r in zip(cases, rc) if c["want"] and set(h[0] for h in r["dense"][:3]) & set(c["want"])),
        sum(1 for c in cases if c["want"]),
        sum(1 for c, r in zip(cases, rc) if c["want"] and set(h[0] for h in r["keyword"][:3]) & set(c["want"])),
        sum(1 for c in cases if c["want"])))

    def sweep(path, los, rels, words=(0,)):
        print("\n== %s: thresholds" % path)
        for n in words:
            for rel in rels:
                for lo in los:
                    picks = [within(r[path], lo, rel, words=n) if r["gate"] else [] for r in rc]
                    st = stats(cases, picks)
                    mark = "  <- off-target < 0.33, no follow-up" if st[3] < 1 / 3 and st[4] == 0 else ""
                    label = "min %.2f rel %.2f" % (lo, rel) + (" words %d" % n if n else "")
                    print(row(label, st) + mark)

    sweep("dense", [x / 100 for x in range(57, 70)], [0.01, 0.02, 0.03, 0.04])
    # Rust's keyword hits already hold at least KEYWORD_WORDS of the message's words (fewer is never scored)
    sweep("keyword", [x / 100 for x in range(15, 60, 5)], [0.04, 0.10, 1.0], words=(rust["keyword_words"],))

    print("\n== per case (dense | keyword), shipped thresholds")
    for c, r, d, k in zip(cases, rc, shipped["dense"], shipped["keyword"]):
        tag = "skill" if c["want"] else "follow"
        print("%-6s %-48s | want %s\n        dense %s | keyword %s" % (tag, c["q"][:48], c["want"], d or "-", k or "-"))

    print("\n== the instructions' skills part (tokens by the Light model's tokenizer; characters)")
    for ai, kind, tb, cb, ta, ca in tokens_part(work, emb.count_tokens):
        print("%-6s %-5s before %5d tokens (%6d chars) | after %4d tokens (%5d chars)" % (ai, kind, tb, cb, ta, ca))

    print("\n== OVERLAP %.2f: the import's similar() scores (newcomer <- closest library skill)" % rbxskills.OVERLAP)
    for b, (a_, s) in sorted(best.items(), key=lambda x: -x[1][1])[:20]:
        print("  %.3f %s %s <- %s" % (s, "decision" if s >= rbxskills.OVERLAP else "added   ", b, a_))
    print("\nran in %.0f s" % (time.time() - began))


if __name__ == "__main__":
    main()
