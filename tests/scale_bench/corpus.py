"""The bench corpora, made offline and deterministically from the real text in the repo.

1x is what the knowledge index holds today without user notes: the vault notes (tests.guides.GUIDES, with SYMPTOMS.md; the
API snapshot the keyword search reads is the app's own, vault/Fridae/Data), every guide rbx_guide serves and the team's expert files. 10x adds
rule-made distractor notes to the vault until the estimated chunk count is about ten times 1x. The distractors are
near-topic: real sections split out, summarised, reworded and recombined, placed in the same topic folders, so many
notes share the same words. Gold notes (any note a question names as an answer) are never used as body text, so the
question's gold files stay the only true answers; "echo" notes copy a gold note's title words and headings over
unrelated bodies from the same folder, which is what a bigger vault looks like to a keyword search.

Quick corpora (--quick) start from a subset: the gold files of the paraphrase and exact sets plus a fixed sample of
the rest, and grow it to 10x the same way.

Files are written with one fixed mtime, so a rebuilt corpus has the same (mtime, size) as before and a cached index
still matches it.
"""
import hashlib
import json
import math
import os
import random
import re
import shutil

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FIXED_MTIME = 1_750_000_000
GEN_VERSION = 5  # bump when the generator's output changes
SECTION_CAP = 1200  # bytes of body per distractor section: one chunk each
QUICK_SAMPLE = 0.03  # quick corpus: this share of the non-gold vault notes and guides

_FENCE = re.compile(r"^\s{0,3}(```+|~~~+)")
_HEAD = re.compile(r"^(#{2,3})\s+(.+?)\s*#*\s*$")
_NAV = re.compile(r"^(\d+\.\s*)?(related|see also|connected notes|files|sources|references|further reading)\b", re.I)


# ------------------------------------------------------------------ reading the real notes
def parse_note(text):
    """{"tags", "tldr", "sections": [(heading, body)]} with the TL;DR, related and auto-link blocks taken out."""
    tags = []
    m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    if m:
        t = re.search(r"^tags:\s*\[(.*?)\]", m.group(1), re.M)
        tags = [x.strip().strip("'\"") for x in t.group(1).split(",")] if t else []
        text = text[m.end():]
    tl = re.search(r"<!-- tldr:start -->\n(.*?)<!-- tldr:end -->", text, re.S)
    tldr = []
    if tl:
        tldr = [ln.lstrip("> ").strip() for ln in tl.group(1).splitlines() if ln.lstrip("> ").startswith("- ")]
        text = text[:tl.start()] + text[tl.end():]
    text = re.sub(r"<!-- (related|auto-links):start -->.*?<!-- (related|auto-links):end -->", "", text, flags=re.S)
    text = re.sub(r"^# .*\n", "", text, count=1, flags=re.M)
    secs, head, buf, fence = [], "", [], False
    for line in text.split("\n"):
        if _FENCE.match(line):
            fence = not fence
        h = None if fence else _HEAD.match(line)
        if h:
            secs.append((head, "\n".join(buf).strip()))
            head, buf = h.group(2).strip(), []
        else:
            buf.append(line)
    secs.append((head, "\n".join(buf).strip()))
    secs = [(h, b) for h, b in secs if b and not _NAV.match(h) and b.strip("-\n ")]
    return {"tags": tags, "tldr": tldr, "sections": secs}


def est_chunks(text):
    """A cheap stand-in for the engine's chunk count of one file: one per section with text, more for long ones."""
    n = 0
    for _, body in parse_note(text)["sections"]:
        n += max(1, math.ceil(len(body.encode("utf-8")) / 1500))
    return max(1, n)


def _cap(body, cap=SECTION_CAP):
    if len(body.encode("utf-8")) <= cap:
        return body
    out = ""
    for para in body.split("\n\n"):
        if len((out + "\n\n" + para).encode("utf-8")) > cap:
            break
        out = (out + "\n\n" + para) if out else para
    if not out:
        out = body.encode("utf-8")[:cap].decode("utf-8", "ignore")
        out = out[:out.rfind(" ")] if " " in out else out
    if out.count("```") % 2:
        out += "\n```"
    return out.strip()


# ------------------------------------------------------------------ rule-based rewriting
SYN = [("make sure", "check"), ("players", "users"), ("player", "user"), ("use", "rely on"), ("avoid", "steer clear of"),
       ("important", "key"), ("quickly", "fast"), ("should", "ought to"), ("because", "since"), ("however", "but"),
       ("usually", "most of the time"), ("easy", "simple"), ("large", "big"), ("small", "little"), ("start", "begin"),
       ("fix", "repair"), ("problem", "issue"), ("build", "put together"), ("show", "display"), ("need", "require"),
       ("good", "solid"), ("bad", "poor"), ("often", "frequently"), ("first", "initial"), ("change", "edit"),
       ("example", "case"), ("rule", "guideline"), ("keep", "hold"), ("game", "experience"), ("works", "functions")]
_SYN_RE = re.compile(r"\b(%s)\b" % "|".join(re.escape(a) for a, _ in SYN), re.I)
_SYN_MAP = {a: b for a, b in SYN}
HEAD_SWAPS = [("Common mistakes", "Mistakes people make"), ("Checklist", "Before shipping"), ("Why", "The reason"),
              ("How to", "Doing it:"), ("Overview", "The gist"), ("Setup", "Getting it set up")]


def reword(text, rng):
    """Synonyms swapped and, in prose paragraphs, the sentences rotated. Code, tables and lists keep their order."""
    def swap(m):
        w = m.group(0)
        r = _SYN_MAP[w.lower()]
        return r[0].upper() + r[1:] if w[0].isupper() else r
    out, fence = [], False
    for para in text.split("\n\n"):
        if para.lstrip().startswith(("```", "~~~")) or fence:
            fence = (para.count("```") + para.count("~~~")) % 2 == 1 if not fence else \
                (para.count("```") + para.count("~~~")) % 2 == 0
            out.append(para)
            continue
        p = _SYN_RE.sub(swap, para)
        if not p.lstrip().startswith(("|", "-", "*", ">", "1.")):
            sents = re.split(r"(?<=[.!?])\s+", p)
            if len(sents) > 2:
                k = rng.randrange(1, len(sents))
                p = " ".join(sents[k:] + sents[:k])
        out.append(p)
    return "\n\n".join(out)


def summarise(text):
    """First sentence of each paragraph, the first three items of each list, no code, a table's first two rows."""
    out = []
    for para in re.sub(r"```.*?```", "", text, flags=re.S).split("\n\n"):
        p = para.strip()
        if not p:
            continue
        lines = p.split("\n")
        if lines[0].startswith("|"):
            out.append("\n".join(lines[:4]))
        elif lines[0].lstrip().startswith(("- ", "* ", "1.")):
            out.append("\n".join(lines[:3]))
        else:
            out.append(re.split(r"(?<=[.!?])\s+", " ".join(lines))[0])
    return "\n\n".join(out[:6])


def reheading(h, rng):
    for a, b in HEAD_SWAPS:
        if h.startswith(a):
            return b + h[len(a):]
    h = re.sub(r"^\d+\.\s*", "", h)
    return rng.choice(["", "", "Notes: ", "On ", "About "]) + h


INTROS = ["Notes I put together while working on {t}.", "Collected over a few sessions on {t}; not all of it is tested.",
          "Scratch notes on {t}, kept for later.", "What I found out about {t}, in my own words.",
          "A short write-up on {t} from a recent build.", "Bits and pieces on {t} that came up during playtests.",
          "Copied from my project log on {t}.", "Reference notes on {t}; check the details before relying on them."]
TITLES_SPLIT = ["{t} - {h}", "{h} ({t})", "{h} - notes", "On {h}"]
TITLES_SUM = ["{t} - short version", "Cheat sheet - {t}", "{t} (summary)", "Quick notes - {t}", "{t} in brief"]
TITLES_REWORD = ["{t} - my take", "Notes on {t}", "{t} revisited", "{t} - second pass", "Working notes - {t}"]
TITLES_MIX = ["{f}: {h1} and {h2}", "{h1}, {h2} - field notes", "{f} scratchpad - {h1}", "{h1} and other {f} notes",
              "Session log - {h1}"]
TITLES_ECHO = ["{t} - open questions", "More on {w}", "{t} checklist (draft)", "{w} - ideas", "Thoughts on {w}",
               "{t} - todo"]


def _topic(folder):
    return re.sub(r"^\d+\s+", "", folder).strip() or "the vault"


def _short(h, n=6):
    h = re.sub(r"^\d+\.\s*", "", h).strip(" .:")
    return " ".join(h.split()[:n]) or "notes"


def render(title, tags, tldr, intro, sections):
    lines = ["---", "tags: [%s]" % ", ".join(tags), "---", "# %s" % title, ""]
    if tldr:
        lines += ["<!-- tldr:start -->", "> [!tldr] TL;DR"] + ["> " + b for b in tldr] + ["<!-- tldr:end -->", ""]
    lines += [intro, ""]
    for h, b in sections:
        lines += ["## %s" % h, "", b, ""]
    return "\n".join(lines).rstrip() + "\n"


# ------------------------------------------------------------------ the corpora
def base_files():
    """(vault notes [(rel path, abs path)], guides [(title, abs path)], team [(name, abs path)]) from the repo."""
    import shim  # noqa: F401  (puts the repo on sys.path)
    import rbxvault
    from tests import guides
    if guides.GUIDES is None:
        raise SystemExit(guides.SKIP)
    kb = guides.GUIDES
    notes = []
    for root, dirs, fs in os.walk(kb):
        dirs[:] = sorted(d for d in dirs if not d.startswith("."))
        for f in sorted(fs):
            p = os.path.join(root, f)
            notes.append((os.path.relpath(p, kb), p))
    guides = rbxvault.guide_files()
    team = os.path.join(REPO, "vault", "Fridae", "Team")
    team = [(f, os.path.join(team, f)) for f in sorted(os.listdir(team)) if not f.startswith(".")]
    return notes, guides, team


def gold_titles(questions, sets):
    out = set()
    for s in sets:
        for q in questions.get(s) or []:
            out |= set(q.get("gold") or [])
            out |= set(q.get("pair") or [])
            if q.get("old"):
                out.add(q["old"])
    return out


GOLD_SETS = ["paraphrase", "exact", "old33"]
EXAM_SETS = ["typo", "multihop", "stale", "guide"]  # exam sets whose gold must be in every corpus (scope: game notes)


def stale_note(spec, marked):
    """The newer note of a stale pair (exam_questions.json "stale"): dated, with its changed fact. marked: it says so
    in its front matter (`supersedes: "Old Title"`, as vault upkeep marks a replaced note); otherwise only its text
    says it replaces the older note."""
    new = spec["new"]
    fm = ["---", "tags: [update]", "date: %s" % new["date"]]
    if marked:
        fm.append('supersedes: "%s"' % spec["old"])
    fm.append("---")
    body = new["body"]
    if not marked:
        body = "This note replaces the older note [[%s]] (dated %s). %s" % (spec["old"], new["date"], body)
    return "\n".join(fm) + "\n# %s\n\n%s\n" % (new["title"], body)


def stale_marked(i):
    """Half the stale pairs carry the front-matter mark (the odd ones, S01, S03, ...), half only the text."""
    return i % 2 == 0


def mark_superseded(path, new_title):
    """Add `superseded_by: "New Title"` to a note's front matter (making one when it has none)."""
    with open(path, encoding="utf-8") as f:
        text = f.read()
    line = 'superseded_by: "%s"\n' % new_title
    if text.startswith("---\n") and "\n---" in text[4:]:
        text = "---\n" + line + text[4:]
    else:
        text = "---\n" + line + "---\n" + text
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


def write_exam_extras(root, exam):
    """The exam's additions to a corpus: the stale pairs' newer notes in the vault, and the game project (game/: its
    GAME.md and .fridae/notes). Returns their estimated chunks."""
    est = 0
    olds = {}
    for dirpath, _, fs in os.walk(os.path.join(root, "vault")):
        for f in fs:
            if f.endswith(".md"):
                olds.setdefault(f[:-3], os.path.join(dirpath, f))
    for i, spec in enumerate(exam.get("stale") or []):
        if stale_marked(i) and spec["old"] in olds:  # as vault upkeep leaves the pair: the old note points onward
            mark_superseded(olds[spec["old"]], spec["new"]["title"])
        folder = os.path.join(root, "vault", spec["new"]["folder"])
        os.makedirs(folder, exist_ok=True)
        text = stale_note(spec, stale_marked(i))
        with open(os.path.join(folder, spec["new"]["title"] + ".md"), "w", encoding="utf-8") as f:
            f.write(text)
        est += est_chunks(text)
    game = exam.get("game")
    if game:
        g = os.path.join(root, "game")
        os.makedirs(os.path.join(g, ".fridae", "notes"), exist_ok=True)
        with open(os.path.join(g, "GAME.md"), "w", encoding="utf-8") as f:
            f.write(game["GAME.md"])
        for name, text in sorted(game.get("notes", {}).items()):
            with open(os.path.join(g, ".fridae", "notes", name), "w", encoding="utf-8") as f:
                f.write(text)
    return est


def _is_gold(title, gold):
    return title in gold or any(title.endswith("/" + g) for g in gold)


def input_hash(questions, scale, quick, exam=None):
    """What decides a corpus: the generator, the gold titles, the repo's text and the parameters."""
    notes, guides, team = base_files()
    h = hashlib.sha256(("%s|%s|%s|%s" % (GEN_VERSION, scale, quick, sorted(gold_titles(
        questions, questions.keys() - {"_about"})))).encode())
    if exam:
        h.update(json.dumps(exam, sort_keys=True).encode())
    with open(os.path.abspath(__file__), "rb") as f:
        h.update(f.read())
    for name, p in notes + guides + team:
        with open(p, "rb") as f:
            h.update(name.encode() + b"\0" + hashlib.sha1(f.read()).digest())
    return h.hexdigest()[:16]


def build(out_dir, questions, scale=10, quick=False, seed=7, log=print, exam=None):
    """Write the corpus to out_dir (vault/, guides/, guides.json, team/, corpus.json; with exam: the stale pairs'
    newer notes and game/). Returns its summary."""
    rng = random.Random("%s|%s|%s" % (seed, scale, quick))
    sample = random.Random("sample|%s" % seed)  # the quick subset is the same at every scale
    notes, guides, team = base_files()
    sets = ["paraphrase", "exact"] if quick else ["paraphrase", "exact", "old33"]
    gold = gold_titles(questions, sets) | (gold_titles(exam, EXAM_SETS) if exam else set())
    all_gold = gold_titles(questions, GOLD_SETS) | (gold_titles(exam, EXAM_SETS) if exam else set())
    if exam:  # the scope questions' general notes are in every corpus too
        gold |= {g for q in exam.get("scope") or [] for g in q.get("general") or []}
    if exam:  # the stale pairs' newer notes are written by write_exam_extras, not looked for in the repo
        gold -= {s["new"]["title"] for s in exam.get("stale") or []}
    tmp = out_dir + ".tmp"
    shutil.rmtree(tmp, ignore_errors=True)
    os.makedirs(tmp)
    vault = os.path.join(tmp, "vault")

    def keep_note(rel):
        title = os.path.basename(rel)[:-3]
        if not rel.endswith(".md") or os.path.dirname(rel) == "" or rel.startswith("_"):
            return True  # SYMPTOMS.md, the API snapshot, start pages at the root
        if not quick:
            return True
        return _is_gold(title, gold) or sample.random() < QUICK_SAMPLE

    base_notes = [(r, p) for r, p in notes if keep_note(r)]
    base_guides = [(t, p) for t, p in guides if not quick or _is_gold(t, gold) or sample.random() < QUICK_SAMPLE]
    # copy the base
    titles = set()
    base_est = 0
    pool = {}  # folder -> [(title, parsed)] of non-gold vault notes: the distractors' material
    gold_notes = {}  # folder -> [(title, parsed)]
    for rel, p in base_notes:
        dst = os.path.join(vault, rel)
        os.makedirs(os.path.dirname(dst), exist_ok=True)
        shutil.copyfile(p, dst)
        if not rel.endswith(".md"):
            continue
        title = os.path.basename(rel)[:-3]
        titles.add(title.casefold())
        if os.path.dirname(rel) in ("", "."):
            continue
        with open(p, encoding="utf-8") as f:
            text = f.read()
        base_est += est_chunks(text)
        folder = os.path.dirname(rel)
        parsed = parse_note(text)
        if not parsed["sections"]:
            continue
        (gold_notes if _is_gold(title, all_gold) else pool).setdefault(folder, []).append((title, parsed))
    gdir = os.path.join(tmp, "guides")
    os.makedirs(gdir)
    glist = []
    for i, (t, p) in enumerate(base_guides):
        name = "%03d-%s.md" % (i, re.sub(r"[^A-Za-z0-9_.-]+", "-", t))
        shutil.copyfile(p, os.path.join(gdir, name))
        glist.append((t, name))
        titles.add(t.casefold())
        with open(p, encoding="utf-8") as f:
            base_est += est_chunks(f.read())
    with open(os.path.join(tmp, "guides.json"), "w", encoding="utf-8") as f:
        json.dump(glist, f, indent=0)
    tdir = os.path.join(tmp, "team")
    os.makedirs(tdir)
    for name, p in team:
        shutil.copyfile(p, os.path.join(tdir, name))
        if name.endswith(".md"):
            with open(p, encoding="utf-8") as f:
                base_est += est_chunks(f.read())
    missing = sorted(g for g in gold if g.casefold() not in titles and not any(
        t.endswith("/" + g.casefold()) for t in titles))
    if missing:
        raise SystemExit("gold titles not in the corpus: %s" % ", ".join(missing))

    # distractors
    target = base_est * scale
    est = base_est
    folders = sorted(pool)
    weights = [len(pool[f]) for f in folders]
    made = {"split": 0, "summary": 0, "reword": 0, "mix": 0, "echo": 0}

    def unique(title):
        title = re.sub(r"[/\\:*?\"<>|]+", "-", title).strip(" .-")[:150] or "Notes"
        t, n = title, 2
        while t.casefold() in titles or _is_gold(t, all_gold):
            t = "%s (%d)" % (title, n)
            n += 1
        titles.add(t.casefold())
        return t

    derived = {}  # distractor title -> {"kind", "from": the notes whose text it carries, "looks_like"}

    def write(folder, title, tags, tldr, intro, sections, kind=None, sources=(), looks_like=None):
        nonlocal est
        sections = [(h or "Notes", _cap(b)) for h, b in sections if b.strip()]
        if not sections:
            return
        derived[title] = {"kind": kind, "from": sorted(set(sources))}
        if looks_like:
            derived[title]["looks_like"] = looks_like
        text = render(title, tags, tldr, intro, sections)
        p = os.path.join(vault, folder, title + ".md")
        with open(p, "w", encoding="utf-8") as f:
            f.write(text)
        est += len(sections) + 1  # the opening (intro, TL;DR) is a chunk of its own

    # echo notes first: every gold vault note gets a few title-and-heading look-alikes with unrelated bodies
    if scale > 1:
        for folder in sorted(gold_notes):
            for gtitle, parsed in gold_notes[folder]:
                src = pool.get(folder) or pool[rng.choice(folders)]
                for _ in range(min(scale - 1, 4)):
                    words = " ".join(gtitle.split(" - ")[0].split()[:4])
                    title = unique(rng.choice(TITLES_ECHO).format(t=gtitle, w=words))
                    heads = [h for h, _ in parsed["sections"] if h][:4] or [words]
                    picked = rng.sample(src, min(3, len(src)))
                    bodies = [b for _, p2 in picked for _, b in p2["sections"]]
                    rng.shuffle(bodies)
                    secs = [(h, bodies[i % len(bodies)]) for i, h in enumerate(heads)] if bodies else []
                    write(folder, title, rng.sample(parsed["tags"], min(4, len(parsed["tags"]))), [],
                          rng.choice(INTROS).format(t=words.lower()), secs, "echo", [t for t, _ in picked], gtitle)
                    made["echo"] += 1
    while est < target:
        folder = rng.choices(folders, weights)[0]
        notes_here = pool[folder]
        kind = rng.choices(["split", "summary", "reword", "mix"], [25, 20, 20, 35])[0]
        topic = _topic(folder)
        title0, p0 = rng.choice(notes_here)
        intro = rng.choice(INTROS).format(t=topic.lower())
        tags = p0["tags"][:]
        rng.shuffle(tags)
        tags = tags[:5]
        if kind == "split":
            h, b = rng.choice(p0["sections"])
            title = unique(rng.choice(TITLES_SPLIT).format(t=title0, h=_short(h)))
            parts = [x for x in b.split("\n\n") if x.strip()]
            secs = [(_short(h, 8), "\n\n".join(parts[:len(parts) // 2 or 1]))]
            if len(parts) > 1:
                secs.append(("More on " + _short(h, 4).lower(), "\n\n".join(parts[len(parts) // 2:])))
            write(folder, title, tags, [], intro, secs, kind, [title0])
        elif kind == "summary":
            title = unique(rng.choice(TITLES_SUM).format(t=title0))
            n0 = rng.randrange(len(p0["sections"]))
            secs = [(reheading(h, rng), summarise(b)) for h, b in (p0["sections"][n0:] + p0["sections"][:n0])[:6]]
            write(folder, title, tags, p0["tldr"][:3], intro, secs, kind, [title0])
        elif kind == "reword":
            title = unique(rng.choice(TITLES_REWORD).format(t=title0))
            secs = [(reheading(h, rng), reword(b, rng)) for h, b in p0["sections"][:8]]
            write(folder, title, tags, [], intro, secs, kind, [title0])
        else:
            srcs = rng.sample(notes_here, min(len(notes_here), rng.randint(2, 4)))
            secs = []
            for t, p in srcs:  # each section keeps its note's title (the same draws as before)
                secs += [(h, b, t) for h, b in rng.sample(p["sections"], min(len(p["sections"]), rng.randint(1, 2)))]
            rng.shuffle(secs)
            secs = secs[:rng.randint(3, 6)]
            owners = [t for _, _, t in secs]
            secs = [(h, b) for h, b, _ in secs]
            hs = [_short(h, 5) for h, _ in secs if h] or [topic]
            title = unique(rng.choice(TITLES_MIX).format(f=topic, h1=hs[0], h2=hs[-1]))
            bullets = [b for _, p in srcs for b in p["tldr"][:1]]
            write(folder, title, tags, bullets if rng.random() < 0.4 else [], intro,
                  [(h, reword(b, rng) if rng.random() < 0.5 else b) for h, b in secs], kind, owners)
        made[kind] += 1
    extra = write_exam_extras(tmp, exam) if exam else 0
    est += extra
    # one mtime for everything, so a regenerated corpus matches a cached index
    for root, _, fs in os.walk(tmp):
        for f in fs:
            os.utime(os.path.join(root, f), (FIXED_MTIME, FIXED_MTIME))
    summary = {"scale": scale, "quick": quick, "base_notes": sum(1 for r, _ in base_notes if r.endswith(".md")),
               "base_guides": len(base_guides), "team": len(team), "base_est_chunks": base_est,
               "est_chunks": est, "distractors": made, "files": sum(len(fs) for _, _, fs in os.walk(tmp)),
               "exam_extras": bool(exam)}
    with open(os.path.join(tmp, "corpus.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=1)
    with open(os.path.join(tmp, "distractors.json"), "w", encoding="utf-8") as f:
        json.dump(derived, f, indent=0, ensure_ascii=False)
    os.utime(os.path.join(tmp, "corpus.json"), (FIXED_MTIME, FIXED_MTIME))
    shutil.rmtree(out_dir, ignore_errors=True)
    os.replace(tmp, out_dir)
    log("corpus %s: %s" % (os.path.basename(out_dir), json.dumps(summary)))
    return summary


def ensure(cache_dir, questions, scale, quick, log=print, exam=None):
    """The corpus folder for these parameters, generated when it is not there yet. Returns (path, summary)."""
    key = input_hash(questions, scale, quick, exam)
    path = os.path.join(cache_dir, "corpus", "%s-%dx-%s" % ("quick" if quick else "full", scale, key))
    meta = os.path.join(path, "corpus.json")
    if not os.path.isfile(meta):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        build(path, questions, scale, quick, log=log, exam=exam)
    with open(meta, encoding="utf-8") as f:
        return path, json.load(f)
