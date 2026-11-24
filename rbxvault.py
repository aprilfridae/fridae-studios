"""Search and read Fridae's vault (vault/): the LihFolk notes in vault/Roblox and the other knowledge shelves.

The search engine below is LihFolk's `_tools/vault.py` from LihFolk Vault V2
(https://github.com/lihfolk-cy/v2-vault), CC BY 4.0, (c) 2026 LihFolk, used with these changes: it reads the
notes from vault/Roblox (a trimmed copy), keeps its index cache in the work folder, and a read is capped.
Everything from "shelves (rbxkit)" on is rbxkit's.
"""
import hashlib
import json
import math
import os
import re
import sys
import urllib.request
from collections import Counter, defaultdict

import rbxpaths
from rbxpaths import KIT, KitError

KB = os.path.join(KIT, "vault", "Roblox")  # LihFolk Vault V2 notes, inside Fridae's one vault


def cache_path(name="lihfolk-vault-index.json"):
    """Index caches live in the work folder (looked up at call time, so tests can point it elsewhere)."""
    return os.path.join(rbxpaths.WORK, "cache", name)

START, END = "<!-- tldr:start -->", "<!-- tldr:end -->"
SKIP_ROOT = {"SYMPTOMS.md", "LICENSE.md"}  # Fridae's copy dropped INDEX, INDEX_SLIM, ROUTER and 000 Home (search replaces them)
FIELDS = {"title": 4.0, "tags": 2.5, "tldr": 2.0, "heads": 1.5, "body": 0.4, "sym": 2.0}  # sym = SYMPTOMS.md lines
K1, B = 1.2, 0.75
SYMPTOM_WEIGHT = 0.0  # extra boost from matched symptom lines; 0 = symptoms act only as the sym field
# Notes that should rank below a real explanation when both match: link lists, short glossary-style concept
# notes, hubs and visual references. (path prefix or title suffix, multiplier)
DAMP = [("46 Resource Library/", 0.5), ("57 API and Concept Atlas/", 0.5), ("52 Core Concepts/", 0.75), ("Start Here", 0.7),
        ("08 Animation/09 References/", 0.6), ("Skill - ", 0.8)]
STOP = set("""a an and are as at be but by can do does for from get got how i if in into is it its make
my me of on or so that the then there this to too up use using want way what when where which while
why will with without you your we our not no just also should would could any some all""".split())
# Small, general synonym map: everyday words -> vault vocabulary. Applied to the query only.
SYN = {
    "lag": ["performance", "fps"], "laggy": ["performance", "fps"], "stutter": ["performance", "hitch"],
    "fps": ["performance", "frame"], "slow": ["performance"], "crash": ["memory", "crash"],
    "save": ["datastore"], "saving": ["datastore"], "saves": ["datastore"], "data": ["datastore"],
    "progress": ["datastore"], "wipe": ["datastore", "reset"], "dupe": ["duplication", "exploit"],
    "buy": ["purchase"], "bought": ["purchase", "receipt"], "robux": ["purchase", "monetization"],
    "exploit": ["exploiter", "security"], "hack": ["exploit", "exploiter"], "cheat": ["exploit", "anticheat"],
    "gui": ["ui"], "ui": ["gui"], "menu": ["ui", "gui"], "button": ["ui", "button"], "hud": ["ui", "hud"],
    "phone": ["mobile"], "mobile": ["phone", "touch"], "controller": ["gamepad"], "xbox": ["gamepad", "console"],
    "sword": ["weapon", "blade", "tool"], "weapon": ["tool", "weapon"], "anim": ["animation"],
    "particles": ["particle", "vfx"], "effect": ["vfx"], "effects": ["vfx"], "sound": ["audio", "sound"],
    "music": ["audio", "sound"], "npc": ["npc", "humanoid", "enemy"], "enemy": ["npc", "enemy"],
    "map": ["level", "world"], "blender": ["blender", "export"], "model": ["mesh", "model"],
    "remote": ["remoteevent"], "remotes": ["remoteevent"], "server": ["server"], "lobby": ["hub", "lobby"],
    "shop": ["shop", "store"], "pets": ["pet"], "eggs": ["egg", "hatch"], "leaderboard": ["leaderboard", "ordereddatastore"],
    "pass": ["gamepass"], "passes": ["gamepass"], "gamepass": ["game", "pass"], "gamepasses": ["game", "pass"],
    "leave": ["playerremoving", "bindtoclose"], "leaves": ["playerremoving", "bindtoclose"], "quit": ["playerremoving"],
    "servers": ["crossserver", "messagingservice"], "touches": ["touched"], "touch": ["touched"],
    "click": ["clickdetector", "mouseclick"], "clicks": ["clickdetector", "mouseclick"],
    "filter": ["textservice", "filterstringasync"], "timer": ["countdown", "round"], "round": ["round", "intermission"],
}


def stem(w):
    for suf, rep in (("ies", "y"), ("ing", ""), ("ed", ""), ("es", ""), ("s", "")):
        if len(w) > len(suf) + 3 and w.endswith(suf):
            return w[: -len(suf)] + rep
    return w


def tokens(s):
    out = []
    for word in re.findall(r"[A-Za-z][A-Za-z0-9]*|\d+", s):
        low = word.lower()
        if low in STOP or len(low) < 2:
            continue
        out.append(stem(low))
        parts = re.findall(r"[A-Z]+(?![a-z])|[A-Z]?[a-z]+|\d+", word)
        if len(parts) > 1:  # CamelCase: UpdateAsync -> updateasync + update + async
            out += [stem(p.lower()) for p in parts if p.lower() not in STOP and len(p) > 1]
    return out


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def note_files():
    files = []
    for root, dirs, fs in os.walk(KB):
        dirs[:] = sorted(d for d in dirs if not d.startswith((".", "_")))
        rel = os.path.relpath(root, KB)
        for f in sorted(fs):
            if f.endswith(".md") and not (rel == "." and f in SKIP_ROOT):
                files.append(os.path.join(root, f))
    return files


def parse(path):
    text = read(path)
    fm = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    tags = re.search(r"^tags:\s*\[(.*?)\]", fm.group(1), re.M) if fm else None
    tags = [t.strip().strip("'\"") for t in tags.group(1).split(",")] if tags else []
    body = text[fm.end():] if fm else text
    m = re.search(re.escape(START) + r"\n(.*?)" + re.escape(END), body, re.S)
    tldr = m.group(1).strip() if m else ""
    if m:
        body = body[: m.start()] + body[m.end():]
    # generated "## Related" blocks (link_builder.py) are navigation, not content
    body = re.sub(r"<!-- related:start -->.*?<!-- related:end -->", "", body, flags=re.S)
    heads = re.findall(r"^#{1,4} +(.+)$", body, re.M)
    plain = re.sub(r"```.*?```", lambda c: c.group(0)[:600], body, flags=re.S)  # code counts, but capped
    title = os.path.basename(path)[:-3]
    rel = os.path.relpath(path, KB).replace(os.sep, "/")
    summary = ""
    if not tldr:
        for para in re.split(r"\n\s*\n", body):
            p = para.strip()
            if p and not p.startswith(("#", "|", "!", "<!--", ">", "-", "```")):
                summary = " ".join(p.split())[:300]
                break
    fmd, after = split_front(text)
    out = {"title": title, "path": rel, "tags": tags, "tldr": tldr, "summary": summary,
           "f": {"title": tokens(title), "tags": tokens(" ".join(tags)), "tldr": tokens(tldr),
                 "heads": tokens(" ".join(heads)), "body": tokens(plain)},
           "kb": round(len(text.encode("utf-8")) / 1024),
           "labels": note_labels("vault", title, path, fmd, text), "links": wikilinks(after)}
    olds, by = supersession(text)  # rbxkit: superseded notes (see SUPERSEDED_SCALE)
    if olds:
        out["supersedes"] = olds
    if by:
        out["superseded_by"] = by
    return out


def parse_symptoms():
    path = os.path.join(KB, "SYMPTOMS.md")
    if not os.path.exists(path):
        return []
    out, area = [], ""
    for line in read(path).split("\n"):
        if line.startswith("## "):
            area = line[3:].strip()
        if not line.startswith("- ") or line.count(" | ") < 3:
            continue
        parts = line[2:].split(" | ")
        links = re.findall(r"\[\[([^\]|]+)", parts[-1])
        if links:
            out.append({"line": line[2:], "area": area, "links": links,
                        "toks": tokens(" ".join(parts[:-1]) + " " + area)})
    return out


def signature(files):
    h = hashlib.sha1(rbxpaths.VERSION.encode())  # the packaged app has no rbxvault.py on disk to stat
    for p in files + [os.path.join(KB, "SYMPTOMS.md"), os.path.abspath(__file__)]:
        if os.path.exists(p):
            st = os.stat(p)
            h.update(f"{p}|{st.st_mtime_ns}|{st.st_size}".encode())
    return h.hexdigest()


def load_index():
    files = note_files()
    sig = signature(files)
    CACHE = cache_path()
    if os.path.exists(CACHE):
        try:
            idx = json.loads(read(CACHE))
            if idx.get("sig") == sig:
                return idx
        except (OSError, ValueError):
            pass
    docs = [parse(p) for p in files]
    for d in docs:  # store term counts, not token lists
        d["tf"] = {f: Counter(t) for f, t in d.pop("f").items()}
        d["len"] = {f: sum(c.values()) for f, c in d["tf"].items()}
    symptoms = parse_symptoms()
    by_title = {d["title"]: d for d in docs}
    for d in docs:
        d["tf"]["sym"], d["len"]["sym"] = Counter(), 0
    for s in symptoms:  # a symptom line is extra searchable text on the notes it links to
        for t in s["links"]:
            if t in by_title:
                by_title[t]["tf"]["sym"].update(s["toks"])
                by_title[t]["len"]["sym"] += len(s["toks"])
    idx = {"sig": sig, "docs": docs, "symptoms": symptoms}
    try:
        os.makedirs(os.path.dirname(CACHE), exist_ok=True)
        with open(CACHE, "w", encoding="utf-8") as f:
            json.dump(idx, f)
    except OSError:
        pass
    return idx


class Docs(list):
    """Index entries that keep their own BM25F statistics between searches: the field averages, and an inverted
    index (which entries hold each term) built once instead of the document frequencies being counted on every
    search. A search then scores only the entries that hold a query term, with numpy. A caller that keeps one corpus
    loaded for many searches (the knowledge index's KeywordRanker) wraps it in this; the scores are exactly the plain
    list's (the same floating-point operations in the same order). Never changed after it is made."""

    def stats(self, fields):
        import threading
        from array import array

        import numpy as np
        key = tuple(fields.items())
        st = self.__dict__.setdefault("_stats", {}).get(key)
        if st is None:
            n = len(self)
            avg = {f: sum(d["len"][f] for d in self) / n or 1 for f in fields}
            tid, terms, docs = {}, array("i"), array("i")
            for k, d in enumerate(self):
                for t in {t for f in fields for t in d["tf"][f]}:
                    i = tid.get(t)
                    if i is None:
                        i = tid[t] = len(tid)
                    terms.append(i)
                    docs.append(k)
            terms = np.frombuffer(terms, dtype=np.int32)
            order = np.argsort(terms, kind="stable")  # entries in order within a term
            counts = np.bincount(terms, minlength=len(tid)) if len(tid) else np.zeros(0, dtype=np.int64)
            starts = np.zeros(len(tid) + 1, dtype=np.int64)
            np.cumsum(counts, out=starts[1:])
            st = {"avg": avg, "tid": tid, "counts": counts, "starts": starts,
                  "docs": np.frombuffer(docs, dtype=np.int32)[order], "tf": {}, "lock": threading.Lock()}
            self._stats[key] = st
        return st

    def df(self, st, t):
        i = st["tid"].get(t)
        return 0 if i is None else int(st["counts"][i])

    def postings(self, st, fields, t):
        """(entries holding t, the BM25F term frequency of t in each), worked out on first use of t and kept."""
        import numpy as np
        hit = st["tf"].get(t)
        if hit is None:
            i = st["tid"][t]
            ids = st["docs"][st["starts"][i]:st["starts"][i + 1]]
            avg = st["avg"]
            tfw = np.array([sum(w * self[k]["tf"][f].get(t, 0) / (1 - B + B * self[k]["len"][f] / avg[f])
                                for f, w in fields.items()) for k in ids.tolist()], dtype=np.float64)
            hit = (ids, tfw)
            with st["lock"]:
                if len(st["tf"]) > 20000:
                    st["tf"].clear()
                st["tf"][t] = hit
        return hit

    def warm(self, fields, min_df=100):
        """Work out the term frequencies of every term held by at least min_df entries, most common first (the
        expensive ones when a search meets them first); for a background thread."""
        st = self.stats(fields)
        common = sorted(((int(c), t) for t, c in zip(st["tid"], st["counts"].tolist()) if c >= min_df), reverse=True)
        for _, t in common:
            self.postings(st, fields, t)

    def part(self, key, keep):
        """The entries for which keep(entry) is true, as a Docs of its own (kept, so its statistics are too)."""
        parts = self.__dict__.setdefault("_parts", {})
        if key not in parts:
            parts[key] = Docs(d for d in self if keep(d))
        return parts[key]


def _bm25f_kept(query_terms, docs, fields):
    """bm25f over a Docs: the same arithmetic, only for the entries that hold a query term (the rest score 0)."""
    import numpy as np
    n = len(docs)
    st = docs.stats(fields)
    scores = np.zeros(n, dtype=np.float64)
    for t, qw in query_terms.items():
        df = docs.df(st, t)
        if not df:
            continue
        ids, tfw = docs.postings(st, fields, t)
        c = qw * math.log(1 + (n - df + 0.5) / (df + 0.5))
        scores[ids] += c * tfw / (K1 + tfw)
    return scores.tolist()


def bm25f(query_terms, docs, fields=None):
    FIELDS_ = fields or FIELDS
    n = len(docs)
    if not n:
        return []
    if isinstance(docs, Docs):
        return _bm25f_kept(query_terms, docs, FIELDS_)
    avg = {f: sum(d["len"][f] for d in docs) / n or 1 for f in FIELDS_}
    df = Counter()
    for d in docs:
        for t in {t for f in FIELDS_ for t in d["tf"][f]}:
            df[t] += 1
    scores = []
    for d in docs:
        s = 0.0
        for t, qw in query_terms.items():
            if not df[t]:
                continue
            tf = sum(w * d["tf"][f].get(t, 0) / (1 - B + B * d["len"][f] / avg[f]) for f, w in FIELDS_.items())
            if tf:
                s += qw * math.log(1 + (n - df[t] + 0.5) / (df[t] + 0.5)) * tf / (K1 + tf)
        scores.append(s)
    return scores


def query_terms(q):
    base = tokens(q)
    terms = Counter(base)
    for w in re.findall(r"[a-z]+", q.lower()):
        for s in SYN.get(w, []):
            terms[stem(s)] += 0.4
    return terms


def search(q, k=3, idx=None):
    idx = idx or load_index()
    docs, terms = idx["docs"], query_terms(q)
    scores = bm25f(terms, docs)
    by_title = {d["title"]: i for i, d in enumerate(docs)}
    hits = []
    if idx["symptoms"]:
        sdf = Counter(t for s in idx["symptoms"] for t in set(s["toks"]))
        ns = len(idx["symptoms"])
        avg = sum(len(s["toks"]) for s in idx["symptoms"]) / ns
        for s in idx["symptoms"]:
            c = Counter(s["toks"])
            sc = sum(qw * math.log(1 + (ns - sdf[t] + 0.5) / (sdf[t] + 0.5)) * c[t] * (K1 + 1)
                     / (c[t] + K1 * (1 - B + B * len(s["toks"]) / avg)) for t, qw in terms.items() if t in c)
            if sc > 0:
                hits.append((sc, s))
        hits.sort(key=lambda x: -x[0])
        top = hits[0][0] if hits else 0
        for sc, s in hits[:8]:
            for t in s["links"]:
                if t in by_title and sc >= 0.5 * top:
                    scores[by_title[t]] += SYMPTOM_WEIGHT * sc
    for i, d in enumerate(docs):
        for prefix, mult in DAMP:
            if d["path"].startswith(prefix) or d["title"].endswith(prefix):
                scores[i] *= mult
    order = sorted(range(len(docs)), key=lambda i: -scores[i])[:k]
    best = hits[0][0] if hits else 0
    return [(scores[i], docs[i]) for i in order if scores[i] > 0], [s for sc, s in hits[:3] if sc >= 0.6 * best]


def find_note(title, idx):
    for d in idx["docs"]:
        if d["title"].lower() == title.lower():
            return d
    cands = [d for d in idx["docs"] if title.lower() in d["title"].lower()]
    return cands[0] if len(cands) == 1 else None


def print_section(title, heading, idx):
    d = find_note(title, idx)
    if not d:
        sys.exit(f"note not found (or ambiguous): {title}")
    text = read(os.path.join(KB, d["path"]))
    heads = [(m.start(), len(m.group(1)), m.group(2).strip()) for m in re.finditer(r"^(#{2,3}) +(.+)$", text, re.M)]
    def end_of(pos, level):
        nxt = re.compile(r"^#{2,%d} " % level, re.M).search(text, pos + 1)
        return nxt.start() if nxt else len(text)
    if heading is None:
        print(f"{d['title']} ({d['path']}, {d['kb']} KB)")
        for pos, lvl, h in heads:
            print("  " * (lvl - 1) + "#" * lvl + " " + h + "  (%s)" % size_text(len(text[pos:end_of(pos, lvl)].encode("utf-8"))))
        return
    hit = [h for h in heads if heading.lower() in h[2].lower()]
    if not hit:
        sys.exit(f"no heading containing '{heading}' in {d['title']}; use --sections")
    pos, level, _ = hit[0]
    sub = [(lvl, h) for p, lvl, h in heads if pos < p < end_of(pos, level) and lvl > level]
    print(cap_text(text[pos: end_of(pos, level)].rstrip(), sub, d["title"], "vault"))


def tldr_parts(d):
    """TL;DR bullets and the 'Read more' section list, without the callout header and cost boilerplate."""
    if not d["tldr"]:
        return ([d["summary"]] if d["summary"] else []), ""
    bullets, more = [], ""
    for line in d["tldr"].split("\n"):
        line = line.lstrip(">").strip()
        if not line or line.startswith("[!tldr]"):
            continue
        if line.startswith("Cost:"):
            m = re.search(r"Read more:\s*(.*)", line)
            more = m.group(1).replace("## ", "").strip() if m else ""
            continue
        bullets.append(line if line.startswith("- ") else "- " + line)
    return bullets, more


_API = None


def api_index():
    """name -> [(class or None, status)] from the dated API snapshot; status 0 = current, 1 = deprecated, str = replacement."""
    global _API
    if _API is None:
        _API = defaultdict(list)
        try:
            d = json.load(open(os.path.join(KB, "_tools", "roblox_api.json"), encoding="utf-8"))
        except OSError:
            return _API
        for cls, info in d.get("classes", {}).items():
            _API[cls].append((None, info.get("deprecated", 0)))
            for m, st in info.get("members", {}).items():
                _API[m].append((cls, st))
        try:  # newer doc deprecations than the snapshot knows
            over = json.load(open(os.path.join(KB, "_tools", "api_overrides.json"), encoding="utf-8"))
        except OSError:
            over = {}
        for key, st in over.items():
            if key.startswith("_") or "." not in key:
                continue
            cls, m = key.split(".", 1)
            rows = _API[m]
            rows[:] = [r for r in rows if r[0] != cls] + [(cls, st)]
    return _API


def api_status(cls, mem):
    """Rows for Class.Member, or for a bare name on any class."""
    return [r for r in api_index().get(mem or cls, []) if mem is None or r[0] == cls]


def api_notes(text):
    """One line per deprecated or renamed API named in the text, quiet for current or unknown names."""
    out, seen = [], set()
    for m in re.finditer(r"\b([A-Z][A-Za-z0-9]+)(?:[.:]([A-Z][A-Za-z0-9]+))?\b", text):
        cls, mem = m.group(1), m.group(2)
        rows = api_status(cls, mem)
        name = mem or cls
        if not rows or name in seen or not all(st for _, st in rows):
            continue
        seen.add(name)
        c, st = rows[0]
        out.append(f"- {c + '.' if c else ''}{name}: deprecated" + (f", use {st}" if isinstance(st, str) else ""))
    return out


def main():
    try:  # Windows consoles default to cp1252, and notes contain arrows and dashes
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    except (AttributeError, ValueError):
        pass
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return
    idx = load_index()
    if args[0] == "--read":
        print_section(args[1], args[2] if len(args) > 2 else None, idx)
        return
    if args[0] == "--sections":
        print_section(args[1], None, idx)
        return
    if args[0] == "--api":  # check API names against the dated snapshot: --api TeleportService.ReserveServer AwardBadge
        for m in re.finditer(r"([A-Za-z0-9]+)(?:[.:]([A-Za-z0-9]+))?", " ".join(args[1:])):
            cls, mem = m.group(1), m.group(2)
            rows = api_status(cls, mem)
            if not rows:
                print(f"{m.group(0)}: not in the API snapshot (check the name)")
            for c, st in rows[:4]:
                state = "current" if not st else "deprecated" + (f", use {st}" if isinstance(st, str) else "")
                print(f"{c + '.' if c else ''}{mem or cls}: {state}")
        return
    k, mode, words = 3, "compact", []
    it = iter(args)
    for a in it:
        if a == "-k":
            k = int(next(it))
        elif a in ("--paths", "--json", "--full", "--brief"):
            mode = a[2:]
        else:
            words.append(a)
    results, symptoms = search(" ".join(words), k, idx)
    if mode == "json":
        print(json.dumps({"results": [{"title": d["title"], "path": d["path"], "score": round(s, 2),
                                       "tldr": d["tldr"] or d["summary"]} for s, d in results],
                          "symptoms": [s["line"] for s in symptoms]}, indent=1))
        return
    if not results:
        print("no match; try other words")
        return
    if mode == "paths":
        for s, d in results:
            print(f"{d['title']} | {d['path']}")
        return
    if mode == "full":
        for s, d in results:
            print(f"## {d['title']}  ({d['path']}, {d['kb']} KB)")
            print(d["tldr"] or ("> " + d["summary"] if d["summary"] else "> (no summary)"))
            print()
        if symptoms:
            print("Symptom matches (symptom | cause | fix | note):")
            for s in symptoms:
                print("- " + s["line"])
            print()
        print('Next: python "LihFolk Vault V2/_tools/vault.py" --read "<title>" "<heading words>"  (one section only)')
        return
    shown = set()
    for rank, (s, d) in enumerate(results, 1):
        bullets, more = tldr_parts(d)
        print(f"{rank}. {d['title']}")
        brief = mode == "brief" and rank > 1
        bullets = [b for b in (bullets[:2] if brief else bullets) if b not in shown]  # no repeats across notes
        shown.update(bullets)
        if bullets:
            print("\n".join(bullets))
        if more and rank == 1:  # --sections lists any other note's headings
            print("Sections: " + more)
    notes = api_notes(" ".join(words))
    if notes:
        print("API status (snapshot):")
        print("\n".join(notes))
    if symptoms:
        print("Symptoms (symptom | cause | fix | note):")
        for s in symptoms:
            print("- " + s["line"])



def _list(folder):
    root = os.path.join(KB, folder) if folder else KB
    if not os.path.isdir(root) or not os.path.abspath(root).startswith(os.path.abspath(KB)):
        raise KitError("No vault folder called %r. Call rbx_vault with action 'list' and no folder for the sections." % folder)
    dirs = sorted(d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)) and not d.startswith((".", "_")))
    notes = sorted(f[:-3] for f in os.listdir(root) if f.endswith(".md"))
    out = [("Folders: " + ", ".join(dirs)) if dirs else ""]
    out += ["Notes:"] + ["- " + n for n in notes] if notes else []
    return "\n".join(x for x in out if x) or "(empty)"


# ======================================================================= shelves (rbxkit)
# Everything below is rbxkit's: one lookup over several "shelves". The vault keeps the note-level engine above; the
# other shelves are ranked by SECTION (each heading with its own text) so an agent reads one section, not a whole file.
#   vault   the LihFolk vault notes (vault/Roblox)
#   guides  every guide rbx_guide serves (rbxkit_mcp.GUIDES) and their references/ files
#   team    the Studio team's expert files (vault/Fridae/Team/*.md, a game's .fridae/team/experts/ overrides them)
#   game    this game's GAME.md, lessons and notes (only when RBXKIT_PROJECT is set); indexed on the fly, never cached
#   yours   the user's notes for everything (<home>/notes, rbxnotes.py); indexed on the fly, works with no project
SHELVES = ("vault", "guides", "team", "game", "yours")
PERSONAL = ("game", "yours")  # the user's own shelves: short notes are fine there, and words are folded more
SECTION_FIELDS = {"title": 2.0, "tldr": 1.0, "heads": 3.0, "body": 1.0}
READ_CAP = 12000  # bytes one read returns at most
GUIDES_CACHE_NAME = "knowledge-sections-index.json"
INDEX_FORMAT = 3  # bump when the section index's shape changes (2: labels, links and the gist; 3: supersedes)
OPENING, WHOLE = "(opening)", "(whole file)"
_FENCE = re.compile(r"^[ \t]{0,3}(```+|~~~+)")
_NAV = re.compile(r"^(related|see also|connected notes|files|sources|references|further reading)\b", re.I)
_HEAD = re.compile(r"^(#{1,6})[ \t]+(.+?)[ \t]*#*[ \t]*$")


def size_text(n):
    return "%.1f KB" % (n / 1024)


def split_front(text):
    """(front matter as a dict of simple keys, the text after it). Folded values (key: >) are joined into one line."""
    m = re.match(r"^﻿?---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(\r?\n|$)", text, re.S)
    if not m:
        return {}, text
    fm, key = {}, None
    for line in m.group(1).splitlines():
        mm = re.match(r"^([A-Za-z_][\w.-]*):[ \t]*(.*)$", line)
        if mm:
            key, val = mm.group(1), mm.group(2).strip()
            fm[key] = "" if val in (">", "|", ">-", "|-", ">+", "|+") else val.strip("\"'")
        elif key and line[:1] in (" ", "\t"):
            fm[key] = (fm[key] + " " + line.strip()).strip()
    return fm, text[m.end():]


# ------------------------------------------------------------------ note labels, gists and links (both searches)
# Every note gets four labels, worked out from its front matter and where it lives (no model involved):
#   game    the game's name for a game's own notes (GAME.md, .fridae/notes, its team overrides), else "all"
#   kind    rule | lesson | taste | reference | guide
#   source  lihfolk | user | agent | community | game | fridae (Fridae's own guides and team files)
#   date    YYYY-MM-DD: the front matter's date, else the file's modification day
# A "Learned in Fridae" section is a lesson an agent added, whatever the note around it is.
KINDS = ("rule", "lesson", "taste", "reference", "guide")
SOURCES = ("lihfolk", "user", "agent", "community", "game", "fridae")
_KIND_WORDS = {
    "rule": "rule", "rules": "rule", "house-rule": "rule", "house-rules": "rule", "policy": "rule",
    "lesson": "lesson", "lessons": "lesson", "learned": "lesson", "learning": "lesson", "note": "lesson",
    "taste": "taste", "style": "taste", "preference": "taste",
    "reference": "reference", "atlas": "reference", "glossary": "reference", "case-study": "reference",
    "hub": "reference", "api": "reference", "card": "reference", "concept": "reference", "teardown": "reference",
    "guide": "guide", "how-to": "guide", "howto": "guide", "tutorial": "guide", "checklist": "guide",
    "recipe": "guide", "build": "guide", "template": "guide", "troubleshooting": "guide", "playbook": "guide"}
_REFERENCE_FOLDERS = ("46 Resource Library/", "52 Core Concepts/", "57 API and Concept Atlas/",
                      "62 Engine Reference Cards/")
_LEARNED = re.compile(r"^learned in fridae\b", re.I)
_CREDIT = re.compile(r"(CC BY[^\n]{0,40}LihFolk|LihFolk[^\n]{0,40}CC BY|\(c\)[^\n]{0,20}LihFolk)", re.I)
_DATE = re.compile(r"\b(\d{4})-(\d{2})(?:-(\d{2}))?\b")
_USER_BY = {"user", "you", "me", "the user", "owner", "human"}
_WIKILINK = re.compile(r"\[\[([^\]\|#\n]+)(?:#[^\]\|\n]*)?(?:\|[^\]\n]*)?\]\]")
LINKS_MAX = 40  # wikilinks kept per note


def game_name(project):
    return os.path.basename(os.path.normpath(project)) if project else ""


def _fm_list(v):
    v = (v or "").strip()
    if v.startswith("[") and v.endswith("]"):
        v = v[1:-1]
    return [x.strip().strip("'\"") for x in re.split(r"[,\s]+", v) if x.strip().strip("'\"")]


def _kind_word(w):
    w = (w or "").strip().casefold()
    if w.startswith("type/") or w.startswith("kind/"):
        w = w.split("/", 1)[1]
    return _KIND_WORDS.get(w) or (w if w in KINDS else "")


def _in_kb(path):
    try:
        kb = os.path.abspath(KB)
        return os.path.commonpath([kb, os.path.abspath(path)]) == kb
    except ValueError:
        return False


def _file_day(path):
    import time
    try:
        return time.strftime("%Y-%m-%d", time.localtime(os.stat(path).st_mtime))
    except (OSError, OverflowError, ValueError):
        return ""


def note_labels(shelf, title, path, fm=None, body="", project=""):
    """{"game", "kind", "source", "date"} for one note (see KINDS and SOURCES above). fm: its front matter
    (split_front); body: its text (for a credit line); project: the game's folder when the note is that game's."""
    fm = fm or {}
    norm = (path or "").replace(os.sep, "/")
    base = os.path.basename(norm)
    t = (title or "").casefold()
    kb_rel = os.path.relpath(path, KB).replace(os.sep, "/") if path and _in_kb(path) else ""
    # kind: the front matter (kind, type, tags), else the folder or the title, else what the shelf holds
    kind = _kind_word(fm.get("kind")) or _kind_word(fm.get("type"))
    if not kind:
        for tag in _fm_list(fm.get("tags")):
            if tag.casefold().startswith(("type/", "kind/")) or tag.casefold() in KINDS:
                kind = _kind_word(tag)
                if kind:
                    break
    if not kind:
        if t in ("house rules", "house rules.md"):
            kind = "rule"
        elif "/taste/" in "/" + norm.casefold():
            kind = "taste"
        elif t in ("lessons.md", "studio-lessons", "lessons"):
            kind = "lesson"
        elif kb_rel.startswith(_REFERENCE_FOLDERS) or t == "glossary":
            kind = "reference"
        elif base == "GAME.md":
            kind = "reference"
        else:
            kind = {"vault": "reference", "guides": "guide", "team": "guide"}.get(shelf, "lesson")
    # source: the front matter, else the shelf and path, the credit line, who wrote it
    src = (fm.get("source") or fm.get("origin") or "").strip().casefold()
    source = src if src in SOURCES else ""
    if not source and (fm.get("contributor") or fm.get("submitted_by")):
        source = "community"
    if not source and shelf == "game":
        source = "game"
    if not source and fm.get("by"):
        source = "user" if fm["by"].strip().casefold() in _USER_BY else "agent"
    if not source and (kb_rel or "/lihfolk/" in norm.casefold() or (body and _CREDIT.search(body[-4000:] + body[:2000]))):
        source = "lihfolk"
    if not source:
        if t in ("studio-lessons", "lessons.md"):
            source = "agent"
        elif shelf in ("guides", "team"):
            source = "fridae"
        else:
            source = "user"
    day = ""
    for key in ("updated", "date", "when", "created", "verified", "last_verified"):
        m = _DATE.search(fm.get(key) or "")
        if m:
            day = "%s-%s-%s" % (m.group(1), m.group(2), m.group(3) or "01")
            break
    return {"game": game_name(project) if project else "all", "kind": kind, "source": source,
            "date": day or _file_day(path)}


def section_labels(labels, heading, parents=()):
    """A section's labels: its note's, except that a "Learned in Fridae" section is an agent's lesson."""
    if any(_LEARNED.match(h or "") for h in list(parents or ()) + [heading or ""]):
        return dict(labels, kind="lesson", source="agent")
    return labels


def _plain(s):
    s = re.sub(r"\[\[([^\]\|]+)\|([^\]]+)\]\]", r"\2", s)
    s = re.sub(r"\[\[([^\]#\|]+)(#[^\]]*)?\]\]", r"\1", s)
    s = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", s)
    s = re.sub(r"[*_`]{1,3}", "", s)
    return " ".join(s.split())


def note_gist(fm, body, words=30):
    """A note's gist: its front matter description or summary, else its first meaningful sentence, at most about
    `words` words."""
    g = (fm or {}).get("description") or (fm or {}).get("summary") or ""
    if not g:
        fence = False
        para = []
        for line in body.split("\n"):
            st = line.strip()
            if _FENCE.match(st):
                fence = not fence
                if para:
                    break
                continue
            if fence:
                continue
            if not st:
                if para:
                    break
                continue
            if (st.startswith(("#", "|", "!", "<!--", ">", "---", "***")) or _HEAD.match(st)
                    or re.match(r"^(source labels|license|credit)\b", st, re.I)):
                if para:
                    break
                continue
            para.append(st.lstrip("-*+ ").strip() if not para else st)
            if st.startswith(("- ", "* ", "+ ")):
                break  # a list: its first item is the gist
        g = " ".join(para)
        m = re.search(r"(?<=[.!?])\s", g)
        if m and len(g[:m.start()].split()) >= 4:
            g = g[:m.start()]
    g = _plain(g)
    w = g.split()
    return " ".join(w[:words]) + ("…" if len(w) > words else "")


def wikilinks(text, limit=LINKS_MAX):
    """The distinct [[note]] targets in a text, in order, casefolded (aliases and #headings dropped)."""
    out, seen = [], set()
    for m in _WIKILINK.finditer(text or ""):
        t = " ".join(m.group(1).split()).casefold()
        if t.endswith(".md"):
            t = t[:-3]
        t = t.rsplit("/", 1)[-1]
        if t and t not in seen:
            seen.add(t)
            out.append(t)
            if len(out) >= limit:
                break
    return out


# ------------------------------------------------------------------ superseded notes (both searches)
# A newer note names the notes it replaces in its front matter (`supersedes: "Old title"`, or a list); filing it with
# rbx_vault 'note' also marks the old note (`superseded_by: "New title"`, `superseded_on: <date>`), or, for a shipped
# note (vault/Roblox, vault/Fridae: read only), adds it to the notes folder's superseded.json (rbxnotes.SIDECAR).
# This is read when a search runs, never put in the knowledge index: a superseded note's score is multiplied by
# SUPERSEDED_SCALE (the replacement wins a tie), its first line says "(replaced by <title>)", and it is never dropped.
SUPERSEDED_SCALE = 0.3
_WIKI_VALUE = re.compile(r"^\[\[([^\]\|#]+)(?:#[^\]\|]*)?(?:\|[^\]]*)?\]\]$")
_LIST_ITEM = re.compile(r'\s*(?:"((?:[^"\\]|\\.)*)"|\'([^\']*)\'|([^,]+?))\s*(?:,|$)')


def _title_value(v):
    v = v.strip().strip("\"'").strip()
    m = _WIKI_VALUE.match(v)
    if m:
        v = m.group(1)
    v = " ".join(v.replace('\\"', '"').split())
    if v.casefold().endswith(".md"):
        v = v[:-3]
    return v.rsplit("/", 1)[-1].strip()


def front_titles(text, key):
    """The note titles a front matter key names: `key: "A"`, `key: A`, `key: ["A", "B"]`, `key: [A, B]`, a block list
    (`key:` then `  - A` lines) or `[[A]]` links; [] when the text has no front matter or no such key."""
    m = re.match(r"^﻿?---[ \t]*\r?\n(.*?)\r?\n---[ \t]*(\r?\n|$)", text or "", re.S)
    if not m:
        return []
    lines = m.group(1).splitlines()
    for n, line in enumerate(lines):
        mm = re.match(r"^%s[ \t]*:[ \t]*(.*)$" % re.escape(key), line)
        if not mm:
            continue
        v = mm.group(1).strip()
        if not v:
            vals = []
            for nxt in lines[n + 1:]:
                item = re.match(r"^[ \t]*-[ \t]+(.+)$", nxt)
                if not item:
                    break
                vals.append(item.group(1))
        elif v.startswith("[") and not v.startswith("[["):
            inner = v[1:-1] if v.endswith("]") else v[1:]
            vals = [a or b or c for a, b, c in _LIST_ITEM.findall(inner)]
        else:
            vals = [v]
        out = []
        for x in vals:
            t = _title_value(x)
            if t and t.casefold() not in {o.casefold() for o in out}:
                out.append(t)
        return out
    return []


def supersession(text):
    """(the titles a note supersedes, the title it is superseded by or "") from its front matter."""
    by = front_titles(text, "superseded_by")
    return front_titles(text, "supersedes"), by[0] if by else ""


def superseded_map(entries=(), sidecars=None):
    """{old title casefolded: the title that replaces it}. entries: dicts with a "title" and optional "supersedes"
    (titles) and "superseded_by"; sidecars: {old casefolded: new} (rbxnotes.sidecars). The old note's own mark wins,
    then a sidecar, then a newer note's claim; a note never replaces itself."""
    claims, marks = {}, {}
    for d in entries:
        if d.get("superseded_by"):
            marks.setdefault(d["title"].casefold(), d["superseded_by"])
        for old in d.get("supersedes") or ():
            claims.setdefault(old.casefold(), d["title"])
    out = dict(claims)
    out.update(sidecars or {})
    out.update(marks)
    return {old: new for old, new in out.items() if new and new.casefold() != old}


def replaced_by(sup, d):
    """The title replacing note d (a doc or hit with "title" and maybe "label"), or None."""
    if not sup:
        return None
    t = (d.get("title") or "").casefold()
    hit = sup.get(t)
    if hit is None and d.get("label") and d["label"].casefold() != t:
        hit = sup.get(d["label"].casefold())
    return hit


def demote(items, score_of, replaced_of, title_of=None, scale=SUPERSEDED_SCALE):
    """[(new score, item)] best first: a superseded item's score times SUPERSEDED_SCALE; on a tie the item that is
    not superseded comes first, then the order given. With title_of, a superseded item still above its replacement
    (scores on another scale, a reranker's) is put right after it, its score lowered to the replacement's. Nothing
    is dropped."""
    scored = []
    for n, x in enumerate(items):
        by = replaced_of(x)
        s = score_of(x) * (scale if by else 1.0)
        scored.append((-s, bool(by), n, s, x, by))
    scored.sort(key=lambda r: r[:3])
    out = [(s, x, by) for _, _, _, s, x, by in scored]
    if title_of:
        for x in [r for r in out if r[2]]:
            p = next(i for i, r in enumerate(out) if r is x)
            want = x[2].casefold()
            q = next((i for i in range(p + 1, len(out)) if (title_of(out[i][1]) or "").casefold() == want), None)
            if q is not None:  # its score no higher than its replacement's, so a later sort keeps it after
                s, item, by = out.pop(p)
                out.insert(q, (min(s, out[q - 1][0]), item, by))
    return [(s, x) for s, x, _ in out]


def _sup_entries(docs):
    """The entries of docs that say anything about superseding (kept on a Docs, which never changes)."""
    if isinstance(docs, Docs):
        hit = docs.__dict__.get("_sup_entries")
        if hit is None:
            hit = docs.__dict__["_sup_entries"] = [d for d in docs if d.get("supersedes") or d.get("superseded_by")]
        return hit
    return [d for d in docs if d.get("supersedes") or d.get("superseded_by")]


def _personal_docs(project=None):
    """The section entries of the user's own shelves (cached per file)."""
    docs = [d for t, p in yours_files() for d in _section_docs("yours", t, p, p)]
    project = project_dir() if project is None else project
    if project and os.path.isdir(project):
        docs += [d for t, p in game_files(project) for d in _section_docs("game", t, p, p, "", project)]
    return docs


def superseded_for(idx=None, docs=None, project=None):
    """The superseded map (superseded_map) for a search: the vault index's notes when it is loaded, the section
    entries (else the user's own notes) and the notes folders' sidecars."""
    import rbxnotes
    project = project_dir() if project is None else project
    entries = _sup_entries(idx["docs"]) if idx else []
    entries = entries + _sup_entries(_personal_docs(project) if docs is None else docs)
    return superseded_map(entries, rbxnotes.sidecars(project or None))


def label_text(labels, project=""):
    """The compact label of one result, e.g. "[lesson · this game · 2026-11]" or "[guide · lihfolk · 2026-10]"."""
    if not labels:
        return ""
    where = "this game" if labels.get("game") not in (None, "", "all") else labels.get("source") or ""
    bits = [labels.get("kind") or "", where, (labels.get("date") or "")[:7]]
    return "[%s]" % " · ".join(b for b in bits if b)


def day_number(date):
    m = _DATE.search(date or "")
    return int("%s%s%s" % (m.group(1), m.group(2), m.group(3) or "01")) if m else 0


def normalize_filters(a, project=None):
    """The search filters from a request: {"game": "this" | "general", "kind": [..], "source": [..],
    "since": YYYYMMDD}, only the ones given (an empty dict filters nothing). game: "this" (or the game's own name)
    for this game's notes only, "general" (or "shared") for the notes for every game; "all" or "any" is no filter.
    kind and source: one value or several (a list or comma-separated). since: a date (YYYY, YYYY-MM or YYYY-MM-DD)."""
    out = {}
    g = str(a.get("game") or "").strip().casefold()
    if g and g not in ("all", "any", "everything", "*"):
        if g in ("this", "current", "this game", "mine") or (project and g == game_name(project).casefold()):
            if not project:
                raise KitError("game 'this' needs a chat with a game project.")
            out["game"] = "this"
        elif g in ("general", "shared", "all games", "every game", "none"):
            out["game"] = "general"
        else:
            raise KitError("game must be 'this' (this game's notes only), 'general' (notes for every game) or 'all'.")
    for key, allowed in (("kind", KINDS), ("source", SOURCES)):
        v = a.get(key)
        vals = v if isinstance(v, list) else re.split(r"[,\s|]+", str(v or ""))
        vals = [str(x).strip().casefold() for x in vals if str(x).strip()]
        if key == "kind":
            vals = [_KIND_WORDS.get(x, x) for x in vals]
        bad = [x for x in vals if x not in allowed]
        if bad:
            raise KitError("%s must be one or more of: %s." % (key, ", ".join(allowed)))
        if vals:
            out[key] = sorted(set(vals))
    since = a.get("since")
    if since not in (None, ""):
        if isinstance(since, int) and since > 10000000:
            out["since"] = since
        else:
            m = re.match(r"^\s*(\d{4})(?:-(\d{1,2}))?(?:-(\d{1,2}))?\s*$", str(since))
            if not m:
                raise KitError("since must be a date: YYYY, YYYY-MM or YYYY-MM-DD.")
            out["since"] = int("%s%02d%02d" % (m.group(1), int(m.group(2) or 1), int(m.group(3) or 1)))
    return out


def labels_match(labels, filters):
    """True when a note's labels pass the filters (normalize_filters)."""
    if not filters:
        return True
    labels = labels or {}
    g = filters.get("game")
    if g == "this" and labels.get("game") in (None, "", "all"):
        return False
    if g == "general" and labels.get("game") not in (None, "", "all"):
        return False
    if filters.get("kind") and labels.get("kind") not in filters["kind"]:
        return False
    if filters.get("source") and labels.get("source") not in filters["source"]:
        return False
    if filters.get("since") and day_number(labels.get("date")) < filters["since"]:
        return False
    return True


def parse_sections(text):
    """A Markdown file split into sections, ignoring '#' lines inside code fences.

    Returns {"fm", "body", "label", "description", "sections"}; each section is {"heading", "level", "parents",
    "start", "own_end", "end"} with offsets into body: own_end is where the next heading starts (the section's own
    text), end is where the next heading of the same or a higher level starts (what a read returns). The text before
    the first heading is the "(opening)" section (level 0); a top H1 is the file's label, not a section.
    """
    fm, body = split_front(text)
    heads, fence, pos = [], None, 0
    for line in body.splitlines(keepends=True):
        f = _FENCE.match(line)
        if f:
            mark = f.group(1)
            if fence is None:
                fence = mark
            elif mark[0] == fence[0] and len(mark) >= len(fence) and not line.strip()[len(mark):].strip():
                fence = None
        elif fence is None:
            h = _HEAD.match(line.rstrip("\r\n"))
            if h and len(h.group(1)) <= 3:
                heads.append((pos, len(h.group(1)), h.group(2).strip()))
        pos += len(line)
    label = ""
    first_h1 = None
    if heads and heads[0][1] == 1 and not body[:heads[0][0]].strip():
        first_h1 = heads.pop(0)
        label = first_h1[2]
    label = label or fm.get("name", "")
    secs = []
    first = heads[0][0] if heads else len(body)
    open_from = 0
    if first_h1:  # the H1 line alone is not an opening worth a section
        nl = body.find("\n", first_h1[0])
        open_from = first if nl < 0 else min(nl + 1, first)
    if body[open_from:first].strip():
        secs.append({"heading": OPENING if heads else WHOLE, "level": 0, "parents": [], "start": 0, "own_end": first,
                     "end": first})
    stack = []
    for i, (p, lvl, h) in enumerate(heads):
        own_end = heads[i + 1][0] if i + 1 < len(heads) else len(body)
        end = next((q for q, ll, _ in heads[i + 1:] if ll <= lvl), len(body))
        stack = [s for s in stack if s[0] < lvl]
        secs.append({"heading": h, "level": lvl, "parents": [s[1] for s in stack], "start": p, "own_end": own_end,
                     "end": end})
        stack.append((lvl, h))
    return {"fm": fm, "body": body, "label": label, "description": fm.get("description", ""), "sections": secs}


def _norm(t):
    """Section shelves only: fold the stemmer's leftovers together (price/prices -> pric, upgrade/upgrades -> upgrad)."""
    return t[:-1] if len(t) > 4 and t.endswith("e") else t


def stokens(s):
    return [_norm(t) for t in tokens(s)]


def squery_terms(q):
    terms = Counter()
    for t, w in query_terms(q).items():
        terms[_norm(t)] += w
    return terms


# The user's own shelves (game, yours) only: their notes are short and in their own words, so a few more word forms
# are folded together. The vault and the kit shelves keep their ranking.
_IRREGULAR = {"chos": "choos", "chosen": "choos", "picked": "pick", "mad": "mak", "made": "mak", "built": "build",
              "went": "go", "gone": "go", "ran": "run", "wrote": "writ", "written": "writ", "liked": "lik",
              "hated": "hat", "hate": "hat", "felt": "feel", "saw": "see", "seen": "see", "kept": "keep",
              "bought": "buy", "sold": "sell", "paid": "pay", "taught": "teach", "thought": "think", "led": "lead"}
_JOIN = re.compile(r"\b(data)[ \t-]+(stores?)\b", re.I)


def _pfold(t):
    t = _norm(t)
    return _IRREGULAR.get(t, t)


def ptokens(s):
    return [_pfold(t) for t in tokens(_JOIN.sub(r"\1\2", s))]


def pquery_terms(q):
    terms = Counter()
    for t, w in query_terms(_JOIN.sub(r"\1\2", q)).items():
        terms[_pfold(t)] += w
    return terms


def _code_capped(text):
    return re.sub(r"```.*?```", lambda c: c.group(0)[:600], text, flags=re.S)


_PERSONAL_CACHE = {}  # (shelf, title, path, rel, label_hint, project) -> ((mtime_ns, size), entries)


def _section_docs(shelf, title, path, rel, label_hint="", project=""):
    """Index entries for one file: one per section. The user's own shelves are read on every search, so their
    entries are kept per file and parsed again only when the file changes. project: the game's folder when the
    file is that game's own (its labels say so)."""
    if shelf in PERSONAL:
        key = (shelf, title, path, rel, label_hint, project)
        try:
            st = os.stat(path)
            sig = (st.st_mtime_ns, st.st_size)
        except OSError:
            sig = None
        hit = _PERSONAL_CACHE.get(key)
        if sig is not None and hit and hit[0] == sig:
            return hit[1]
        out = _parse_section_docs(shelf, title, path, rel, label_hint, project)
        if sig is not None:
            if len(_PERSONAL_CACHE) > 50000:
                _PERSONAL_CACHE.clear()
            _PERSONAL_CACHE[key] = (sig, out)
        return out
    return _parse_section_docs(shelf, title, path, rel, label_hint, project)


def _parse_section_docs(shelf, title, path, rel, label_hint="", project=""):
    try:
        text = read(path)
    except (OSError, UnicodeDecodeError):
        return []
    p = parse_sections(text)
    label = p["label"] or label_hint or title
    tok = ptokens if shelf in PERSONAL else stokens
    # on the user's own shelves, what an agent learned (a lesson) is shown apart from what the user said (a note)
    lesson = shelf in PERSONAL and (p["fm"].get("kind") == "lesson" or title in ("lessons.md", "studio-lessons"))
    labels = note_labels(shelf, title, path, p["fm"], text, project)
    gist = p["description"] or note_gist(p["fm"], p["body"])  # the note's context line, as the knowledge index has
    olds, by = supersession(text)
    sup = dict({"supersedes": olds} if olds else {}, **({"superseded_by": by} if by else {}))
    out = []
    for i, s in enumerate(p["sections"]):
        own = p["body"][s["start"]:s["own_end"]]
        if s["level"]:
            own = own.split("\n", 1)[1] if "\n" in own else ""
        heads = " ".join(s["parents"] + ([s["heading"]] if s["level"] else [label]))
        f = {"title": tok(title.replace("-", " ") + " " + label), "tldr": tok(gist),
             "heads": tok(heads), "body": tok(_code_capped(own))}
        tf = {k: Counter(v) for k, v in f.items()}
        out.append({"shelf": shelf, "title": title, "label": label, "rel": rel, "path": path, "i": i,
                    "heading": s["heading"], "parents": s["parents"], "level": s["level"],
                    "size": len(p["body"][s["start"]:s["end"]].encode("utf-8")),
                    "own": len(own.strip()), "tf": tf, "len": {k: sum(c.values()) for k, c in tf.items()},
                    "lesson": lesson, "labels": section_labels(labels, s["heading"], s["parents"]),
                    "links": wikilinks(own), **sup})
    return out


def guide_files():
    """[(title, absolute path)] for every Markdown guide rbx_guide serves, plus each guide folder's references/*.md."""
    import rbxkit_mcp  # lazy: the guide list lives in the tool server
    out, seen, ref_dirs = [], set(), set()
    for name, (rel, _) in rbxkit_mcp.GUIDES.items():
        path = os.path.normpath(os.path.join(KIT, rel))
        if rel.endswith(".md") and os.path.isfile(path) and path not in seen:
            out.append((name, path))
            seen.add(path)
    for name, (rel, _) in rbxkit_mcp.GUIDES.items():
        refs = os.path.join(os.path.dirname(os.path.normpath(os.path.join(KIT, rel))), "references")
        if refs in ref_dirs or not os.path.isdir(refs):
            continue
        ref_dirs.add(refs)
        for f in sorted(os.listdir(refs)):
            path = os.path.join(refs, f)
            if f.endswith(".md") and not f.startswith(".") and path not in seen:
                out.append(("%s/%s" % (name, f[:-3]), path))
                seen.add(path)
    return out


TEAM = os.path.join(KIT, "vault", "Fridae", "Team")  # the roster and one <id>.md per expert


def kit_team_dir():
    return TEAM


def team_files(folder):
    if not os.path.isdir(folder):
        return []
    return [(f[:-3], os.path.join(folder, f)) for f in sorted(os.listdir(folder)) if f.endswith(".md") and
            not f.startswith(".")]


def _team_label(expert_id):
    try:
        with open(os.path.join(TEAM, "roster.json"), encoding="utf-8") as f:
            names = {e.get("id"): e.get("name") for e in json.load(f) if isinstance(e, dict)}
    except (OSError, ValueError):
        names = {}
    return "%s (team expert)" % (names.get(expert_id) or expert_id)


def project_dir():
    p = (os.environ.get("RBXKIT_PROJECT") or "").strip()
    return p if p and os.path.isdir(p) else None


def game_files(project):
    """(title, path) for this game's own notes that exist: GAME.md, the old lessons file and .fridae/notes/."""
    import rbxnotes
    cands = [("GAME.md", os.path.join(project, "GAME.md")),
             ("lessons.md", os.path.join(project, ".fridae", "lessons.md"))]
    return [(t, p) for t, p in cands if os.path.isfile(p)] + rbxnotes.shelf_files(rbxnotes.game_notes(project))


def yours_files():
    import rbxnotes
    return rbxnotes.yours_files()


def _kit_files():
    return [("guides", t, p) for t, p in guide_files()] + [("team", t, p) for t, p in team_files(kit_team_dir())]


def load_sections():
    """The cached part of the section index (guides and the kit's team files), rebuilt when any file changes."""
    files = _kit_files()
    h = hashlib.sha1(("%s|%s" % (INDEX_FORMAT, rbxpaths.VERSION)).encode())
    # this file is stat'ed only when it exists: in the packaged app it lives inside the program's archive;
    # roster.json gives the team files their labels
    for _, t, p in files + [("", "", os.path.abspath(__file__)), ("", "", os.path.join(TEAM, "roster.json"))]:
        if os.path.exists(p):
            st = os.stat(p)
            h.update(f"{t}|{p}|{st.st_mtime_ns}|{st.st_size}".encode())
    sig, path = h.hexdigest(), cache_path(GUIDES_CACHE_NAME)
    try:
        idx = json.loads(read(path))
        if idx.get("sig") == sig:
            return idx["docs"]
    except (OSError, ValueError):
        pass
    docs = []
    for shelf, t, p in files:
        docs += _section_docs(shelf, t, p, os.path.relpath(p, KIT).replace(os.sep, "/"),
                              _team_label(t) if shelf == "team" else "")
    for d in docs:
        if d["shelf"] == "team":
            d["label"] = _team_label(d["title"])
    tmp = None
    try:  # written whole or not at all: a temporary file, then one rename
        import tempfile
        os.makedirs(os.path.dirname(path), exist_ok=True)
        fd, tmp = tempfile.mkstemp(prefix=".tmp-", suffix=".json", dir=os.path.dirname(path))
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump({"sig": sig, "docs": docs}, f)
        os.replace(tmp, path)
    except OSError:
        if tmp:
            try:
                os.remove(tmp)
            except OSError:
                pass
    return docs


def section_corpus(project=None, kit=None):
    """Every section doc: the cached kit part, with the yours shelf, a game's team overrides and the game shelf added
    on the fly. project: the game's folder ("" for none; None: RBXKIT_PROJECT). kit: load_sections() when the caller
    keeps it loaded."""
    docs = load_sections() if kit is None else list(kit)  # never added to in place: the caller keeps kit
    for t, p in yours_files():
        docs = docs + _section_docs("yours", t, p, p)
    project = project_dir() if project is None else (project if project and os.path.isdir(project) else None)
    if not project:
        return docs
    mine = team_files(os.path.join(project, ".fridae", "team", "experts"))
    if mine:
        ids = {t for t, _ in mine}
        docs = [d for d in docs if not (d["shelf"] == "team" and d["title"] in ids)]
        for t, p in mine:
            docs += _section_docs("team", t, p, p, _team_label(t), project)
    for t, p in game_files(project):
        docs += _section_docs("game", t, p, p, "", project)
    return docs


def _coverage(terms, tf, fields):
    total = sum(terms.values()) or 1
    return sum(w for t, w in terms.items() if any(tf[f].get(t) for f in fields)) / total


def search_sections(q, shelves, k, docs=None):
    """[(score, coverage, doc)] best sections on the given shelves."""
    docs = section_corpus() if docs is None else docs
    hits = []
    # The kit shelves and the user's own shelves are ranked apart, each with its own word folding and statistics.
    # A handful of notes gives far smaller raw scores than a big corpus, so each group's scores are scaled to its own
    # best (1.0); the order inside a group is unchanged.
    for group, terms in ((False, squery_terms(q)), (True, pquery_terms(q))):
        if isinstance(docs, Docs):
            part = docs.part(("personal", group), lambda d, g=group: (d["shelf"] in PERSONAL) == g)
        else:
            part = [d for d in docs if (d["shelf"] in PERSONAL) == group]
        if not any(d["shelf"] in shelves for d in part):
            continue
        mine = []
        for s, d in zip(bm25f(terms, part, SECTION_FIELDS), part):
            if s <= 0 or d["shelf"] not in shelves:
                continue
            if d["own"] < 120 and not group:  # a heading with almost nothing under it is a poor thing to read
                s *= 0.5
            if _NAV.match(d["heading"]):  # link lists and file lists rank below real explanations
                s *= 0.5
            mine.append((s, _coverage(terms, d["tf"], SECTION_FIELDS), d))
        top = max((h[0] for h in mine), default=1) or 1
        hits += [(s / top, cov, d) for s, cov, d in mine]
    hits.sort(key=lambda x: -x[0])
    return hits[:k]


def snippet(text, terms, n=3, width=160):
    """Up to n lines of a section: the ones that share most words with the query, else the first ones."""
    lines, fence = [], False
    for line in text.split("\n"):
        st = line.strip()
        if _FENCE.match(st):
            fence = not fence
            continue
        if not st or _HEAD.match(st) or st.startswith("<!--") or re.match(r"^\|?[\s:|-]+\|?$", st):
            continue
        st = " ".join(st.strip("|").replace(" | ", " · ").split())
        lines.append(st if len(st) <= width else st[:width - 1].rstrip() + "…")
    if not lines:
        return []
    want = {_norm(t) for t in terms}
    scored = [(len(set(stokens(l)) & want), i) for i, l in enumerate(lines)]
    if max(sc for sc, _ in scored) > 0:
        pick = sorted(i for sc, i in sorted(scored, key=lambda x: (-x[0], x[1]))[:n] if sc > 0)
    else:
        pick = list(range(min(n, len(lines))))
    return [lines[i] for i in pick]


def _reader_text(d):
    """A file's text as an agent should see it (guide placeholders filled in)."""
    text = read(d["path"] if os.path.isabs(d["path"]) else os.path.join(KIT, d["path"]))
    if d["shelf"] == "guides":
        import rbxkit_mcp
        text = rbxkit_mcp.fill(text)
    return text


def heading_arg(p, s):
    """The heading words that pick exactly this section in a read."""
    if not s["level"]:
        return s["heading"]
    same = [x for x in p["sections"] if x["heading"].casefold() == s["heading"].casefold()]
    if len(same) > 1 and s["parents"]:
        return s["parents"][-1] + " > " + s["heading"]
    return s["heading"]


def find_section(p, heading):
    """The section a read's heading words name: exact heading first, then 'Parent > Child', then a heading that
    contains the words, then one that has every word. None when nothing matches."""
    want = " ".join(heading.split()).casefold()
    secs = p["sections"]
    if want in ("", "opening", OPENING, "whole", "whole file", WHOLE):
        return secs[0] if secs and not secs[0]["level"] else None
    for s in secs:
        if s["heading"].casefold() == want:
            return s
    if " > " in want:
        parent, child = [x.strip() for x in want.rsplit(" > ", 1)]
        for s in secs:
            if s["parents"] and parent in s["parents"][-1].casefold() and child in s["heading"].casefold():
                return s
    for s in secs:
        if s["level"] and want in s["heading"].casefold():
            return s
    words = want.split()
    for s in secs:
        if s["level"] and all(w in s["heading"].casefold() for w in words):
            return s
    return None


def cap_text(text, subheads, title, shelf, cap=READ_CAP):
    """A section in full, or its first part with the list of its sub-headings when it is larger than the cap."""
    raw = text.encode("utf-8")
    if len(raw) <= cap:
        return text
    cut = raw[:cap].decode("utf-8", "ignore")
    cut = cut[:cut.rfind("\n")] if "\n" in cut else cut
    note = "\n\n(This section is %s; only its first %d KB is shown." % (size_text(len(raw)), cap // 1000)
    if subheads:
        note += " Read one of its parts by heading instead: " + " · ".join(h for _, h in subheads)
        note += " (rbx_vault action 'read', shelf '%s', title '%s')." % (shelf, title)
    else:
        note += ")"
    return cut.rstrip() + note


def sections_listing(d_or_path, shelf, title):
    p = parse_sections(d_or_path)
    lines = []
    for s in p["sections"]:
        size = len(p["body"][s["start"]:s["end"]].encode("utf-8"))
        ind = "  " * max(0, s["level"] - 2)
        mark = ("#" * s["level"] + " ") if s["level"] else ""
        lines.append("%s%s%s  (%s)" % (ind, mark, s["heading"], size_text(size)))
    return p, lines


def resolve(title, shelf, docs):
    """(shelf, key) for a read: exact titles first (vault, then the other shelves), then a unique partial match."""
    t = title.strip().casefold()
    vault_ok = shelf in (None, "all", "vault") and os.path.isdir(KB)
    idx = load_index() if vault_ok else None
    sec = [d for d in docs if shelf in (None, "all") or d["shelf"] == shelf]
    if idx:
        for d in idx["docs"]:
            if d["title"].casefold() == t:
                return "vault", d["title"]
    for d in sec:
        if d["title"].casefold() == t or d["label"].casefold() == t:
            return d["shelf"], d["title"]
    if idx:
        cands = [d for d in idx["docs"] if t in d["title"].casefold()]
        if len(cands) == 1:
            return "vault", cands[0]["title"]
    keys = sorted({(d["shelf"], d["title"]) for d in sec if t in d["title"].casefold() or t in d["label"].casefold()})
    if len(keys) == 1:
        return keys[0]
    if keys:
        raise KitError("'%s' matches several files: %s. Give the exact title (and shelf)." % (
            title, ", ".join("%s (%s)" % (k, s) for s, k in keys[:12])))
    raise KitError("No file called '%s'%s. Search first: rbx_vault action 'search'." % (
        title, "" if shelf in (None, "all") else " on the %s shelf" % shelf))


def read_file_section(shelf, key, heading, docs):
    """One section (or, with no heading, the section list) of a guides/team/game file."""
    d = next(d for d in docs if d["shelf"] == shelf and d["title"] == key)
    text = _reader_text(d)
    p, lines = sections_listing(text, shelf, key)
    head = "[%s] %s" % (shelf, key) + (" — %s" % d["label"] if d["label"] and d["label"] != key else "")
    if heading is None and len(p["sections"]) != 1:
        return "%s (%s)\n%s\n\nNext: rbx_vault action 'read' with shelf '%s', title '%s' and the heading of ONE section." % (
            head, size_text(len(text.encode("utf-8"))), "\n".join(lines), shelf, key)
    s = find_section(p, heading or "") if heading is not None else p["sections"][0] if p["sections"] else None
    if s is None:
        raise KitError("No heading like '%s' in %s. Its sections:\n%s" % (heading, key, "\n".join(lines)))
    body = p["body"][s["start"]:s["end"]].rstrip()
    if not s["level"] and p["description"]:
        body = "> " + p["description"] + "\n\n" + body.lstrip()
    sub = [(x["level"], x["heading"]) for x in p["sections"] if s["start"] < x["start"] < s["end"]]
    out = "%s › %s (%s)\n\n%s" % (head, s["heading"], size_text(s_size(p, s)), cap_text(body, sub, key, shelf))
    return out + ("\n\n" + guide_excerpt_tag(key) if shelf == "guides" else "")


def s_size(p, s):
    return len(p["body"][s["start"]:s["end"]].encode("utf-8"))


def _vault_section_hint(d):
    _, more = tldr_parts(d)
    return [h.strip() for h in re.split(r"\s*·\s*", more) if h.strip()] if more else []


PROMOTE_AT = 0.2  # a shelf's best hit is moved up only when it matches at least this well


def personal_match(q, d):
    """A note on the user's own shelves may be moved up only when it shares at least two of the query's words, or
    half of them: one shared word is not enough to put it above everything else."""
    want = [t for t, w in pquery_terms(q).items() if w >= 1]
    have = sum(1 for t in want if any(d["tf"][f].get(t) for f in d["tf"]))
    return have >= 2 or (want and 2 * have >= len(want))


def interleave(pool, k, may_promote=lambda item: True):
    """The k results to show: first the best hit of each shelf that has a good one (best first), then the rest by
    score. Without this one shelf (usually the vault, whose notes are long) can fill every place."""
    pool = sorted(pool, key=lambda x: -x[0])
    firsts, seen = [], set()
    for item in pool:
        if item[1] not in seen and item[0] >= PROMOTE_AT and may_promote(item):
            seen.add(item[1])
            firsts.append(item)
    rest = [item for item in pool if not any(item is f for f in firsts)]
    return (firsts + rest)[:k]


def _keyword_pool(q, shelves, k, idx=None, docs=None, filters=None, links=False, project=None, info=None):
    """The keyword search's k results over several shelves, in the order it shows them, and the symptom lines:
    sections from guides/team/game and notes from the vault, merged by how much of the query each matches (scores
    from different shelves are not comparable, coverage is). filters (normalize_filters) keep only the notes whose
    labels pass; links adds up to two notes the first results link to (_linked_pool). A superseded note's score is
    scaled down (demote) and it is never moved up as its shelf's best; info (a dict), when given, gets the superseded
    map ("sup")."""
    terms = query_terms(q)
    pool = []
    symptoms = []
    use_vault = "vault" in shelves and os.path.isdir(KB)
    sec_shelves = [s for s in shelves if s != "vault"]
    if use_vault:
        idx = idx or load_index()
    if sec_shelves:
        docs = section_corpus() if docs is None else docs
    sup = superseded_for(idx if use_vault else None, docs if sec_shelves else None, project)
    if info is not None:
        info["sup"] = sup

    def old(d):
        return replaced_by(sup, d)
    if use_vault:
        res, symptoms = search(q, (max(k * 8, 40) if filters else k) + (10 if sup else 0), idx)
        if filters:
            res = [(s, d) for s, d in res if labels_match(d.get("labels"), filters)]
        top = res[0][0] if res else 1  # the best before scaling: a scaled one stays low
        if sup:
            res = [(s, x[1]) for s, x in demote(res, lambda x: x[0], lambda x: old(x[1]), lambda x: x[1]["title"])]
        for s, d in res[:k]:
            pool.append((_coverage(terms, d["tf"], FIELDS) * s / top, "vault", d))
    if sec_shelves:
        if filters:
            docs = [d for d in docs if labels_match(d.get("labels"), filters)]
        hits = search_sections(q, sec_shelves, max(k, 8) * len(sec_shelves), docs)
        if sup:
            hits = [(s, x[1], x[2]) for s, x in demote(hits, lambda x: x[0], lambda x: old(x[2]),
                                                       lambda x: x[2]["title"])]
        taken = Counter()
        for s, cov, d in hits:  # each shelf's own best k, by score (already scaled to 1.0 at the best)
            if taken[d["shelf"]] < k:
                taken[d["shelf"]] += 1
                pool.append((cov * s, d["shelf"], d))
    if sup:  # already scaled: the replacement wins a tie and comes first (interleave keeps this order)
        pool = [x for _, x in demote(pool, lambda x: x[0], lambda x: old(x[2]), lambda x: x[2]["title"], 1.0)]
    pool = interleave(pool, k, lambda item: (item[1] not in PERSONAL or personal_match(q, item[2]))
                      and not (sup and old(item[2])))
    if links:
        pool = _linked_pool(q, pool, shelves, k, idx if "vault" in shelves else None,
                            docs if sec_shelves else None, filters)
    return pool, symptoms


LINKED_MAX = 2  # linked results added to one answer at most
DIRECT_KEPT = 3  # ... and never in place of one of the first three direct results
LINK_COVER = 0.6  # a linked note qualifies when its best part holds at least this share of the query's own words
LINK_FIELDS = ("title", "tags", "tldr", "heads")  # ... a vault note's in these (its long body holds most words)
LINK_CANDIDATES = 24  # linked notes looked at per search at most


def place_linked(direct, linked, k):
    """The k results to show: the first DIRECT_KEPT direct ones, then up to LINKED_MAX linked ones, then the rest of
    the direct ones."""
    head = direct[:DIRECT_KEPT]
    room = min(LINKED_MAX, k - len(head))
    return (head + list(linked)[:max(0, room)] + direct[DIRECT_KEPT:])[:k]


def _linked_pool(q, pool, shelves, k, idx, docs, filters=None):
    """The keyword search's link following: the [[links]] in the first results' notes, and of the notes they name
    (on the searched shelves, not already shown, passing the same filters: `docs` is already filtered) the best part
    of each that holds most of the query's own words (LINK_COVER; synonyms do not count, and a vault note counts only
    its title, tags, TL;DR and headings). Linked items carry a 4th element: the linking title."""
    if k <= DIRECT_KEPT or len(pool) < 1:
        return pool
    terms = Counter({t: w for t, w in query_terms(q).items() if w >= 1})
    have = {(item[1], item[2]["title"]) for item in pool}
    vault_by = {d["title"].casefold(): d for d in (idx or {}).get("docs", [])} if idx else {}
    notes, by_name = {}, {}
    for d in docs or ():
        if d["shelf"] not in shelves:
            continue
        key = (d["shelf"], d["title"])
        notes.setdefault(key, []).append(d)
        for name in {d["title"].casefold(), os.path.basename(d["title"]).casefold(),
                     (d.get("label") or "").casefold()} - {""}:
            if name.endswith(".md"):
                name = name[:-3]
            by_name.setdefault(name, [])
            if key not in by_name[name]:
                by_name[name].append(key)
    cands, src = [], {}
    for _, shelf, d in [x[:3] for x in pool[:DIRECT_KEPT]]:
        if shelf == "vault":
            targets = d.get("links") or []
        else:
            own = d.get("links") or []
            targets = own + [t for x in notes.get((shelf, d["title"]), ()) for t in x.get("links") or ()
                             if t not in own]
        for t in targets:
            keys = ([("vault", vault_by[t]["title"])] if t in vault_by else []) + by_name.get(t, [])
            for key in keys:
                if key not in have and key not in src:
                    src[key] = d["title"]
                    cands.append(key)
        if len(cands) >= LINK_CANDIDATES:
            break
    found = []
    for key in cands[:LINK_CANDIDATES]:
        if key[0] == "vault":
            d = vault_by.get(key[1].casefold())
            if d is None or not labels_match(d.get("labels"), filters):
                continue
            found.append((_coverage(terms, d["tf"], LINK_FIELDS), "vault", d, src[key]))
        else:
            t = pquery_terms(q) if key[0] in PERSONAL else squery_terms(q)
            t = Counter({x: w for x, w in t.items() if w >= 1})
            best = max(((_coverage(t, x["tf"], SECTION_FIELDS), x["own"], x) for x in notes.get(key, ())),
                       key=lambda c: (c[0], c[1]), default=None)
            if best:
                found.append((best[0], key[0], best[2], src[key]))
    linked = sorted((f for f in found if f[0] >= LINK_COVER), key=lambda f: -f[0])
    return place_linked(pool, linked, k) if linked else pool


def keyword_ranking(q, shelves, depth, idx=None, docs=None, project=None):
    """The keyword search's own order for q, as (shelf, title, section index), the index None for a vault note
    (the vault search ranks whole notes). The knowledge index fuses this with its dense ranking. idx and docs: the
    vault index and the section docs (section_corpus) when the caller keeps them loaded; project: the game's folder
    ("" for none, None: RBXKIT_PROJECT), for the superseded notes."""
    if list(shelves) == ["vault"]:  # the vault on its own ranks as its own search does
        if not os.path.isdir(KB):
            return []
        idx = idx or load_index()
        sup = superseded_for(idx, None, project)
        res = search(q, depth + (10 if sup else 0), idx)[0]
        if sup:
            res = [x for _, x in demote(res, lambda x: x[0], lambda x: replaced_by(sup, x[1]))]
        return [("vault", d["title"], None) for _, d in res[:depth]]
    pool, _ = _keyword_pool(q, shelves, depth, idx, docs, project=project)
    return [(shelf, d["title"], None if shelf == "vault" else d["i"]) for _, shelf, d in pool]


def combined_search(q, shelves, k, filters=None):
    """Text for a search over several shelves (`_keyword_pool`), with the notes linked from the first results."""
    terms = query_terms(q)
    info = {}
    pool, symptoms = _keyword_pool(q, shelves, k, None, None, filters, links=True, info=info)
    if not pool:
        return "No match on %s%s. Try other words%s." % (
            ", ".join(shelves), " with those filters" if filters else "",
            ", or action 'list' to browse the vault" if "vault" in shelves else "")
    out = ['Results for "%s" (%s):' % (q, ", ".join(shelves))]
    files = {}
    for rank, item in enumerate(pool, 1):
        _, shelf, d = item[:3]
        linked = item[3] if len(item) > 3 else None
        replaced = replaced_by(info.get("sup"), d)
        if shelf == "vault":
            out += _vault_note_lines(rank, d, linked, replaced)
        else:
            key = (shelf, d["title"])
            if key not in files:
                try:
                    files[key] = parse_sections(_reader_text(d))
                except OSError:
                    files[key] = None
            p = files[key]
            s = p["sections"][d["i"]] if p and d["i"] < len(p["sections"]) else None
            out += _section_lines(rank, shelf, d, p, s, terms, linked=linked, replaced=replaced)
    return _results_tail(out, q, shelves, symptoms)


# ------------------------------------------------------------------ the lines of one search result (both searches)
def _tail(labels, linked=None, lesson=False, replaced=None):
    """The end of a result's first line: its labels, the note that links to it when it was found that way, and the
    note that replaces it when it is superseded."""
    lab = label_text(labels) or ("[lesson]" if lesson else "")
    return (("  " + lab if lab else "") + ("  (linked from %s)" % linked if linked else "")
            + ("  (replaced by %s)" % replaced if replaced else ""))


def _vault_note_lines(rank, d, linked=None, replaced=None):
    """A vault note found by the keyword search: its TL;DR bullets, its sections and the arguments to read one."""
    out = []
    bullets, _ = tldr_parts(d)
    hints = _vault_section_hint(d)
    out.append("%d. [vault] %s  (note, %d KB)%s" % (rank, d["title"], d["kb"],
                                                     _tail(d.get("labels"), linked, replaced=replaced)))
    out += ["   " + b for b in bullets[:2]]
    if hints:
        out.append("   Sections: " + " · ".join(hints))
    args = {"action": "read", "shelf": "vault", "title": d["title"], "heading": hints[0] if hints else "<heading>"}
    if not hints:
        out.append('   sections: {"action": "sections", "shelf": "vault", "title": %s}' % json.dumps(d["title"], ensure_ascii=False))
    out.append("   read: " + json.dumps(args, ensure_ascii=False))
    return out


def guide_excerpt_tag(title):
    """The line under every piece of a guide that search or a section read hands out: a piece is for finding the
    guide, and the agent loads the whole guide before acting on it."""
    return ("[Excerpt from guide: %s — do not act on this snippet; call rbx_guide with topic `%s` for the full "
            "procedure first.]" % (title, title))


def _section_lines(rank, shelf, d, p, s, terms, args=None, lines=None, linked=None, replaced=None):
    """One section: rank, shelf, its labels, title › heading, size, a snippet (or the given passage lines) and the
    arguments that read exactly it; a result found through a link says which note links to it, a superseded one
    which note replaces it."""
    out = []
    where = d["heading"] if not d["parents"] else " › ".join(d["parents"][-1:] + [d["heading"]])
    label = " (%s)" % d["label"] if d["label"] and d["label"] != d["title"] else ""
    out.append("%d. [%s] %s%s › %s  (%s)%s" % (rank, shelf, d["title"], label, where, size_text(d["size"]),
                                               _tail(d.get("labels"), linked, d.get("lesson"), replaced)))
    if lines is not None:
        out += ["   " + x for x in lines]
    elif s:
        text = p["body"][s["start"]:s["own_end"]]
        if not s["level"] and p["description"]:
            text = p["description"] + "\n" + text
        out += ["   " + x for x in snippet(text, terms)]
    if shelf == "guides":
        out.append("   " + guide_excerpt_tag(d["title"]))
    if args is None:
        args = {"action": "read", "shelf": shelf, "title": d["title"], "heading": heading_arg(p, s) if s else d["heading"]}
    out.append("   read: " + json.dumps(args, ensure_ascii=False))
    return out


def _results_tail(out, q, shelves, symptoms):
    if "vault" in shelves:
        notes = api_notes(q)
        if notes:
            out.append("API status (snapshot):")
            out += notes
        if symptoms:
            out.append("Symptoms (symptom | cause | fix | note):")
            out += ["- " + s["line"] for s in symptoms]
    out.append("\nNext: rbx_vault action 'read' with the arguments shown, for ONE section at a time.")
    return "\n".join(out)


# ------------------------------------------------------------------ search through the knowledge index (RAG)
RAG_TIMEOUT = 3.0  # seconds; a slower answer is not waited for (the keyword search answers instead)
# calls to the knowledge index stay on this Mac: never through an HTTP proxy (the environment's or the system's),
# which would otherwise receive the token and the query
_LOOPBACK = urllib.request.build_opener(urllib.request.ProxyHandler({}))
_LOOPBACK_HOSTS = ("127.0.0.1", "localhost", "[::1]")


def rag_endpoint():
    """(url, token) from the endpoint file the app writes while the knowledge index is ready (its path is in
    FRIDAE_RAG_FILE), read fresh on every search; None when there is none or it does not read."""
    path = (os.environ.get("FRIDAE_RAG_FILE") or "").strip()
    if not path:
        return None
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        url = str(data.get("url") or "").strip().rstrip("/")
        token = str(data.get("token") or "").strip()
    except (OSError, ValueError, AttributeError, UnicodeDecodeError):
        return None
    host = url[len("http://"):].split("/", 1)[0].rsplit(":", 1)[0] if url.startswith("http://") else ""
    if not token or host not in _LOOPBACK_HOSTS:
        return None
    return url, token


PASSAGE_CHARS = 700  # one hit's passage at most (about 175 tokens)
ANSWER_CHARS = 3200  # every passage of one answer together at most (about 800 tokens); the rest show a snippet


CONTEXT_MARK = "Context: "  # the knowledge index's context line, second in a chunk (rbxrag.chunks)


def passage(text, budget):
    """The lines of a matched chunk (its own text, without the title › heading line and the context line the index
    puts first), up to budget characters: the passage an agent reads instead of the whole note."""
    body = text.split("\n", 1)[1] if "\n" in text else text
    if body.startswith(CONTEXT_MARK):
        body = body.split("\n", 1)[1] if "\n" in body else ""
    out, used, fence = [], 0, False
    for line in body.split("\n"):
        st = line.strip()
        if _FENCE.match(st):
            fence = not fence
            continue
        if not st or st.startswith("<!--") or re.match(r"^\|?[\s:|-]+\|?$", st):
            continue
        st = " ".join(st.strip("|").replace(" | ", " · ").split()) if not fence else st
        if used + len(st) > budget:
            room = budget - used
            if room > 40:
                out.append(st[:room - 1].rsplit(" ", 1)[0] + "…")
            break
        out.append(st)
        used += len(st)
    return out


def _rag_hit_lines(rank, hit, terms, budget=0):
    """The lines for one hit of the knowledge index, or None when the file or section is no longer there. With a
    budget (characters) and the hit's chunk text, the matched passage is shown instead of a snippet."""
    shelf, title, path = hit.get("shelf"), hit.get("title"), hit.get("path")
    if shelf not in SHELVES or not title or not isinstance(path, str) or not path.endswith(".md"):
        return None
    try:
        text = _reader_text({"shelf": shelf, "path": path})
    except (OSError, UnicodeDecodeError):
        return None
    p = parse_sections(text)
    i, heading = hit.get("i"), hit.get("heading")
    secs = p["sections"]
    s = secs[i] if isinstance(i, int) and 0 <= i < len(secs) and secs[i]["heading"] == heading else None
    if s is None:  # the file changed since it was indexed: the same heading elsewhere, else nothing
        s = next((x for x in secs if x["heading"] == heading), None)
    if s is None:
        return None
    label = _team_label(title) if shelf == "team" else (p["label"] or title)
    lesson = shelf in PERSONAL and (p["fm"].get("kind") == "lesson" or title in ("lessons.md", "studio-lessons"))
    labels = section_labels(note_labels(shelf, title, path, p["fm"], text, hit.get("project") or ""),
                            s["heading"], s["parents"])
    d = {"title": title, "label": label, "heading": s["heading"], "parents": s["parents"], "size": s_size(p, s),
         "lesson": lesson, "labels": labels}
    args = None
    if shelf == "vault":  # a vault read takes heading words of a ## or ### heading; without one it lists the sections
        args = {"action": "read", "shelf": "vault", "title": title}
        if s["level"] >= 2:
            args["heading"] = s["heading"]
    lines = None
    if budget > 0 and isinstance(hit.get("text"), str) and hit["text"].strip():
        lines = passage(hit["text"], budget) or None
    linked = hit.get("linked_from") if isinstance(hit.get("linked_from"), str) else None
    # the index says which note replaces it (rbxrag.engine); the file's own mark says so too
    replaced = hit.get("replaced_by") if isinstance(hit.get("replaced_by"), str) else None
    return _section_lines(rank, shelf, d, p, s, terms, args, lines, linked, replaced or supersession(text)[1] or None)


def _rag_hits(end, q, shelves, k, filters=None):
    """The knowledge index's hits for q (a non-empty list), or None when it answers anything else. Raises on a failed
    call (the callers turn every failure into the keyword search)."""
    url, token = end
    req_body = {"q": q, "shelves": list(shelves), "k": k, "project": project_dir() or ""}
    if filters:
        req_body["filters"] = filters
    body = json.dumps(req_body).encode()
    req = urllib.request.Request(url + "/query", data=body, headers={
        "Content-Type": "application/json", "Authorization": "Bearer " + token})
    with _LOOPBACK.open(req, timeout=RAG_TIMEOUT) as r:
        if r.status != 200:
            return None
        hits = json.load(r).get("hits")
    return hits if isinstance(hits, list) and hits else None


def _close_quietly(e):
    if hasattr(e, "close"):  # an HTTP error answer holds its connection open
        try:
            e.close()
        except Exception:
            pass


def rag_search(q, shelves, k, filters=None):
    """Search text from the knowledge index service (its endpoint file, see rag_endpoint), in the same format as the
    keyword search, or None on any failure (not running, slower than 3 s, an error, a bad answer, no hits) so the
    caller answers with the keyword search instead."""
    end = rag_endpoint()
    if not end:
        return None
    try:
        hits = _rag_hits(end, q, shelves, k, filters)
        if hits is None:
            return None
        terms = query_terms(q)
        out = ['Results for "%s" (%s):' % (q, ", ".join(shelves))]
        rank, left = 0, ANSWER_CHARS
        for hit in hits[:k]:
            budget = min(PASSAGE_CHARS, left)
            lines = _rag_hit_lines(rank + 1, hit, terms, budget if budget > 80 else 0) if isinstance(hit, dict) else None
            if lines:
                rank += 1
                out += lines
                if budget > 80 and isinstance(hit.get("text"), str):
                    left -= min(budget, sum(len(x) - 3 for x in lines[1:-1]))
        if not rank:
            return None
        symptoms = search(q, k, load_index())[1] if "vault" in shelves and os.path.isdir(KB) else []
        return _results_tail(out, q, shelves, symptoms).replace(
            "\nNext: rbx_vault action 'read' with the arguments shown, for ONE section at a time.",
            "\nThe lines under each result are its matching passage.\nNext: rbx_vault action 'read' with the "
            "arguments shown, only if you need more, for ONE section at a time.")
    except Exception as e:  # never an error from here: the keyword search is the answer
        _close_quietly(e)
        return None


# ------------------------------------------------------------------ several questions at once (the app's Librarian)
# The swarm's Librarian answers a board question without an agent: the app splits it into its parts and asks for
# all of them in one call (`rbx lookup`), then has one cheap model write the answer from these passages.
LOOKUP_K = 3  # passages per part
LOOKUP_MAX = 6  # parts per call
LOOKUP_CHARS = 600  # one passage at most


def _shelves_now():
    return [s for s in SHELVES if s != "game" or project_dir()]


def _rag_passages(end, q, shelves, k):
    """[{shelf, title, heading, text}] from the knowledge index, or None on any failure or no usable hit."""
    try:
        hits = _rag_hits(end, q, shelves, k)
    except Exception as e:
        _close_quietly(e)
        return None
    out = []
    for hit in (hits or [])[:k]:
        if not isinstance(hit, dict) or hit.get("shelf") not in SHELVES or not hit.get("title"):
            continue
        text = hit.get("text") if isinstance(hit.get("text"), str) else ""
        lines = passage(text, LOOKUP_CHARS) if text.strip() else []
        if lines:
            title = _team_label(hit["title"]) if hit["shelf"] == "team" else str(hit["title"])
            replaced = hit.get("replaced_by") if isinstance(hit.get("replaced_by"), str) else None
            out.append({"shelf": hit["shelf"], "title": title, "heading": str(hit.get("heading") or ""),
                        "text": replaced_mark(replaced) + " ".join(lines)})
    return out or None


def replaced_mark(replaced):
    """What a superseded note's passage starts with for the Librarian's writer."""
    return "(Outdated: replaced by '%s'; prefer that note.) " % replaced if replaced else ""


def _cut(text, n=LOOKUP_CHARS):
    text = " ".join(text.split())
    return text if len(text) <= n else text[:n - 1].rsplit(" ", 1)[0] + "…"


def _keyword_passages(q, shelves, k, idx, docs):
    """[{shelf, title, heading, text}] from the keyword search: a vault note's TL;DR and its lines that match best, a
    section's matching lines."""
    terms = query_terms(q)
    info = {}
    pool, _ = _keyword_pool(q, shelves, k, idx, docs, info=info)
    out = []
    for _, shelf, d in pool:
        mark = replaced_mark(replaced_by(info.get("sup"), d))
        if shelf == "vault":
            bullets, _ = tldr_parts(d)
            try:
                lines = snippet(split_front(read(os.path.join(KB, d["path"])))[1], terms, n=3, width=200)
            except (OSError, UnicodeDecodeError):
                lines = []
            text = " ".join([b[2:] if b.startswith("- ") else b for b in bullets[:2]] + lines)
            out.append({"shelf": "vault", "title": d["title"], "heading": "", "text": mark + _cut(text)})
            continue
        try:
            p = parse_sections(_reader_text(d))
        except (OSError, UnicodeDecodeError):
            continue
        s = p["sections"][d["i"]] if d["i"] < len(p["sections"]) else None
        if not s:
            continue
        lines = snippet(p["body"][s["start"]:s["own_end"]], terms, n=4, width=200)
        out.append({"shelf": shelf, "title": d.get("label") or d["title"], "heading": d["heading"],
                    "text": mark + _cut(" ".join(lines))})
    return out


def _lookup_one(q, shelves, k, end, corpora):
    """One part's passages: the knowledge index when it answers, else the keyword search (corpora() loads its
    indexes once for every part)."""
    hits = _rag_passages(end, q, shelves, k) if end else None
    if hits is not None:
        return {"query": q, "via": "rag", "hits": hits}
    idx, docs = corpora()
    return {"query": q, "via": "keyword", "hits": _keyword_passages(q, shelves, k, idx, docs)}


def lookup(queries, k=LOOKUP_K):
    """Passages for several questions at once, every part searched at the same time: [{query, via, hits:
    [{shelf, title, heading, text}]}] in the order asked, or a part's {"error"} when its search failed."""
    import threading
    from concurrent.futures import ThreadPoolExecutor
    queries = [" ".join(str(q).split()) for q in queries if str(q).strip()][:LOOKUP_MAX]
    if not queries:
        return []
    k = max(1, min(int(k), 8))
    shelves, end = _shelves_now(), rag_endpoint()
    loaded, lock = {}, threading.Lock()

    def corpora():
        with lock:
            if not loaded:
                loaded["idx"] = load_index() if os.path.isdir(KB) else None
                loaded["docs"] = section_corpus()
            return loaded["idx"], loaded["docs"]

    def one(q):
        try:
            return _lookup_one(q, shelves, k, end, corpora)
        except Exception as e:  # one part failing never loses the others
            return {"query": q, "via": "", "hits": [], "error": "%s: %s" % (type(e).__name__, e)}

    with ThreadPoolExecutor(max_workers=len(queries)) as ex:
        return list(ex.map(one, queries))


def _vault_superseded():
    """True when a note of the vault is superseded (the vault search alone then answers through combined_search,
    which ranks and marks it)."""
    if not os.path.isdir(KB):
        return False
    idx = load_index()
    sup = superseded_for(idx, None, None)
    return bool(sup) and any(d["title"].casefold() in sup for d in idx["docs"])


def _list(folder):
    root = os.path.join(KB, folder) if folder else KB
    if not os.path.isdir(root) or not os.path.abspath(root).startswith(os.path.abspath(KB)):
        raise KitError("No vault folder called %r. Call rbx_vault with action 'list' and no folder for the sections." % folder)
    dirs = sorted(d for d in os.listdir(root) if os.path.isdir(os.path.join(root, d)) and not d.startswith((".", "_")))
    notes = sorted(f[:-3] for f in os.listdir(root) if f.endswith(".md"))
    out = [("Folders: " + ", ".join(dirs)) if dirs else ""]
    out += ["Notes:"] + ["- " + n for n in notes] if notes else []
    return "\n".join(x for x in out if x) or "(empty)"


def _shelf_arg(a):
    shelf = str(a.get("shelf") or "all").strip().lower()
    if shelf not in SHELVES + ("all",):
        raise KitError("shelf must be one of: all, %s." % ", ".join(SHELVES))
    return shelf


def _vault_call(fn):
    """Run one of the vault's printing functions and return what it printed (library code never prints to stdout)."""
    import contextlib
    import io
    if not os.path.isdir(KB):
        raise KitError("The LihFolk vault is not installed with this copy of rbxkit.")
    buf = io.StringIO()
    try:
        with contextlib.redirect_stdout(buf):
            fn()
    except SystemExit as e:
        raise KitError(str(e.code) if e.code not in (None, 0) else buf.getvalue() or "Not found.")
    return buf.getvalue().strip()


NOTE_ACTIONS = ("note", "notes", "forget", "rule_edit", "upkeep")


def run(action, a):
    """rbx_vault: search (query, shelf, k), read (title, heading, shelf), sections (title, shelf), list (folder), api
    (names), and the user's notes: note, notes, forget, rule_edit (rbxnotes.py)."""
    if action in NOTE_ACTIONS:
        import rbxnotes
        return rbxnotes.run(action, a)
    if action == "list":
        if not os.path.isdir(KB):
            raise KitError("The LihFolk vault is not installed with this copy of rbxkit.")
        return _list((a.get("folder") or "").strip().strip("/"))
    if action == "api":
        def api():
            sys.argv = ["vault", "--api"] + str(a.get("names") or a.get("query") or "").split()
            main()
        return _vault_call(api) or "No API names given."
    shelf = _shelf_arg(a)
    if action in ("read", "sections"):
        title = (a.get("title") or "").strip()
        if not title:
            raise KitError("%s needs a title (from a search)." % action)
        heading = ((a.get("heading") or "").strip() or None) if action == "read" else None
        docs = section_corpus() if shelf != "vault" else []
        where, key = resolve(title, None if shelf == "all" else shelf, docs)
        if where == "vault":
            return _vault_call(lambda: print_section(key, heading, load_index()))
        return read_file_section(where, key, heading, docs)
    if action != "search":
        raise KitError("action must be search, read, sections, list, api, note, notes, forget, rule_edit or upkeep.")
    q = (a.get("query") or "").strip()
    if not q:
        raise KitError("search needs a query: the task or bug in plain words.")
    kk = a.get("k") or a.get("count")
    try:
        k = max(1, min(int(kk), 8)) if kk else (3 if shelf == "vault" else 5)
    except (TypeError, ValueError):
        raise KitError("k must be a whole number from 1 to 8.")
    shelves = list(SHELVES) if shelf == "all" else [shelf]
    if shelf == "game" and not project_dir():
        raise KitError("This chat has no game project (RBXKIT_PROJECT), so there are no game notes to search.")
    if shelf == "all" and not project_dir():
        shelves.remove("game")
    filters = normalize_filters(a, project_dir())
    text = rag_search(q, shelves, k, filters)  # the knowledge index, when the app runs it; None: the keyword search
    if text is not None:
        return text
    if shelf == "vault" and not filters and not _vault_superseded():  # the vault on its own answers as it always has
        def vault_search():
            sys.argv = ["vault", "-k", str(k), q]
            main()
        text = _vault_call(vault_search) or "No match. Try other words, or action 'list' to browse the sections."
        return text + "\n\nNext: rbx_vault action 'read' with a title and heading words for one section (not the whole note)."
    return combined_search(q, shelves, k, filters)
