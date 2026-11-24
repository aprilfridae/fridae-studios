"""The knowledge exam's analysis: scores from the saved stage results and the Markdown report (exam.py's analysis
stage). Reads only; every number comes from the stage files."""
import datetime
import os
import platform
import random

import exam_lib as L

ANSWERABLE = ("paraphrase", "exact", "old33", "typo", "multihop", "stale", "guide")
SIGNALS = (("kw_cov", "keyword coverage"), ("kw_bm25", "keyword BM25"), ("dense_top", "dense cosine"),
           ("rerank_top", "reranker score"))


# ------------------------------------------------------------------ retrieval
def rank(row, q, k=None, exp=None):
    gold = q.get("gold") or []
    return L.first_rank(row.get("titles") or [], exp(gold) if exp else gold, k)


def expander(ex, scale):
    """gold titles -> the gold titles plus every distractor made from them (split, summary, rewording, or a mix
    holding one of their sections; not the echo notes, which only look like them), from the corpus manifest."""
    rev = {}
    for t, m in ex.derived(scale).items():
        if m.get("kind") == "echo":
            continue
        for f in m.get("from") or []:
            rev.setdefault(f.casefold(), []).append(t)
    return lambda gold: list(gold) + [t for g in gold for t in rev.get(g.casefold(), [])]


def set_table(rows, qs, sets, exp=None):
    """Hit rates per set; exp (expander) scores content-right instead of strict."""
    out = {}
    for s in sets:
        rr = [r for r in rows if r["set"] == s and qs.get(r["id"], {}).get("gold")]
        if rr:
            out[s] = L.hit_rates([rank(r, qs[r["id"]], None, exp) for r in rr])
    rr = [r for r in rows if r["set"] in ANSWERABLE and qs.get(r["id"], {}).get("gold")]
    if rr:
        out["answerable"] = L.hit_rates([rank(r, qs[r["id"]], None, exp) for r in rr])
    return out


def drop(ex, small, large, qs):
    """Of the answerable questions found strictly (gold in the top 5) at the small scale, the ones lost at the large
    scale: still content-right there (a copy of the gold note ranked instead) or really missed."""
    exp = expander(ex, large[0])
    lost_copy = lost_real = kept = 0
    by = {r["id"]: r for r in large[1]}
    for r in small:
        q = qs.get(r["id"]) or {}
        if r["set"] not in ANSWERABLE or not q.get("gold") or not rank(r, q, 5) or r["id"] not in by:
            continue
        b = by[r["id"]]
        if rank(b, q, 5):
            kept += 1
        elif rank(b, q, 5, exp):
            lost_copy += 1
        else:
            lost_real += 1
    return {"kept": kept, "copy": lost_copy, "real": lost_real}


def multihop(rows, qs):
    rr = [r for r in rows if r["set"] == "multihop"]
    both = either = 0
    for r in rr:
        pair = qs[r["id"]].get("pair") or qs[r["id"]]["gold"]
        got = [L.first_rank(r["titles"], [g], 5) for g in pair]
        both += all(got)
        either += any(got)
    return {"n": len(rr), "both@5": both / len(rr) if rr else None, "either@5": either / len(rr) if rr else None}


def stale(rows, qs):
    out = {}
    for half, marked in (("marked", True), ("text only", False)):
        rr = [r for r in rows if r["set"] == "stale" and qs[r["id"]].get("marked") == marked]
        if not rr:
            continue
        above = new3 = old3 = old5 = 0
        old_ranks = []
        for r in rr:
            q = qs[r["id"]]
            n = L.first_rank(r["titles"], q["gold"])
            o = L.first_rank(r["titles"], [q["old"]])
            above += bool(n and (not o or n < o))
            new3 += bool(n and n <= 3)
            old3 += bool(o and o <= 3)
            old5 += bool(o and o <= 5)
            if o and o <= 5:
                old_ranks.append(o)
        out[half] = {"n": len(rr), "new_above_old": above / len(rr), "new@3": new3 / len(rr),
                     "old@3": old3 / len(rr), "old@5": old5 / len(rr), "old_rank": L.mean(old_ranks)}
    return out


def guides(rows, text_rows, qs):
    rr = [r for r in rows if r["set"] == "guide"]
    t5 = {r["id"]: r for r in text_rows or []}
    found = sum(1 for r in rr if rank(r, qs[r["id"]], 5))
    tagged = sum(1 for r in rr if r["id"] in t5 and t5[r["id"]].get("guide_tag") and
                 L.first_rank(t5[r["id"]]["titles"], qs[r["id"]]["gold"], 5))
    return {"n": len(rr), "found@5": found / len(rr) if rr else None,
            "found and tagged@5": tagged / len(rr) if rr and t5 else None}


def scope(rows, qs):
    rr = [r for r in rows if r["set"] == "scope"]
    if not rr:
        return {}
    def share(key, gold_key):
        return sum(1 for r in rr if L.first_rank(r.get(key) or [], qs[r["id"]][gold_key] if gold_key == "gold"
                                                 else qs[r["id"]].get(gold_key) or [], 3)) / len(rr)
    return {"n": len(rr), "game note @3, no filter": share("titles", "gold"),
            "game note @3, game=this": share("titles_this", "gold"),
            "general note @3, game=general": share("titles_general", "general"),
            "general note @3, no filter": share("titles", "general")}


def thresholds(rows, qs):
    out = {}
    pos_rows = [r for r in rows if r["set"] in ANSWERABLE]
    neg_rows = [r for r in rows if r["set"] == "negative"]
    for key, label in SIGNALS:
        b = L.best_threshold([r.get(key) for r in pos_rows], [r.get(key) for r in neg_rows])
        if b:
            out[label] = b
    return out


# ------------------------------------------------------------------ answers
def first_token(rec):
    calls = rec.get("calls")
    for c in calls if isinstance(calls, list) else []:
        if c.get("what") == "write":
            for t in c.get("tap") or []:
                ft = (t.get("first") or {}).get("session.step.first-token")
                if ft is not None:
                    return ft
    return None


def tokens(rec):
    """(input, output, reasoning, model calls) of one answer."""
    if "calls" in rec and isinstance(rec["calls"], list):
        i = o = rs = 0
        n = 0
        for c in rec["calls"]:
            n += 1
            taps = c.get("tap") or []
            if taps:
                for t in taps:
                    tk = t.get("tokens") or {}
                    i += tk.get("input", 0) + tk.get("cache_read", 0)
                    o += tk.get("output", 0)
                    rs += tk.get("reasoning", 0)
            else:
                u = c.get("usage") or {}
                i += (u.get("input") or 0) + (u.get("cache_read") or 0)
                o += u.get("output") or 0
        return i, o, rs, n
    u = rec.get("usage") or {}
    return u.get("input") or 0, u.get("output") or 0, None, rec.get("calls") or 1


def answer_scores(ex, name, recs, verdicts):
    by = {e["id"]: e for e in ex.entries}
    rows = []
    for qid, e in by.items():
        rec = recs.get(qid)
        v = (verdicts.get("%s|%s" % (name, qid)) or {}).get("verdict") if rec else None
        s = L.score(e, v) if v else None
        rows.append((e, rec, v, s))
    return rows


def answer_summary(rows):
    judged = [(e, r, v, s) for e, r, v, s in rows if s]
    ans = [x for x in judged if x[0]["expect"] == "answer"]
    neg = [x for x in judged if x[0]["expect"] == "not_found"]
    gd = [x for x in judged if x[0]["expect"] == "guide"]
    st = [x for x in judged if x[0].get("stale_old")]
    recs = [r for _, r, _, _ in rows if r]
    tk = [tokens(r) for r in recs]
    return {
        "n": len(rows), "answered": len(recs), "judged": len(judged),
        "correct": L.mean([1.0 if s["correct"] else 0.0 for _, _, _, s in judged]),
        "recall": L.mean([s["recall"] for _, _, _, s in ans]),
        "wrong": L.mean([1.0 if x[3]["wrong"] else 0.0 for x in judged if x[0]["expect"] != "not_found"]),
        "neg_abstained": (sum(1 for x in neg if x[3]["abstained"]), len(neg)),
        "false_not_found": L.mean([1.0 if s["false_not_found"] else 0.0 for _, _, _, s in ans]),
        "guide_ok": (sum(1 for x in gd if x[3]["correct"]), len(gd)),
        "stale_old_fact": (sum(1 for x in st if any(x[2]["wrong"])), len(st)),
        "in": L.mean([t[0] for t in tk]), "out": L.mean([t[1] for t in tk]),
        "reasoning": L.mean([t[2] for t in tk if t[2] is not None]),
        "calls": L.mean([t[3] for t in tk]),
        "p50": L.pct([r.get("secs") for r in recs], 50), "p95": L.pct([r.get("secs") for r in recs], 95),
        "first_token_p50": L.pct([first_token(r) for r in recs], 50),
        "format_fail": sum(1 for r in recs if "format_ok" in r and not r["format_ok"]),
        "errors": sum(1 for r in recs if r.get("error")),
    }


def vault_conflicts(exam):
    """[(answer id, what the vault's notes disagree on)] from the answer keys marked vault_conflict."""
    return [(a["id"], a["vault_conflict"]) for a in exam.get("answers") or [] if a.get("vault_conflict")]


def config_scale(name):
    return int(name.split("-")[2].rstrip("x"))


def copy_only(ex, name, rows, passages):
    """Answers whose passages held a copy of the gold note (a distractor made from it) but not the note itself."""
    exp = expander(ex, config_scale(name))
    n = 0
    for e, rec, v, s in rows:
        gold = e.get("gold") or []
        if not rec or not gold:
            continue
        titles = [h.get("title") for p in (passages.get(e["id"]) or {}).get("parts", []) for h in p.get("hits") or []]
        if not any(L.is_gold(t, gold) for t in titles) and any(L.is_gold(t, exp(gold)) for t in titles):
            n += 1
    return n


def failures(ex, name, rows, passages):
    """{failure class: [(entry, record, verdict)]}. A passage from a copy of the gold note counts as found."""
    exp = expander(ex, config_scale(name))
    out = {}
    for e, rec, v, s in rows:
        if not rec or not v:
            continue
        r = dict(rec)
        r["passages"] = [p.get("hits") or [] for p in (passages.get(e["id"]) or {}).get("parts", [])]
        if "writer_raw" in rec:
            calls = rec.get("calls") or []
            r["failed_writer"] = rec.get("writer_raw") is None and any(c.get("failed") for c in calls)
        cat = L.classify(e, v, r, exp(e.get("gold") or []), e.get("stale_old"))
        if cat:
            out.setdefault(cat, []).append((e, rec, v))
    return out


# ------------------------------------------------------------------ the report
def build(ex):
    a = ex.a
    p = ex.plan()
    rk, ak = ex.key("retrieval"), ex.key("ablations")
    qs = ex.qs
    out = []
    w = out.append
    w("# Knowledge exam%s" % (" (quick smoke)" if a.quick else ""))
    w("")
    w("Made %s on %s (%s, Python %s). Corpora: %s. Search code %s." % (
        datetime.datetime.now().isoformat(timespec="minutes"), platform.node() and "this Mac", platform.machine(),
        platform.python_version(), ", ".join("%dx = %s (about %d chunks)" % (s, ex.corpus_name(s),
                                                                          ex.corpus_for(s)[1]["est_chunks"])
                                             for s in sorted(ex.corpora)), ex.key("retrieval")))
    run = getattr(ex, "run_meta", None) or {}
    w("")
    w("Code measured: commit `%s` on `%s`. Each saved result carries the commit it was measured at (`_measured`); "
      "results from other commits: %s." % (run.get("commit", "?"), run.get("branch", "?"), ", ".join(
          sorted(other_commits(ex, run.get("commit")))) or "none"))
    if a.quick:
        w("")
        w("**Quick smoke:** a sampled corpus, a few questions per set and %d answer questions. These numbers check "
          "that every stage runs; they are far too few to compare methods." % len(ex.entries))
    w("")

    # ---------------------------------------------------------- retrieval
    w("## 1. Retrieval")
    w("")
    w("Hit@k: the gold note among the first k distinct notes the engine returns (asked for %d). MRR: mean of 1/rank. "
      "No model calls; the same questions for every method." % 8)
    w("")
    res = {}
    for m, sc in p["retrieval"]:
        r = ex.cache.get("retrieval", rk, "%s-%dx" % (m, sc))
        if r:
            res[(m, sc)] = r
    sets = ["paraphrase", "exact", "old33", "typo", "guide", "stale", "multihop", "answerable"]
    rows = []
    for (m, sc), r in res.items():
        t = set_table(r["rows"]["app"], qs, sets[:-1])
        c = set_table(r["rows"]["app"], qs, sets[:-1], expander(ex, sc))
        for s in sets:
            if s in t:
                x, y = t[s], c[s]
                rows.append([m, "%dx" % sc, s, x["n"], L.pc(x["hit@1"]), L.pc(x["hit@3"]), L.pc(x["hit@5"]),
                             L.f2(x["mrr"]), L.pc(y["hit@1"]), L.pc(y["hit@3"]), L.pc(y["hit@5"])])
    w(L.md_table(["method", "scale", "set", "n", "hit@1", "hit@3", "hit@5", "MRR", "content-right hit@1", "hit@3",
                  "hit@5"], rows) if rows else "(no results)")
    w("")
    w("Strict: the gold note itself. Content-right: the gold note or a distractor the corpus made from it (a split, "
      "summary or rewording of it, or a mix holding one of its sections; corpus.py records each distractor's source "
      "notes in distractors.json). Echo notes, which only borrow the gold note's title words, never count.")
    w("")
    rows = []
    for m in sorted({m for m, _ in res}):
        ss = sorted(s for mm, s in res if mm == m)
        if len(ss) > 1:
            d = drop(ex, res[(m, ss[0])]["rows"]["app"], (ss[-1], res[(m, ss[-1])]["rows"]["app"]), qs)
            lost = d["copy"] + d["real"]
            rows.append([m, "%dx → %dx" % (ss[0], ss[-1]), d["kept"] + lost, d["kept"], lost,
                         "%d (%s)" % (d["copy"], L.pc(d["copy"] / lost) if lost else "–"), d["real"]])
    if rows:
        w("Where the bigger vault's drop comes from (answerable questions with the gold note in the top 5 at the "
          "smaller scale):")
        w("")
        w(L.md_table(["method", "scales", "found at small", "still found", "lost", "lost to a copy (right content)",
                      "really missed"], rows))
        w("")
    w("Multi-hop, stale pairs, guides and game scope:")
    w("")
    rows = []
    for (m, sc), r in res.items():
        app = r["rows"]["app"]
        mh = multihop(app, qs)
        stl = stale(app, qs)
        gd = guides(app, (r.get("text") or {}).get("5"), qs)
        scp = scope(app, qs)
        rows.append([m, "%dx" % sc, "%s / %s" % (L.pc(mh["both@5"]), L.pc(mh["either@5"])),
                     " / ".join("%s %s (outdated in top 5: %s, mean rank %s)" % (
                         h, L.pc(x["new_above_old"]), L.pc(x["old@5"]), L.f2(x["old_rank"], 1))
                         for h, x in stl.items()),
                     "%s / %s" % (L.pc(gd["found@5"]), L.pc(gd["found and tagged@5"])),
                     "%s → %s" % (L.pc(scp.get("game note @3, no filter")), L.pc(scp.get("game note @3, game=this"))),
                     L.pc(scp.get("general note @3, game=general"))])
    w(L.md_table(["method", "scale", "multi-hop both@5 / either@5", "stale: new above old (marked / text only)",
                  "guide found@5 / with tag", "game note @3: no filter → game=this", "general @3 with game=general"],
                 rows) if rows else "(none)")
    w("")
    w("Cost per search and per index:")
    w("")
    rows = []
    for (m, sc), r in res.items():
        tx = r.get("text") or {}
        b = r.get("build") or ({"build_s": r["build_s"], "keyword": True} if r.get("build_s") is not None else {})
        app = r["rows"]["app"]
        e2e = [x["e2e_secs"] for x in tx.get("5", [])]
        slow = sum(1 for x in tx.get("5", []) if x.get("over_app_timeout"))
        rows.append([m, "%dx" % sc] + [L.f2(L.mean([x["tokens"] for x in tx.get(str(k), [])]), 0) for k in (3, 5, 8)] +
                    ["%s / %s" % (L.f2(L.pct([x["secs"] for x in app], 50), 3), L.f2(L.pct([x["secs"] for x in app], 95), 3)),
                     "%s / %s" % (L.f2(L.pct(e2e, 50), 3), L.f2(L.pct(e2e, 95), 3)), slow,
                     L.fmt_secs(b.get("build_s")) + ("" if not b else " (keyword indexes)" if b.get("keyword") else
                                                     " (%d embedded, %d cached)" % (b.get("embed_misses", 0),
                                                                                    b.get("embed_hits", 0))),
                     L.f2(r.get("open_s"), 1, " s"), L.f2(r.get("update_s"), 2, " s"),
                     L.f2(r.get("rss_open_mb"), 0), L.f2(r.get("peak_rss_mb"), 0),
                     L.f2(r.get("qdrant_rss_mb"), 0), L.f2(r.get("disk_mb"), 0)])
    w(L.md_table(["method", "scale", "tokens k=3", "k=5", "k=8", "engine s p50/p95", "rbx_vault s p50/p95 (k=5)",
                  "over 3 s", "cold build", "open (warm)", "one-note update", "RSS open MB", "peak RSS MB",
                  "Qdrant RSS MB", "disk MB"], rows) if rows else "(none)")
    w("")
    w("Tokens: rbx_vault's search text as an agent reads it (about 4 characters a token). Cold build: from no index "
      "(vectors already in the exam's vector cache are not embedded again; the counts say how many were). "
      "Open: an existing index loaded and checked. Over 3 s: searches the app would have answered with the keyword "
      "search instead (its timeout).")
    w("")
    rows = []
    for (m, sc), r in res.items():
        ta = r.get("text_app_timeout")
        if not ta:
            continue
        fell = sum(1 for x in ta if x.get("via") != "rag")
        e2e = [x["e2e_secs"] for x in ta]
        rows.append([m, "%dx" % sc, len(ta), "%d (%s)" % (fell, L.pc(fell / len(ta))),
                     "%s / %s" % (L.f2(L.pct(e2e, 50), 2), L.f2(L.pct(e2e, 95), 2))])
    if rows:
        w("Silent fallback: the same searches (k=5) with rbx_vault's real 3 s timeout. A search that gets no index "
          "answer in time is answered by the keyword search, and the agent is not told.")
        w("")
        w(L.md_table(["method", "scale", "searches", "fell back to keyword", "rbx_vault s p50 / p95"], rows))
        w("")
    w("### Telling \"not in the knowledge base\" apart")
    w("")
    w("Each answerable question's and each negative question's best score, and the threshold that separates them "
      "best (balanced accuracy: the mean of answerable kept and negatives refused). Negatives: %d." %
      sum(1 for q in ex.rq if q["set"] == "negative"))
    w("")
    rows = []
    for (m, sc), r in res.items():
        for label, b in thresholds(r["rows"]["app"], qs).items():
            rows.append([m, "%dx" % sc, label, L.f2(b["threshold"], 3), L.pc(b["balanced"]), L.pc(b["kept"]),
                         L.pc(b["refused"])])
    w(L.md_table(["method", "scale", "signal", "threshold", "balanced", "answerable kept", "negatives refused"], rows)
      if rows else "(none)")
    w("")

    # ---------------------------------------------------------- ablations
    w("## 2. Ablations")
    w("")
    w("Each row switches one piece off (or changes it) and compares with the app as shipped on the same index and "
      "questions. Deltas in percentage points over the answerable questions; negative means the piece helps.")
    w("")
    rows = []
    for m, sc in p["retrieval"]:
        r = ex.cache.get("ablations", ak, "%s-%dx-query" % (m, sc))
        if not r:
            continue
        base = set_table(r["rows"]["app"], qs, [])["answerable"]
        for v, vr in r["rows"].items():
            if v == "app":
                continue
            t = set_table(vr, qs, [])["answerable"]
            rows.append([m, "%dx" % sc, v, L.pc(t["hit@1"]), L.pc(t["hit@5"]), L.f2(t["mrr"]),
                         L.delta(base["hit@1"], t["hit@1"]), L.delta(base["hit@5"], t["hit@5"]),
                         L.delta(base["mrr"], t["mrr"])])
    for m, sc, v in p["ablation_index"]:
        r = ex.cache.get("ablations", ak, "%s-%dx-%s" % (m, sc, v))
        b = res.get((m, sc))
        if not r or not b:
            continue
        base = set_table(b["rows"]["app"], qs, [])["answerable"]
        t = set_table(r["rows"]["app"], qs, [])["answerable"]
        rows.append([m, "%dx" % sc, "index: " + {"noctx": "no context line", "nogist": "no gist"}.get(v, v),
                     L.pc(t["hit@1"]), L.pc(t["hit@5"]), L.f2(t["mrr"]), L.delta(base["hit@1"], t["hit@1"]),
                     L.delta(base["hit@5"], t["hit@5"]), L.delta(base["mrr"], t["mrr"])])
    w(L.md_table(["method", "scale", "variant", "hit@1", "hit@5", "MRR", "Δ hit@1", "Δ hit@5", "Δ MRR (×100)"],
                 rows) if rows else "(none)")
    w("")
    w("Variants: dense = the dense ranking alone; lexical = the keyword search's ranking alone (through the engine); "
      "sparse = bm25 sparse vectors alone; dense+sparse = the two fused without the keyword ranking; app+sparse = the "
      "app's plan with the sparse ranking added; norerank = Heavy without its reranker vote; nolinks = no [[link]] "
      "following. Index variants rebuild the index without the chunk's context line, or without the note's gist in "
      "it.")
    w("")
    w("How many results to return (rbx_vault's text at k = 3, 5, 8):")
    w("")
    rows = []
    for (m, sc), r in res.items():
        tx = r.get("text") or {}
        cells = []
        for k in (3, 5, 8):
            tr = [x for x in tx.get(str(k), []) if x["set"] in ANSWERABLE and qs[x["id"]].get("gold")]
            hit = L.mean([1.0 if L.first_rank(x["titles"], qs[x["id"]]["gold"], k) else 0.0 for x in tr])
            cells.append("%s, %s tok" % (L.pc(hit), L.f2(L.mean([x["tokens"] for x in tr]), 0)))
        rows.append([m, "%dx" % sc] + cells)
    w(L.md_table(["method", "scale", "k=3: hit, tokens", "k=5", "k=8"], rows) if rows else "(none)")
    w("")

    # ---------------------------------------------------------- answers
    w("## 3. Answers")
    w("")
    vk = ex.key("judge")
    verdicts = ex.cache.records("judge", vk, "verdicts")
    sets_ = ex.answer_sets()
    pas = {}
    for name, _ in sets_:
        parts = name.split("-")
        suffix = "-k5" if name.endswith("-k5") else ""
        pas[name] = ex.cache.get("answers", ex.key("answers"), "passages-%s-%s%s" % (parts[1], parts[2], suffix)) or {}
    n_ent = len(ex.entries)
    kinds = {}
    for e in ex.entries:
        kinds[e["expect"]] = kinds.get(e["expect"], 0) + 1
    w("%d questions (%s), each with 2-4 key facts and some wrong facts. Librarian: the app's own answer path (split, "
      "writer, quote and support checks) through `fridae --librarian-exam`, the writer on the user's OpenCode plan. "
      "Reader: Claude Sonnet with the same passages, no tools, citing passages. Judge: Claude Opus, 8 answers a "
      "call, strict fixed prompt; its raw replies are kept. Correct: every key fact and no wrong one (negatives: "
      "says it is not in the knowledge base; guides: points at the right guide)." % (
          n_ent, ", ".join("%d %s" % (v, k) for k, v in sorted(kinds.items()))))
    w("")
    rows, summaries, fails = [], {}, {}
    for name, recs in sets_:
        sr = answer_scores(ex, name, recs, verdicts)
        s = answer_summary(sr)
        summaries[name] = s
        fails[name] = failures(ex, name, sr, pas[name])
        s["copy_only"] = copy_only(ex, name, sr, pas[name])
        rows.append([name, "%d/%d/%d" % (s["n"], s["answered"], s["judged"]), L.pc(s["correct"]), L.pc(s["recall"]),
                     L.pc(s["wrong"]), "%d/%d" % s["neg_abstained"], L.pc(s["false_not_found"]),
                     "%d/%d" % s["guide_ok"], "%d/%d" % s["stale_old_fact"], L.f2(s["in"], 0), L.f2(s["out"], 0),
                     L.f2(s["reasoning"], 0), L.f2(s["calls"], 1),
                     "%s / %s" % (L.f2(s["p50"], 1), L.f2(s["p95"], 1)), L.f2(s["first_token_p50"], 1),
                     s["format_fail"] + s["errors"], s["copy_only"]])
    w(L.md_table(["config", "asked/answered/judged", "correct", "fact recall", "wrong facts", "negatives abstained",
                  "false not-found", "guides right", "stale old fact", "tokens in", "out", "reasoning", "calls",
                  "secs p50 / p95", "first token s", "format fails / errors", "from a copy only"], rows)
      if rows else "(none)")
    w("")
    w("Wrong facts: share of answers (negatives aside) that assert a wrong fact or contradict a key fact. Tokens: per "
      "answer, all model calls (the Librarian's writer and support check; the reader's one call). First token: the "
      "Librarian writer's first streamed token (reasoning included). From a copy only: the passages held a "
      "distractor made from the gold note but not the note itself (the answer can still be right).")
    w("")
    eff = [n for n, _ in sets_ if n.endswith("-low")]
    if eff:
        low = eff[0]
        dflt = low[:-len("-low")] + "-default"
        if dflt in summaries:
            a1, b1 = summaries[dflt], summaries[low]
            w("### Writer effort: DeepSeek Flash at its default and at its lowest (\"low\") effort")
            w("")
            w(L.md_table(["effort", "correct", "fact recall", "wrong facts", "reasoning tokens", "first token s p50",
                          "secs p50 / p95"],
                         [[x, L.pc(s["correct"]), L.pc(s["recall"]), L.pc(s["wrong"]), L.f2(s["reasoning"], 0),
                           L.f2(s["first_token_p50"], 1), "%s / %s" % (L.f2(s["p50"], 1), L.f2(s["p95"], 1))]
                          for x, s in (("default", a1), ("low", b1))]))
            w("")
    pairs = [(a, b, why) for a, b, why in (
        ("librarian-light-1x-default", "librarian-light-1x-rerun", "the same Librarian run twice (the writer's own noise)"),
        ("librarian-light-1x-default", "librarian-light-1x-k5", "the Librarian with 5 passages a part instead of 3"),
        ("reader-light-1x", "reader-light-1x-k5", "Sonnet with 5 passages a part instead of 3")) if a in summaries and b in summaries]
    if pairs:
        w("### Same questions, one thing changed")
        w("")
        rows = []
        for a, b, why in pairs:
            for name in (a, b):
                s = summaries[name]
                three = [x for x in answer_scores(ex, name, dict(sets_)[name], verdicts)
                         if x[3] and x[0]["expect"] == "answer" and len(x[0]["facts"]) >= 3]
                rows.append([why if name == a else "", name, L.pc(s["correct"]), L.pc(s["recall"]), L.pc(s["wrong"]),
                             "%d/%d" % s["neg_abstained"], L.pc(s["false_not_found"]),
                             "%d/%d" % (sum(1 for x in three if x[3]["correct"]), len(three)), L.f2(s["in"], 0)])
        w(L.md_table(["comparison", "config", "correct", "fact recall", "wrong facts", "negatives abstained",
                      "false not-found", "3-fact questions right", "tokens in"], rows))
        w("")
    # spot-check sheet
    judged = [(k, v) for k, v in verdicts.items() if v.get("verdict")]
    random.Random(5).shuffle(judged)
    if judged:
        w("### Judge spot-check sheet")
        w("")
        w("Ten verdicts picked at random, for a person to check against the answers (raw replies in the judge "
          "stage's raw folder).")
        w("")
        by = {e["id"]: e for e in ex.entries}
        allrecs = dict(sets_)
        for k, v in judged[:10]:
            name, qid = k.split("|", 1)
            e = by.get(qid)
            rec = (allrecs.get(name) or {}).get(qid) or {}
            if not e:
                continue
            vd = v["verdict"]
            w("- **%s** (%s, expect %s): facts %s; wrong %s; abstained %s; guide %s. Answer: %s" % (
                qid, name, e["expect"], ", ".join("%s=%s" % (f[:40], s) for f, s in zip(e["facts"], vd["facts"])) or "–",
                vd["wrong"], vd["abstained"], vd["guide_pointer"], _clip(rec.get("answer"), 300)))
        w("")

    # ---------------------------------------------------------- failures
    w("## 4. Why answers failed")
    w("")
    w("Each answer that is not correct gets the first cause that explains it: format failure (the writer gave no "
      "usable answer), guide handling, distractor or stale won (a wrong or old fact, or an answer to a negative), "
      "retrieval miss (no gold note among the passages), check dropped a true claim (the writer had it, a check "
      "removed it), writer omission (the gold passage was there, the fact was not used).")
    w("")
    rows = []
    for name in fails:
        rows.append([name] + [len(fails[name].get(c, [])) for c in L.FAILURES])
    w(L.md_table(["config"] + list(L.FAILURES), rows) if rows else "(none)")
    w("")
    for c in L.FAILURES:
        ex_ = [(n, x) for n in fails for x in fails[n].get(c, [])]
        if not ex_:
            continue
        w("**%s** (examples):" % c)
        w("")
        for n, (e, rec, v) in ex_[:3]:
            miss = [f for f, s in zip(e["facts"], v["facts"]) if s != "present"]
            w("- %s, %s: \"%s\" Missing: %s. Answer: %s" % (e["id"], n, _clip(e["question"], 120),
                                                            "; ".join(_clip(f, 80) for f in miss) or "–",
                                                            _clip(rec.get("answer"), 220)))
        w("")

    conflicts = vault_conflicts(ex.exam)
    if conflicts:
        w("### The vault disagrees with itself")
        w("")
        w("Answer keys where two notes in the vault give different answers. Either note's claim is graded as "
          "correct; these are findings for vault upkeep, not exam noise.")
        w("")
        for qid, text in conflicts:
            w("- **%s**: %s" % (qid, text))
        w("")

    # ---------------------------------------------------------- verdicts in plain words
    w("## 5. What makes it good, what makes it bad")
    w("")
    for line in plain_findings(ex, res, summaries, fails):
        w("- " + line)
    w("")
    w("### What these numbers show and what they do not")
    w("")
    w("- They show how often each method puts the right note in front of the model, on %d retrieval questions over "
      "a vault padded with look-alike distractors, and how good the answers are on %d questions graded by one strong "
      "judge." % (len(ex.rq), n_ent))
    w("- Sets are small (3-15 questions each outside paraphrase): a difference of one or two questions is noise. "
      "Treat gaps under about 10 points on a small set as a tie.")
    w("- The distractors are generated from the vault's own notes (split, summarised, reworded, mixed); real vault "
      "growth may be easier or harder to search.")
    w("- Latency was measured on this Mac while other work may have been running (load averages are saved with each "
      "build). The model calls' times depend on the providers' load at the time.")
    w("- The judge is a model: its verdicts are kept raw, and a spot-check sheet is above. Fact matching is strict "
      "on numbers and names.")
    w("- The failure causes are assigned by rules from what was kept (passages, the writer's raw claims, the checks); "
      "an answer has one cause even when several played a part.")
    w("")
    return "\n".join(out)


def other_commits(ex, commit):
    """Commits other than this run's that measured results the report uses (a resumed run can mix them)."""
    import glob
    import json
    out = set()
    dirs = [os.path.join(ex.cache.root, "%s-%s" % (st, ex.key(st))) for st in ("retrieval", "ablations", "answers",
                                                                               "judge")]
    for f in [f for d in dirs for f in glob.glob(os.path.join(d, "*.json")) + glob.glob(os.path.join(d, "*.jsonl"))]:
        try:
            with open(f, encoding="utf-8") as fh:
                for line in (fh if f.endswith(".jsonl") else [fh.read()]):
                    m = json.loads(line).get("_measured") if line.strip() else None
                    c = m.get("commit") if isinstance(m, dict) else m
                    if c and c != commit:
                        out.add(c)
        except (OSError, ValueError, AttributeError):
            continue
    return out


def _clip(t, n):
    t = " ".join((t or "").split())
    return t if len(t) <= n else t[:n - 1] + "…"


def plain_findings(ex, res, summaries, fails):
    out = []
    qs = ex.qs
    tops = {}
    for (m, sc), r in res.items():
        t = set_table(r["rows"]["app"], qs, [])
        if "answerable" in t:
            tops[(m, sc)] = t["answerable"]
    if tops:
        sc = max(s for _, s in tops)
        at = sorted(((v["hit@5"], m) for (m, s), v in tops.items() if s == sc), reverse=True)
        if at:
            out.append("At %dx, the right note is in the first five results for %s." % (
                sc, ", ".join("%s %s" % (m, L.pc(h)) for h, m in at)))
        for m in {m for m, _ in tops}:
            ss = sorted(s for mm, s in tops if mm == m)
            if len(ss) > 1:
                a, b = tops[(m, ss[0])]["hit@5"], tops[(m, ss[-1])]["hit@5"]
                out.append("%s from %dx to %dx: hit@5 %s → %s (%s points)." % (m, ss[0], ss[-1], L.pc(a), L.pc(b),
                                                                               L.delta(a, b)))
    for (m, sc), r in res.items():
        b = thresholds(r["rows"]["app"], qs)
        if b:
            label, best = max(b.items(), key=lambda kv: kv[1]["balanced"])
            out.append("%s %dx can tell \"not in the knowledge base\" best by its %s (%s balanced: keeps %s of "
                       "answerable, refuses %s of negatives)." % (m, sc, label, L.pc(best["balanced"]),
                                                                    L.pc(best["kept"]), L.pc(best["refused"])))
    for (m, sc), r in res.items():
        st = stale(r["rows"]["app"], qs)
        if len(st) == 2 and sc == max(s for _, s in res):
            a, b = st["marked"], st["text only"]
            out.append("%s %dx, stale pairs: the newer note ranks above the outdated one %s of the time when it "
                       "carries `supersedes:`, %s when only its text says so; the outdated note is still in the "
                       "top 5 for %s / %s." % (m, sc, L.pc(a["new_above_old"]), L.pc(b["new_above_old"]),
                                                L.pc(a["old@5"]), L.pc(b["old@5"])))
    abl = []
    ak = ex.key("ablations")
    for m, sc in ex.plan()["retrieval"]:
        r = ex.cache.get("ablations", ak, "%s-%dx-query" % (m, sc))
        if not r:
            continue
        base = set_table(r["rows"]["app"], qs, [])["answerable"]["hit@5"]
        for v, vr in r["rows"].items():
            if v != "app":
                abl.append((set_table(vr, qs, [])["answerable"]["hit@5"] - base, m, sc, v))
    if abl:
        worst = min(abl)
        best = max(abl)
        if worst[0] < 0:
            out.append("The change that hurts most against the app as shipped: %s for %s %dx (%s points of hit@5)." % (
                worst[3], worst[1], worst[2], L.delta(0, worst[0])))
        if best[0] > 0:
            out.append("A change that beats the app as shipped: %s for %s %dx (%s points of hit@5)." % (
                best[3], best[1], best[2], L.delta(0, best[0])))
    if summaries:
        ranked = sorted(((s["correct"] or 0, n) for n, s in summaries.items()), reverse=True)
        out.append("Answers correct: %s." % ", ".join("%s %s" % (n, L.pc(c)) for c, n in ranked))
        wr = [(s["wrong"], n) for n, s in summaries.items() if s["wrong"] is not None]
        if wr:
            hi = max(wr)
            out.append("Most wrong facts: %s (%s of its answers)." % (hi[1], L.pc(hi[0])))
    total = {}
    for n, f in fails.items():
        for c, xs in f.items():
            total[c] = total.get(c, 0) + len(xs)
    if total:
        c, k = max(total.items(), key=lambda kv: kv[1])
        out.append("The most common reason an answer failed: %s (%d of %d failures)." % (c, k, sum(total.values())))
    return out or ["(no results yet)"]
