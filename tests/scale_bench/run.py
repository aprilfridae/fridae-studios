#!/usr/bin/env python3
"""Scale bench for Fridae's knowledge search: does it still find the right note when the vault is ten times bigger,
and what does that cost?

  .venv/bin/python tests/scale_bench/run.py                 # full: 1x and 10x, keyword and Light, all question sets
  .venv/bin/python tests/scale_bench/run.py --quick         # a subset corpus (gold files + 5% of the rest) at 1x and 10x
  options: --methods keyword,light  --scales 1,10  --models DIR  --cache-dir DIR  --cold  --rebuild  --out FILE.md

Methods:
- keyword: rbx_vault's search with no knowledge index (rbxvault.run("search")), as the tools answer today.
- light: the rbxrag Engine in process (MemStore, LightEmbedders), asked through rbx_vault's own rag_search over a real
  /query server (rbxrag.serve.make_server), so the tokens counted are what an agent reads.
- heavy: skipped (needs Ollama for its dense model and the Qdrant service; the bench installs and runs neither).

Corpora are made offline by corpus.py and cached in tests/scale_bench/.cache (git-ignored; FRIDAE_SCALE_BENCH_CACHE
or --cache-dir to share one between worktrees). Light indexes are cached by corpus and engine source hash, so an
engine change rebuilds; chunk vectors are cached by text (embed-cache.sqlite), so a rebuild that keeps the chunk
texts is fast. --cold ignores that vector cache (it still fills it) for an honest build time; --rebuild ignores the
index cache. Light needs the Light models: --models, FRIDAE_RAG_MODELS, or the app's
~/Library/Application Support/Fridae Studios/rag/models. Nothing here reaches the network.
"""
import argparse
import datetime
import hashlib
import json
import math
import os
import platform
import resource
import shutil
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import shim  # noqa: E402  (puts the repo on sys.path)

REPO = shim.REPO
K = 5
APP_TIMEOUT = 3.0  # rbx_vault waits this long for the index before answering with the keyword search
SETS = ["paraphrase", "exact", "old33"]
QUICK_SETS = ["paraphrase", "exact"]
EDIT_MARK = "\n\nA line the scale bench added to time an update.\n"


# ------------------------------------------------------------------ small helpers
def say(*a):
    print(*a, file=sys.stderr, flush=True)


def pct(xs, p):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    return xs[min(len(xs) - 1, max(0, int(math.ceil(p / 100 * len(xs))) - 1))]


def rough_tokens(text):
    """About four characters a token: close enough to compare answers."""
    return math.ceil(len(text or "") / 4)


def peak_rss_mb():
    r = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    return r / (1024 * 1024) if sys.platform == "darwin" else r / 1024


def rss_mb():
    try:
        out = subprocess.run(["ps", "-o", "rss=", "-p", str(os.getpid())], capture_output=True, text=True).stdout
        return int(out.strip()) / 1024
    except (OSError, ValueError):
        return None


def dir_mb(path):
    total = 0
    for root, _, fs in os.walk(path):
        for f in fs:
            try:
                total += os.path.getsize(os.path.join(root, f))
            except OSError:
                pass
    return total / (1024 * 1024)


def engine_hash():
    h = hashlib.sha256()
    for root in (os.path.join(REPO, "rbxrag"),):
        for f in sorted(os.listdir(root)):
            if f.endswith(".py"):
                with open(os.path.join(root, f), "rb") as fh:
                    h.update(f.encode() + fh.read())
    with open(os.path.join(REPO, "rbxvault.py"), "rb") as fh:
        h.update(fh.read())
    return h.hexdigest()[:12]


def load_questions():
    with open(os.path.join(HERE, "questions.json"), encoding="utf-8") as f:
        return json.load(f)


def questions_for(qs, sets):
    out = []
    for s in sets:
        for q in qs[s]:
            out.append(dict(q, set=s, shelves=q.get("shelves") or ["all"]))
    return out


def titles_from_text(text):
    """(shelf, title) of each result in rbx_vault's search text, in order."""
    import re
    out = []
    for line in (text or "").splitlines():
        s = line.strip()
        if s.startswith("read: {"):
            try:
                a = json.loads(s[len("read: "):])
                out.append((a.get("shelf"), a.get("title")))
            except ValueError:
                pass
        else:
            m = re.match(r"^(\d+)\. ([^\[].*)$", line)  # the vault-only answer: "1. Title"
            if m:
                out.append(("vault", m.group(2).strip()))
    return out


def dedupe(pairs):
    seen, out = set(), []
    for p in pairs:
        if p not in seen:
            seen.add(p)
            out.append(p)
    return out


def is_gold(title, gold):
    t = (title or "").casefold()
    return any(t == g.casefold() or t.endswith("/" + g.casefold()) for g in gold)


def ranks(results, gold):
    """1-based rank of the first gold result among the first K results (by result, as the agent sees them)."""
    for i, (_, t) in enumerate(results[:K], 1):
        if is_gold(t, gold):
            return i
    return None


def pick_edit_note(corpus):
    """A fixed, ordinary note to change for the update timing (never a gold note)."""
    p = os.path.join(corpus, "vault", "02 Luau Scripting", "Strings Patterns and Formatting.md")
    if os.path.isfile(p):
        return p
    for root, _, fs in sorted(os.walk(os.path.join(corpus, "vault"))):
        for f in sorted(fs):
            if f.endswith(".md") and os.path.basename(root)[:2].isdigit():
                return os.path.join(root, f)
    raise SystemExit("no note to edit in %s" % corpus)


class Edited:
    """Append a line to a note, and put it back exactly (content and mtime) afterwards."""

    def __init__(self, path):
        self.path = path
        st = os.stat(path)
        self.times = (st.st_atime_ns, st.st_mtime_ns)
        with open(path, "rb") as f:
            self.orig = f.read()

    def __enter__(self):
        with open(self.path, "ab") as f:
            f.write(EDIT_MARK.encode())
        os.utime(self.path, ns=(self.times[0], self.times[1] + 1_000_000_000))
        return self

    def restore(self):
        with open(self.path, "wb") as f:
            f.write(self.orig)
        os.utime(self.path, ns=self.times)

    def __exit__(self, *exc):
        self.restore()


# ------------------------------------------------------------------ workers (one process each, so memory is theirs)
def worker_keyword(a):
    shim.point_at(a.corpus)
    qs = questions_for(load_questions(), a.sets.split(","))
    t = time.perf_counter()
    shim.keyword_build()
    build_s = time.perf_counter() - t
    rss_built = rss_mb()
    rows = []
    for q in qs:
        t = time.perf_counter()
        text = shim.keyword_search(q["q"], q["shelves"], K)
        dt = time.perf_counter() - t
        res = dedupe(titles_from_text(text))
        rows.append({"id": q["id"], "set": q["set"], "secs": dt, "tokens": rough_tokens(text), "chars": len(text),
                     "rank": ranks(res, q["gold"]), "top": [t for _, t in res[:K]]})
    # one note changes: the next search rebuilds what it must
    note = pick_edit_note(a.corpus)
    q0 = qs[0]
    with Edited(note):
        t = time.perf_counter()
        shim.keyword_search(q0["q"], q0["shelves"], K)
        update_s = time.perf_counter() - t
    t = time.perf_counter()
    shim.keyword_search(q0["q"], q0["shelves"], K)
    restore_s = time.perf_counter() - t
    return {"method": "keyword", "build_s": build_s, "rows": rows, "update_s": update_s, "restore_s": restore_s,
            "update_note": os.path.relpath(note, a.corpus), "update_kind": "full re-parse of the vault index",
            "rss_built_mb": rss_built, "peak_rss_mb": peak_rss_mb(),
            "index_disk_mb": dir_mb(os.path.join(__import__("rbxpaths").WORK, "cache"))}


def _light_setup(a):
    shim.point_at(a.corpus)
    t = time.perf_counter()
    raw = shim.light_embedders(a.models)
    load_s = time.perf_counter() - t
    emb = shim.EmbedCache(raw, os.path.join(a.cache_dir, "embed-cache.sqlite"), read=not a.cold)
    return emb, load_s


def worker_light_build(a):
    emb, load_s = _light_setup(a)
    shutil.rmtree(a.data, ignore_errors=True)
    os.makedirs(a.data)
    eng = shim.light_engine(a.data, emb)
    last = [0.0, time.time()]
    t0 = time.perf_counter()

    def progress(step, fraction):
        if fraction - last[0] >= 0.02 or (fraction >= 1.0 and last[0] < 1.0):
            el = time.perf_counter() - t0
            eta = el / fraction * (1 - fraction) if fraction > 0 else 0
            say("  light build %s %5.1f%%  %6.0f s elapsed, about %6.0f s to go" % (step, 100 * fraction, el, eta))
            last[0] = fraction
    load_before = os.getloadavg()
    stats = shim.engine_sync(eng, progress=progress, full=True)
    build_s = time.perf_counter() - t0
    meta = {"build_s": build_s, "stats": stats, "embed_hits": emb.hits, "embed_misses": emb.misses,
            "embed_s": emb.embed_s, "cold": bool(a.cold), "model_load_s": load_s,
            "peak_rss_build_mb": peak_rss_mb(), "loadavg_before": load_before, "loadavg_after": os.getloadavg(),
            "when": datetime.datetime.now().isoformat(timespec="seconds"), "engine": engine_hash()}
    with open(os.path.join(a.data, "bench-meta.json"), "w") as f:
        json.dump(meta, f, indent=1)
    return meta


def worker_light_query(a):
    emb, load_s = _light_setup(a)
    import gc
    gc.collect()
    rss_models = rss_mb()
    eng = shim.light_engine(a.data, emb)
    t = time.perf_counter()
    stats = shim.engine_sync(eng)
    open_s = time.perf_counter() - t
    gc.collect()
    rss_open = rss_mb()
    t = time.perf_counter()
    shim.quick_refresh(eng)
    refresh_s = time.perf_counter() - t
    tmp = shim.point_at(a.corpus)  # fresh work folder for rbx_vault's own caches
    ep = shim.RagEndpoint(eng, tmp)
    qs = questions_for(load_questions(), a.sets.split(","))
    rows = []
    try:
        for q in qs:
            t = time.perf_counter()
            text, secs, hits = ep.search(q["q"], q["shelves"], K)
            e2e = time.perf_counter() - t
            res = dedupe([(h.get("shelf"), h.get("title")) for h in hits])
            rows.append({"id": q["id"], "set": q["set"], "secs": secs, "e2e_secs": e2e, "tokens": rough_tokens(text),
                         "chars": len(text or ""), "rank": ranks(res, q["gold"]), "top": [t for _, t in res[:K]],
                         "over_app_timeout": secs is not None and secs > APP_TIMEOUT})
        # warm second pass, engine time only
        warm = []
        for q in qs:
            t = time.perf_counter()
            shim.engine_query(eng, q["q"], shim.shelves_for(q["shelves"]), K)
            warm.append(time.perf_counter() - t)
    finally:
        ep.close()
    note = pick_edit_note(a.corpus)
    ed = Edited(note)
    with ed:
        t = time.perf_counter()
        up = shim.quick_refresh(eng)
        update_s = time.perf_counter() - t
    t = time.perf_counter()
    back = shim.quick_refresh(eng)
    restore_s = time.perf_counter() - t
    return {"method": "light", "rows": rows, "warm_secs": warm, "open_s": open_s, "open_stats": stats,
            "refresh_nochange_s": refresh_s, "update_s": update_s, "update_stats": up, "restore_s": restore_s,
            "restore_stats": back, "update_note": os.path.relpath(note, a.corpus),
            "rss_models_mb": rss_models, "rss_open_mb": rss_open, "peak_rss_mb": peak_rss_mb(),
            "index_disk_mb": dir_mb(os.path.join(a.data, "light")), "model_load_s": load_s,
            "embed_hits": emb.hits, "embed_misses": emb.misses}


def worker_main(argv):
    p = argparse.ArgumentParser()
    p.add_argument("--method", required=True)
    p.add_argument("--phase", default="query")
    p.add_argument("--corpus", required=True)
    p.add_argument("--sets", required=True)
    p.add_argument("--data")
    p.add_argument("--models")
    p.add_argument("--cache-dir")
    p.add_argument("--cold", action="store_true")
    p.add_argument("--out", required=True)
    a = p.parse_args(argv)
    if a.method == "keyword":
        res = worker_keyword(a)
    elif a.phase == "build":
        res = worker_light_build(a)
    else:
        res = worker_light_query(a)
    with open(a.out, "w") as f:
        json.dump(res, f)


# ------------------------------------------------------------------ the orchestrator
def find_models(arg):
    for c in (arg, os.environ.get("FRIDAE_RAG_MODELS"),
              os.path.expanduser("~/Library/Application Support/Fridae Studios/rag/models")):
        if c and os.path.isdir(c):
            return os.path.abspath(c)
    return None


def run_worker(args, out):
    cmd = [sys.executable, os.path.abspath(__file__), "_worker"] + args + ["--out", out]
    t = time.perf_counter()
    r = subprocess.run(cmd)
    if r.returncode != 0:
        raise RuntimeError("worker failed (%d): %s" % (r.returncode, " ".join(args[:4])))
    with open(out) as f:
        res = json.load(f)
    res["wall_s"] = time.perf_counter() - t
    return res


def summarise(res, sets):
    rows = res["rows"]
    out = {}
    for s in sets + ["all"]:
        rr = [r for r in rows if s == "all" or r["set"] == s]
        if not rr:
            continue
        out[s] = {"n": len(rr)}
        for k in (1, 3, 5):
            out[s]["hit@%d" % k] = sum(1 for r in rr if r["rank"] and r["rank"] <= k)
    taste = [r for r in rows if r["id"].startswith("T")]
    if taste:
        out["taste"] = {"n": len(taste), **{"hit@%d" % k: sum(1 for r in taste if r["rank"] and r["rank"] <= k)
                                            for k in (1, 3, 5)}}
    secs = [r["secs"] for r in rows]
    tok = [r["tokens"] for r in rows]
    out["cost"] = {"tokens_mean": sum(tok) / len(tok), "tokens_p95": pct(tok, 95),
                   "q_p50": pct(secs, 50), "q_p95": pct(secs, 95)}
    if res["method"] == "light":
        e2e = [r["e2e_secs"] for r in rows]
        out["cost"].update({"e2e_p50": pct(e2e, 50), "e2e_p95": pct(e2e, 95),
                            "warm_p50": pct(res["warm_secs"], 50), "warm_p95": pct(res["warm_secs"], 95),
                            "over_app_timeout": sum(1 for r in rows if r["over_app_timeout"])})
    return out


def fmt_hits(sm, s):
    if s not in sm:
        return "-"
    d = sm[s]
    return "%d / %d / %d (of %d)" % (d["hit@1"], d["hit@3"], d["hit@5"], d["n"])


def f2(x, unit=""):
    return "-" if x is None else ("%.2f%s" % (x, unit) if x < 10 else "%.0f%s" % (x, unit))


def mb(x):
    return "-" if x is None else ("%.0f MB" % x if x < 1024 else "%.2f GB" % (x / 1024))


def report(results, sets, scales, meta):
    L = []
    L.append("Run %s on %s (%s, %d CPUs); engine source hash %s; load average at start %s." % (
        meta["when"], platform.node() and "this Mac", platform.machine(), os.cpu_count(), meta["engine"],
        " / ".join("%.1f" % x for x in meta["loadavg"])))
    L.append("")
    L.append("Corpora:")
    L.append("")
    L.append("| Scale | Vault notes | Guides | Team | Distractor notes | Estimated chunks | Engine chunks |")
    L.append("|---|---:|---:|---:|---:|---:|---:|")
    for sc in scales:
        c = meta["corpora"][str(sc)]
        lm = results.get(("light", sc), {}).get("build", {})
        ch = (lm.get("stats") or {}).get("chunks")
        L.append("| %dx | %d | %d | %d | %d | %s | %s |" % (
            sc, c["base_notes"], c["base_guides"], c["team"], sum(c["distractors"].values()),
            format(c["est_chunks"], ","), format(ch, ",") if ch else "-"))
    L.append("")
    L.append("### Quality: hit@1 / hit@3 / hit@5")
    L.append("")
    L.append("| Method | Scale | " + " | ".join(sets) + " | of which taste |")
    L.append("|---|---|" + "---|" * len(sets) + "---|")
    for (m, sc), r in sorted(results.items(), key=lambda x: (x[0][0], x[0][1])):
        if "summary" not in r:
            continue
        sm = r["summary"]
        L.append("| %s | %dx | %s | %s |" % (m, sc, " | ".join(fmt_hits(sm, s) for s in sets), fmt_hits(sm, "taste")))
    for m in ("heavy",):
        L.append("| %s | - | %s |" % (m, " | ".join(["skipped"] * (len(sets) + 1))))
    L.append("")
    if len(scales) >= 2:
        lo, hi = scales[0], scales[-1]
        L.append("### Headline: the drop from %dx to %dx (hit@3, and hit@5)" % (lo, hi))
        L.append("")
        L.append("| Method | Set | hit@3 %dx | hit@3 %dx | change | hit@5 %dx | hit@5 %dx | change |" % (lo, hi, lo, hi))
        L.append("|---|---|---:|---:|---:|---:|---:|---:|")
        for m in ("keyword", "light"):
            a, b = results.get((m, lo), {}).get("summary"), results.get((m, hi), {}).get("summary")
            if not a or not b:
                continue
            for s in sets + ["taste"]:
                if s not in a or s not in b:
                    continue
                n = a[s]["n"]

                def p(x):
                    return 100.0 * x / n
                L.append("| %s | %s | %.0f%% | %.0f%% | %+.0f pts | %.0f%% | %.0f%% | %+.0f pts |" % (
                    m, s, p(a[s]["hit@3"]), p(b[s]["hit@3"]), p(b[s]["hit@3"]) - p(a[s]["hit@3"]),
                    p(a[s]["hit@5"]), p(b[s]["hit@5"]), p(b[s]["hit@5"]) - p(a[s]["hit@5"])))
        L.append("")
    L.append("### Cost")
    L.append("")
    L.append("| Method | Scale | Tokens returned per query (mean / p95) | Query p50 / p95 | rbx_vault end to end p50 / p95 "
             "| Queries over rbx_vault's 3 s | Full build | Update after one note changes | Peak memory (build / serving) "
             "| Store in memory | Index on disk |")
    L.append("|---|---|---|---|---|---|---|---|---|---|---|")
    for (m, sc), r in sorted(results.items(), key=lambda x: (x[0][0], x[0][1])):
        if "summary" not in r:
            continue
        c = r["summary"]["cost"]
        q = r["query"]
        if m == "keyword":
            L.append("| keyword | %dx | %.0f / %d | %s / %s | same | - | %s (index parse) | %s (next search re-parses all) "
                     "| - / %s | - | %s |" % (
                         sc, c["tokens_mean"], c["tokens_p95"], f2(c["q_p50"], " s"), f2(c["q_p95"], " s"),
                         f2(q["build_s"], " s"), f2(q["update_s"], " s"), mb(q["peak_rss_mb"]), mb(q["index_disk_mb"])))
        else:
            b = r.get("build", {})
            cold = b.get("embed_misses", 0) and not b.get("embed_hits")
            bt = f2(b.get("build_s"), " s") + ("" if cold else " (vector cache: %d of %d hits)" % (
                b.get("embed_hits", 0), b.get("embed_hits", 0) + b.get("embed_misses", 0)))
            us = q.get("update_stats") or {}
            L.append("| light | %dx | %.0f / %d | %s / %s | %s / %s | %d | %s | %s (embedded %s chunks; whole store "
                     "rewritten) | %s / %s | %s | %s |" % (
                         sc, c["tokens_mean"], c["tokens_p95"], f2(c["q_p50"], " s"), f2(c["q_p95"], " s"),
                         f2(c["e2e_p50"], " s"), f2(c["e2e_p95"], " s"), c["over_app_timeout"], bt,
                         f2(q["update_s"], " s"), us.get("embedded", "?"), mb(b.get("peak_rss_build_mb")),
                         mb(q["peak_rss_mb"]), mb((q["rss_open_mb"] or 0) - (q["rss_models_mb"] or 0)),
                         mb(q["index_disk_mb"])))
    L.append("| heavy | - | skipped: needs Ollama (dense mxbai-embed-large) and the Qdrant service | | | | | | | | |")
    L.append("")
    lt = [(sc, r["query"]) for (m, sc), r in sorted(results.items()) if m == "light" and "query" in r]
    if lt:
        L.append("Light, other timings: " + "; ".join(
            "%dx: open the saved index %s, the per-query refresh with nothing changed %s, warm engine query p50 %s / "
            "p95 %s, model load %s" % (sc, f2(q["open_s"], " s"), f2(q["refresh_nochange_s"], " s"),
                                      f2(pct(q["warm_secs"], 50), " s"), f2(pct(q["warm_secs"], 95), " s"),
                                      f2(q["model_load_s"], " s")) for sc, q in lt) + ".")
        L.append("")
    for (m, sc), r in sorted(results.items()):
        b = r.get("build") or {}
        if m == "light" and b.get("embed_hits") and b.get("embed_misses"):
            per = b["embed_s"] / b["embed_misses"]
            L.append("Light %dx build read %d chunk vectors from the cache; at the measured %.1f chunks/s embedding "
                     "rate a cold build would take about %s." % (
                         sc, b["embed_hits"], 1 / per, f2(b["build_s"] + b["embed_hits"] * per, " s")))
            L.append("")
    # straight-line extrapolation to 100x from the two measured scales
    if len(scales) >= 2 and scales[-1] > scales[0]:
        lo, hi = scales[0], scales[-1]
        far = hi * 10

        def ext(v1, v2):
            if v1 is None or v2 is None:
                return None
            return v2 + (v2 - v1) * (far - hi) / (hi - lo)
        lines = []
        for m in ("keyword", "light"):
            a, b = results.get((m, lo)), results.get((m, hi))
            if not a or not b:
                continue
            qa, qb = a["query"], b["query"]
            if m == "keyword":
                lines.append("| keyword | query p50 | %s | %s | %s |" % (
                    f2(qa and a["summary"]["cost"]["q_p50"], " s"), f2(b["summary"]["cost"]["q_p50"], " s"),
                    f2(ext(a["summary"]["cost"]["q_p50"], b["summary"]["cost"]["q_p50"]), " s")))
                lines.append("| keyword | first search after a change | %s | %s | %s |" % (
                    f2(qa["update_s"], " s"), f2(qb["update_s"], " s"), f2(ext(qa["update_s"], qb["update_s"]), " s")))
                lines.append("| keyword | peak memory | %s | %s | %s |" % (
                    mb(qa["peak_rss_mb"]), mb(qb["peak_rss_mb"]), mb(ext(qa["peak_rss_mb"], qb["peak_rss_mb"]))))
                continue
            sa = (qa["rss_open_mb"] or 0) - (qa["rss_models_mb"] or 0)
            sb = (qb["rss_open_mb"] or 0) - (qb["rss_models_mb"] or 0)
            ba, bb = a["build"], b["build"]

            def cold(x):
                if x.get("embed_hits") and x.get("embed_misses"):
                    return x["build_s"] + x["embed_hits"] * x["embed_s"] / x["embed_misses"]
                return x.get("build_s")
            rows = [("store in memory", mb(sa), mb(sb), mb(ext(sa, sb))),
                    ("index on disk", mb(qa["index_disk_mb"]), mb(qb["index_disk_mb"]),
                     mb(ext(qa["index_disk_mb"], qb["index_disk_mb"]))),
                    ("peak memory while building", mb(ba.get("peak_rss_build_mb")), mb(bb.get("peak_rss_build_mb")),
                     mb(ext(ba.get("peak_rss_build_mb"), bb.get("peak_rss_build_mb")))),
                    ("full build (cold)", f2(cold(ba), " s"), f2(cold(bb), " s"), f2(ext(cold(ba), cold(bb)), " s")),
                    ("update after one note changes", f2(qa["update_s"], " s"), f2(qb["update_s"], " s"),
                     f2(ext(qa["update_s"], qb["update_s"]), " s")),
                    ("query p50", f2(a["summary"]["cost"]["q_p50"], " s"), f2(b["summary"]["cost"]["q_p50"], " s"),
                     f2(ext(a["summary"]["cost"]["q_p50"], b["summary"]["cost"]["q_p50"]), " s"))]
            lines += ["| light | %s | %s | %s | %s |" % r for r in rows]
        if lines:
            L.append("### Straight-line extrapolation to %dx (from the %dx and %dx measurements)" % (far, lo, hi))
            L.append("")
            L.append("| Method | Measure | %dx | %dx | %dx (extrapolated) |" % (lo, hi, far))
            L.append("|---|---|---:|---:|---:|")
            L += lines
            L.append("")
    # what changed between scales, per question
    if len(scales) >= 2:
        lo, hi = scales[0], scales[-1]
        for m in ("keyword", "light"):
            a, b = results.get((m, lo), {}).get("query"), results.get((m, hi), {}).get("query")
            if not a or not b:
                continue
            bb = {r["id"]: r for r in b["rows"]}
            lost = [r for r in a["rows"] if r["rank"] and r["rank"] <= 3 and not (bb[r["id"]]["rank"] and
                                                                                   bb[r["id"]]["rank"] <= 3)]
            if lost:
                L.append("%s lost at %dx (in the top 3 at %dx, not at %dx): %s." % (
                    m.capitalize(), hi, lo, hi, "; ".join("%s (now: %s)" % (r["id"], ", ".join(
                        bb[r["id"]]["top"][:3]) or "nothing") for r in lost)))
                L.append("")
    return "\n".join(L)


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "_worker":
        return worker_main(sys.argv[2:])
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--quick", action="store_true")
    p.add_argument("--methods", default="keyword,light")
    p.add_argument("--scales", default="1,10")
    p.add_argument("--sets")
    p.add_argument("--models")
    p.add_argument("--cache-dir", default=os.environ.get("FRIDAE_SCALE_BENCH_CACHE") or os.path.join(HERE, ".cache"))
    p.add_argument("--cold", action="store_true", help="do not read the chunk-vector cache (build time from scratch)")
    p.add_argument("--rebuild", action="store_true", help="ignore cached Light indexes")
    p.add_argument("--out", help="also write the report (Markdown) here")
    a = p.parse_args()
    import corpus
    qs = load_questions()
    sets = a.sets.split(",") if a.sets else (QUICK_SETS if a.quick else SETS)
    scales = [int(x) for x in a.scales.split(",")]
    methods = a.methods.split(",")
    cache = os.path.abspath(a.cache_dir)
    os.makedirs(cache, exist_ok=True)
    models = find_models(a.models)
    meta = {"when": datetime.datetime.now().isoformat(timespec="seconds"), "engine": engine_hash(),
            "loadavg": os.getloadavg(), "corpora": {}, "quick": a.quick, "sets": sets}
    results = {}
    work = os.path.join(cache, "runs", time.strftime("%Y%m%d-%H%M%S"))
    os.makedirs(work, exist_ok=True)
    for sc in scales:
        say("== %dx%s" % (sc, " (quick)" if a.quick else ""))
        cdir, csum = corpus.ensure(cache, qs, sc, a.quick, log=say)
        meta["corpora"][str(sc)] = csum
        key = os.path.basename(cdir)
        for m in methods:
            if m == "heavy":
                continue
            base = ["--method", m, "--corpus", cdir, "--sets", ",".join(sets), "--cache-dir", cache]
            if m == "keyword":
                say("-- keyword %dx" % sc)
                r = run_worker(base, os.path.join(work, "keyword-%d.json" % sc))
                results[(m, sc)] = {"query": r, "summary": summarise(r, sets)}
                continue
            if not models:
                say("-- light skipped: no Light models (give --models or FRIDAE_RAG_MODELS)")
                continue
            data = os.path.join(cache, "index", "%s-%s" % (key, engine_hash()))
            base += ["--models", models, "--data", data] + (["--cold"] if a.cold else [])
            bm = os.path.join(data, "bench-meta.json")
            if a.rebuild or not os.path.isfile(bm):
                say("-- light %dx: building the index" % sc)
                run_worker(base + ["--phase", "build"], os.path.join(work, "light-build-%d.json" % sc))
            with open(bm) as f:
                build = json.load(f)
            say("-- light %dx: queries" % sc)
            r = run_worker(base + ["--phase", "query"], os.path.join(work, "light-%d.json" % sc))
            results[(m, sc)] = {"build": build, "query": r, "summary": summarise(r, sets)}
    text = report(results, sets, scales, meta)
    with open(os.path.join(work, "results.json"), "w") as f:
        json.dump({"meta": meta, "results": {"%s-%d" % k: v for k, v in results.items()}}, f, indent=1)
    with open(os.path.join(work, "report.md"), "w") as f:
        f.write(text + "\n")
    print(text)
    if a.out:
        with open(a.out, "w") as f:
            f.write(text + "\n")
    say("raw results: %s" % work)


if __name__ == "__main__":
    main()
