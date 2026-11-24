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
    return {"title": title, "path": rel, "tags": tags, "tldr": tldr, "summary": summary,
            "f": {"title": tokens(title), "tags": tokens(" ".join(tags)), "tldr": tokens(tldr),
                  "heads": tokens(" ".join(heads)), "body": tokens(plain)},
            "kb": round(len(text.encode("utf-8")) / 1024)}


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
INDEX_FORMAT = 1  # bump when the section index's shape changes
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


_PERSONAL_CACHE = {}  # (shelf, title, path, rel, label_hint) -> ((mtime_ns, size), entries), for the user's shelves


def _section_docs(shelf, title, path, rel, label_hint=""):
    """Index entries for one file: one per section. The user's own shelves are read on every search, so their
    entries are kept per file and parsed again only when the file changes."""
    if shelf in PERSONAL:
        key = (shelf, title, path, rel, label_hint)
        try:
            st = os.stat(path)
            sig = (st.st_mtime_ns, st.st_size)
        except OSError:
            sig = None
        hit = _PERSONAL_CACHE.get(key)
        if sig is not None and hit and hit[0] == sig:
            return hit[1]
        out = _parse_section_docs(shelf, title, path, rel, label_hint)
        if sig is not None:
            if len(_PERSONAL_CACHE) > 50000:
                _PERSONAL_CACHE.clear()
            _PERSONAL_CACHE[key] = (sig, out)
        return out
    return _parse_section_docs(shelf, title, path, rel, label_hint)


def _parse_section_docs(shelf, title, path, rel, label_hint=""):
    try:
        text = read(path)
    except (OSError, UnicodeDecodeError):
        return []
    p = parse_sections(text)
    label = p["label"] or label_hint or title
    tok = ptokens if shelf in PERSONAL else stokens
    # on the user's own shelves, what an agent learned (a lesson) is shown apart from what the user said (a note)
    lesson = shelf in PERSONAL and (p["fm"].get("kind") == "lesson" or title in ("lessons.md", "studio-lessons"))
    out = []
    for i, s in enumerate(p["sections"]):
        own = p["body"][s["start"]:s["own_end"]]
        if s["level"]:
            own = own.split("\n", 1)[1] if "\n" in own else ""
        heads = " ".join(s["parents"] + ([s["heading"]] if s["level"] else [label]))
        f = {"title": tok(title.replace("-", " ") + " " + label), "tldr": tok(p["description"]),
             "heads": tok(heads), "body": tok(_code_capped(own))}
        tf = {k: Counter(v) for k, v in f.items()}
        out.append({"shelf": shelf, "title": title, "label": label, "rel": rel, "path": path, "i": i,
                    "heading": s["heading"], "parents": s["parents"], "level": s["level"],
                    "size": len(p["body"][s["start"]:s["end"]].encode("utf-8")),
                    "own": len(own.strip()), "tf": tf, "len": {k: sum(c.values()) for k, c in tf.items()},
                    "lesson": lesson})
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
            docs += _section_docs("team", t, p, p, _team_label(t))
    for t, p in game_files(project):
        docs += _section_docs("game", t, p, p)
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
    return "%s › %s (%s)\n\n%s" % (head, s["heading"], size_text(s_size(p, s)), cap_text(body, sub, key, shelf))


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


def _keyword_pool(q, shelves, k, idx=None, docs=None):
    """The keyword search's k results over several shelves, in the order it shows them, and the symptom lines:
    sections from guides/team/game and notes from the vault, merged by how much of the query each matches (scores
    from different shelves are not comparable, coverage is)."""
    terms = query_terms(q)
    pool = []
    symptoms = []
    if "vault" in shelves and os.path.isdir(KB):
        idx = idx or load_index()
        res, symptoms = search(q, k, idx)
        top = res[0][0] if res else 1
        for s, d in res:
            pool.append((_coverage(terms, d["tf"], FIELDS) * s / top, "vault", d))
    sec_shelves = [s for s in shelves if s != "vault"]
    if sec_shelves:
        docs = section_corpus() if docs is None else docs
        hits = search_sections(q, sec_shelves, max(k, 8) * len(sec_shelves), docs)
        taken = Counter()
        for s, cov, d in hits:  # each shelf's own best k, by score (already scaled to 1.0 at the best)
            if taken[d["shelf"]] < k:
                taken[d["shelf"]] += 1
                pool.append((cov * s, d["shelf"], d))
    return interleave(pool, k, lambda item: item[1] not in PERSONAL or personal_match(q, item[2])), symptoms


def keyword_ranking(q, shelves, depth, idx=None, docs=None):
    """The keyword search's own order for q, as (shelf, title, section index), the index None for a vault note
    (the vault search ranks whole notes). The knowledge index fuses this with its dense ranking. idx and docs: the
    vault index and the section docs (section_corpus) when the caller keeps them loaded."""
    if list(shelves) == ["vault"]:  # the vault on its own ranks as its own search does
        if not os.path.isdir(KB):
            return []
        return [("vault", d["title"], None) for _, d in search(q, depth, idx or load_index())[0]]
    pool, _ = _keyword_pool(q, shelves, depth, idx, docs)
    return [(shelf, d["title"], None if shelf == "vault" else d["i"]) for _, shelf, d in pool]


def combined_search(q, shelves, k):
    """Text for a search over several shelves (`_keyword_pool`)."""
    terms = query_terms(q)
    pool, symptoms = _keyword_pool(q, shelves, k)
    if not pool:
        return "No match on %s. Try other words%s." % (", ".join(shelves), ", or action 'list' to browse the vault" if "vault" in shelves else "")
    out = ['Results for "%s" (%s):' % (q, ", ".join(shelves))]
    files = {}
    for rank, (_, shelf, d) in enumerate(pool, 1):
        if shelf == "vault":
            out += _vault_note_lines(rank, d)
        else:
            key = (shelf, d["title"])
            if key not in files:
                try:
                    files[key] = parse_sections(_reader_text(d))
                except OSError:
                    files[key] = None
            p = files[key]
            s = p["sections"][d["i"]] if p and d["i"] < len(p["sections"]) else None
            out += _section_lines(rank, shelf, d, p, s, terms)
    return _results_tail(out, q, shelves, symptoms)


# ------------------------------------------------------------------ the lines of one search result (both searches)
def _vault_note_lines(rank, d):
    """A vault note found by the keyword search: its TL;DR bullets, its sections and the arguments to read one."""
    out = []
    bullets, _ = tldr_parts(d)
    hints = _vault_section_hint(d)
    out.append("%d. [vault] %s  (note, %d KB)" % (rank, d["title"], d["kb"]))
    out += ["   " + b for b in bullets[:2]]
    if hints:
        out.append("   Sections: " + " · ".join(hints))
    args = {"action": "read", "shelf": "vault", "title": d["title"], "heading": hints[0] if hints else "<heading>"}
    if not hints:
        out.append('   sections: {"action": "sections", "shelf": "vault", "title": %s}' % json.dumps(d["title"], ensure_ascii=False))
    out.append("   read: " + json.dumps(args, ensure_ascii=False))
    return out


def _section_lines(rank, shelf, d, p, s, terms, args=None, lines=None):
    """One section: rank, shelf, title › heading, size, a snippet (or the given passage lines) and the arguments
    that read exactly it."""
    out = []
    where = d["heading"] if not d["parents"] else " › ".join(d["parents"][-1:] + [d["heading"]])
    label = " (%s)" % d["label"] if d["label"] and d["label"] != d["title"] else ""
    kind = " [lesson]" if d.get("lesson") else ""
    out.append("%d. [%s]%s %s%s › %s  (%s)" % (rank, shelf, kind, d["title"], label, where, size_text(d["size"])))
    if lines is not None:
        out += ["   " + x for x in lines]
    elif s:
        text = p["body"][s["start"]:s["own_end"]]
        if not s["level"] and p["description"]:
            text = p["description"] + "\n" + text
        out += ["   " + x for x in snippet(text, terms)]
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


def passage(text, budget):
    """The lines of a matched chunk (its own text, without the title › heading line the index puts first), up to
    budget characters: the passage an agent reads instead of the whole note."""
    body = text.split("\n", 1)[1] if "\n" in text else text
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
        p = parse_sections(_reader_text({"shelf": shelf, "path": path}))
    except (OSError, UnicodeDecodeError):
        return None
    i, heading = hit.get("i"), hit.get("heading")
    secs = p["sections"]
    s = secs[i] if isinstance(i, int) and 0 <= i < len(secs) and secs[i]["heading"] == heading else None
    if s is None:  # the file changed since it was indexed: the same heading elsewhere, else nothing
        s = next((x for x in secs if x["heading"] == heading), None)
    if s is None:
        return None
    label = _team_label(title) if shelf == "team" else (p["label"] or title)
    lesson = shelf in PERSONAL and (p["fm"].get("kind") == "lesson" or title in ("lessons.md", "studio-lessons"))
    d = {"title": title, "label": label, "heading": s["heading"], "parents": s["parents"], "size": s_size(p, s),
         "lesson": lesson}
    args = None
    if shelf == "vault":  # a vault read takes heading words of a ## or ### heading; without one it lists the sections
        args = {"action": "read", "shelf": "vault", "title": title}
        if s["level"] >= 2:
            args["heading"] = s["heading"]
    lines = None
    if budget > 0 and isinstance(hit.get("text"), str) and hit["text"].strip():
        lines = passage(hit["text"], budget) or None
    return _section_lines(rank, shelf, d, p, s, terms, args, lines)


def _rag_hits(end, q, shelves, k):
    """The knowledge index's hits for q (a non-empty list), or None when it answers anything else. Raises on a failed
    call (the callers turn every failure into the keyword search)."""
    url, token = end
    body = json.dumps({"q": q, "shelves": list(shelves), "k": k, "project": project_dir() or ""}).encode()
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


def rag_search(q, shelves, k):
    """Search text from the knowledge index service (its endpoint file, see rag_endpoint), in the same format as the
    keyword search, or None on any failure (not running, slower than 3 s, an error, a bad answer, no hits) so the
    caller answers with the keyword search instead."""
    end = rag_endpoint()
    if not end:
        return None
    try:
        hits = _rag_hits(end, q, shelves, k)
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
            out.append({"shelf": hit["shelf"], "title": title, "heading": str(hit.get("heading") or ""),
                        "text": " ".join(lines)})
    return out or None


def _cut(text, n=LOOKUP_CHARS):
    text = " ".join(text.split())
    return text if len(text) <= n else text[:n - 1].rsplit(" ", 1)[0] + "…"


def _keyword_passages(q, shelves, k, idx, docs):
    """[{shelf, title, heading, text}] from the keyword search: a vault note's TL;DR and its lines that match best, a
    section's matching lines."""
    terms = query_terms(q)
    pool, _ = _keyword_pool(q, shelves, k, idx, docs)
    out = []
    for _, shelf, d in pool:
        if shelf == "vault":
            bullets, _ = tldr_parts(d)
            try:
                lines = snippet(split_front(read(os.path.join(KB, d["path"])))[1], terms, n=3, width=200)
            except (OSError, UnicodeDecodeError):
                lines = []
            text = " ".join([b[2:] if b.startswith("- ") else b for b in bullets[:2]] + lines)
            out.append({"shelf": "vault", "title": d["title"], "heading": "", "text": _cut(text)})
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
                    "text": _cut(" ".join(lines))})
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


NOTE_ACTIONS = ("note", "notes", "forget", "rule_edit")


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
        raise KitError("action must be search, read, sections, list, api, note, notes, forget or rule_edit.")
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
    text = rag_search(q, shelves, k)  # the knowledge index, when the app runs it; None means the keyword search
    if text is not None:
        return text
    if shelf == "vault":  # the vault on its own answers exactly as it always has
        def vault_search():
            sys.argv = ["vault", "-k", str(k), q]
            main()
        text = _vault_call(vault_search) or "No match. Try other words, or action 'list' to browse the sections."
        return text + "\n\nNext: rbx_vault action 'read' with a title and heading words for one section (not the whole note)."
    return combined_search(q, shelves, k)
