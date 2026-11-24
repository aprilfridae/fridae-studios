"""Vault upkeep: keeping the user's notes accurate as they grow. No model is called and nothing leaves this computer:
the checks compare the notes' words.

- When a note is filed (rbx_vault 'note'), the notes in the same scope plus the notes for every game are checked
  first: a near-duplicate (the same title, or nearly the same words with no fact that differs) is not saved a second
  time, and a possible contradiction (a sentence on the same thing whose numbers, yes/no or API names differ) is
  shown side by side after saving. The calling agent judges; these only find the candidates.
- The sweep (rbx_vault 'upkeep', `rbx upkeep`) writes a Markdown report to <notes home>/upkeep/YYYY-MM-DD.md:
  near-duplicate clusters, possible contradictions, superseded notes still linked from current ones, old notes that
  newer notes on the same topic exist for, and broken [[links]]. It only reports: it never changes a note.

How two notes are compared (thresholds below):
  words      a note's title and body words, lower case and stemmed, without the common words (rbxvault.STOP);
             numbers and API names count as words too
  duplicate  the same title, or word sets with Jaccard >= DUP_JACCARD (and at least DUP_MIN_WORDS words each),
             when no pair of their sentences conflicts
  conflict   two sentences whose "frames" (their words less numbers, API names and yes/no words) share at least
             SENT_SHARED words with Jaccard >= SENT_JACCARD, and whose numbers differ (both have some), whose yes/no
             differs (an odd number of not/never/cannot/false... in one only), or whose API names differ (both have some)
  topic      two notes share at least TOPIC_SHARED of the older one's TOPIC_RARE rarest words and their word sets
             have Jaccard >= TOPIC_JACCARD
Notes of different games are never compared with each other (a game's note is compared with its own game's notes
and with the notes for every game). A superseded note is not checked for contradictions: it is already replaced.
"""
import math
import os
import re
import time
from collections import Counter, defaultdict

import rbxnotes
import rbxvault

DUP_JACCARD = 0.8
DUP_MIN_WORDS = 3
SENT_JACCARD = 0.6
SENT_SHARED = 2
TOPIC_RARE = 10
TOPIC_SHARED = 3
TOPIC_JACCARD = 0.25
STALE_DAYS = 180
SHOW_CONFLICTS = 3  # notes shown for one filing at most, ...
SHOW_PAIRS = 2  # ... with this many sentence pairs each
REPORT_MAX = 200  # entries per report section at most

_WORD = re.compile(r"[A-Za-z][A-Za-z0-9_]*(?:'[A-Za-z]+)?|\d+(?:[.,]\d+)*")
_API = re.compile(r"^(?:[A-Z][a-z0-9]+(?:[A-Z][A-Za-z0-9]*)+|[a-z]+[A-Z][A-Za-z0-9]*)$")  # UpdateAsync, getAsync
_DOTTED = re.compile(r"\b[A-Z][A-Za-z0-9]*[.:][A-Za-z][A-Za-z0-9]*\b")  # DataStoreService.GetDataStore, Part:Clone
_DATES = re.compile(r"\b\d{4}-\d{1,2}(?:-\d{1,2})?\b|\b\d{1,2}:\d{2}\b")
_NEGATORS = {"not", "no", "never", "cannot", "none", "nobody", "nothing", "nor", "neither", "false", "disabled",
             "dont", "doesnt", "didnt", "isnt", "arent", "wasnt", "werent", "cant", "wont", "shouldnt", "mustnt",
             "wouldnt", "couldnt", "hasnt", "havent"}
_POSITIVE = {"yes", "always", "true", "enabled"}
_FENCE = re.compile(r"^[ \t]{0,3}(```+|~~~+)")
_SPLIT = re.compile(r"(?<=[.!?;])\s+")


# ------------------------------------------------------------------ reading notes
class Note:
    __slots__ = ("title", "path", "scope", "root", "date", "words", "sents", "links", "supersedes", "superseded_by",
                 "body")

    def __repr__(self):
        return "Note(%r, %s)" % (self.title, self.scope)


_STEMS = {}  # word -> its stem, or "" for a common word


def _content(word):
    hit = _STEMS.get(word)
    if hit is None:
        low = word.lower()
        hit = "" if low in rbxvault.STOP or len(low) < 2 else rbxvault.stem(low)
        if len(_STEMS) > 500000:
            _STEMS.clear()
        _STEMS[word] = hit
    return hit or None


def facts(sentence):
    """(frame words, numbers, yes/no as 0 or 1, API names) of one sentence."""
    s = _DATES.sub(" ", sentence.replace("’", "'"))
    apis = set(m.group(0) for m in _DOTTED.finditer(s))
    s = _DOTTED.sub(" ", s)
    frame, nums, neg = set(), set(), 0
    for w in _WORD.findall(s):
        if w[0].isdigit():
            nums.add(w.replace(",", ""))
            continue
        low = w.lower()
        if low in _NEGATORS or low.endswith("n't"):
            neg += 1
            continue
        if low in _POSITIVE:
            continue
        if _API.match(w):
            apis.add(w)
            continue
        c = _content(w)
        if c:
            frame.add(c)
    return frozenset(frame), frozenset(nums), neg % 2, frozenset(apis)


def words(text):
    """A text's comparable words: content words (stemmed), numbers and API names."""
    out = set()
    for w in _WORD.findall(_DATES.sub(" ", text.replace("’", "'"))):
        if w[0].isdigit():
            out.add(w.replace(",", ""))
        elif _API.match(w):
            out.add(w.lower())
        else:
            c = _content(w)
            if c and w.lower() not in _NEGATORS:
                out.add(c)
    return frozenset(out)


def sentences(body):
    """The sentences of a note's body: list items, lines and their sentences; no headings, code or tables."""
    out, fence = [], False
    for line in body.splitlines():
        st = line.strip()
        if _FENCE.match(st):
            fence = not fence
            continue
        if fence or not st or st.startswith(("#", "|", "<!--", "---")):
            continue
        st = re.sub(r"^(?:>\s*)+", "", st)
        st = re.sub(r"^(?:[-*+]|\d+[.)])\s+", "", st)
        st = rbxvault._plain(st)
        out += [x.strip() for x in _SPLIT.split(st) if len(x.strip()) > 2]
    return out


def make_note(title, body, scope="all", root="", path=None, fm=None, date=""):
    n = Note()
    fm = fm or {}
    n.title, n.path, n.scope, n.root, n.body = title, path, scope, root, body
    n.date = date
    n.words = words(title + "\n" + body)
    n.sents = [(s, facts(s)) for s in sentences(body)]
    n.links = rbxvault.wikilinks(body)
    n.supersedes, n.superseded_by = [], ""
    return n


_CACHE = {}  # (path, scope, root) -> ((mtime_ns, size), Note)


def read_note(title, path, scope, root):
    """A note file as a Note (kept until the file changes), or None when it does not read."""
    try:
        st = os.stat(path)
    except OSError:
        return None
    key, sig = (path, scope, root), (st.st_mtime_ns, st.st_size)
    hit = _CACHE.get(key)
    if hit and hit[0] == sig and hit[1].title == title:
        return hit[1]
    try:
        text = rbxvault.read(path)
    except (OSError, UnicodeDecodeError):
        return None
    fm, body = rbxvault.split_front(text)
    body = re.sub(r"\A\s*#[ \t]+[^\n]*\n?", "", body, count=1)  # the H1 is the title
    date = rbxvault.note_labels("game" if scope == "game" else "yours", title, path, fm)["date"]
    n = make_note(title, body, scope, root, path, fm, date)
    n.supersedes, n.superseded_by = rbxvault.supersession(text)
    if len(_CACHE) > 50000:
        _CACHE.clear()
    _CACHE[key] = (sig, n)
    return n


def load_notes(roots):
    """Every note in the given folders [(scope, folder)] (not House rules, not the old lessons file)."""
    out = []
    for scope, root in roots:
        for title, path in rbxnotes.note_files(root):
            if title.casefold() == rbxnotes.LESSONS_NAME[:-3].casefold():
                continue
            n = read_note(title, path, scope, root)
            if n is not None:
                out.append(n)
    return out


def filing_roots(scope, project=None):
    """The folders a note filed in this scope is checked against: its own, plus the notes for every game."""
    roots = [(scope, rbxnotes.folder(scope, project))]
    if scope == "game":
        roots.append(("all", rbxnotes.folder("all", project)))
    return roots


# ------------------------------------------------------------------ comparing two notes
def jaccard(a, b):
    if not a or not b:
        return 0.0
    inter = len(a & b)
    return inter / (len(a) + len(b) - inter)


def conflict(fa, fb):
    """Why two sentences' facts (facts()) may contradict each other, or None."""
    wa, na, pa, aa = fa
    wb, nb, pb, ab = fb
    shared = len(wa & wb)
    if shared < SENT_SHARED or shared < SENT_JACCARD * (len(wa) + len(wb) - shared):
        return None
    if na and nb and na != nb:
        return "numbers"
    if pa != pb:
        return "yes/no"
    if aa and ab and aa != ab:
        return "API names"
    return None


def conflicting_pairs(a, b, limit=SHOW_PAIRS):
    """[(sentence of a, sentence of b, why)] for two notes, at most limit."""
    out = []
    for sa, fa in a.sents:
        for sb, fb in b.sents:
            why = conflict(fa, fb)
            if why:
                out.append((sa, sb, why))
                if len(out) >= limit:
                    return out
    return out


def is_duplicate(a, b, by_title=True):
    """The Jaccard of two notes' words (1.0 for the same title) when they are near-duplicates, else None. by_title
    False: a's title was made from its first words, so only the two texts are compared."""
    if by_title and a.title.casefold() == b.title.casefold():
        return 1.0
    wa, wb = (a.words, b.words) if by_title else (words(a.body), words(b.body))
    if min(len(wa), len(wb)) < DUP_MIN_WORDS:
        return None
    j = jaccard(wa, wb)
    if j < DUP_JACCARD or conflicting_pairs(a, b, 1):
        return None
    return j


def near_copy(a, b):
    """True when two notes' texts are near-duplicates (the same words, no fact that differs)."""
    return is_duplicate(a, b, by_title=False) is not None


def _related(a, b):
    """True when one of two notes supersedes the other (they are not checked against each other)."""
    return (b.title.casefold() in {t.casefold() for t in a.supersedes} or
            a.title.casefold() in {t.casefold() for t in b.supersedes} or
            a.superseded_by.casefold() == b.title.casefold() or b.superseded_by.casefold() == a.title.casefold())


# ------------------------------------------------------------------ when a note is filed
def _candidates(scope, project, skip=(), exclude=None):
    skip = {t.casefold() for t in skip}
    ex = os.path.normcase(os.path.abspath(exclude)) if exclude else None
    out = []
    for n in load_notes(filing_roots(scope, project)):
        if n.title.casefold() in skip or (ex and os.path.normcase(os.path.abspath(n.path)) == ex):
            continue
        out.append(n)
    return out


def _replaced_here(scope, project):
    """The titles (casefolded) of the notes for every game that this game's notes replace (its superseded.json)."""
    if scope != "game" or not project:
        return set()
    return {" ".join(old.split()).casefold() for old, v in rbxnotes.sidecar_read(rbxnotes.game_notes(project)).items()
            if v.get("shelf") == "yours"}


def _current(n, replaced):
    return not n.superseded_by and not (n.scope == "all" and n.title.casefold() in replaced)


def find_duplicate(text, title, scope, project=None, skip=(), exclude=None, titled=True):
    """(the existing note a new one would duplicate, how alike, whether by its title, whether its text is a near-copy)
    or None. skip: titles the new note supersedes; exclude: the path of the note it updates; titled False: the title
    was made from the text's first words, so only the texts are compared (two notes may start alike)."""
    new = make_note(title, (text or "").strip(), scope)
    best = None
    replaced = _replaced_here(scope, project)
    for n in _candidates(scope, project, skip, exclude):
        if not _current(n, replaced):
            continue
        j = is_duplicate(new, n, titled)
        if j is not None and (best is None or j > best[1] or (j == best[1] and n.scope == scope != best[0].scope)):
            best = (n, j, titled and n.title.casefold() == new.title.casefold())
    if best is None:
        return None
    n, j, same = best
    return n, j, same, j < 1.0 or near_copy(new, n)


def find_conflicts(text, title, scope, project=None, skip=(), exclude=None):
    """[(note, [(new sentence, its sentence, why)])] for the current notes a new one may contradict, most first."""
    new = make_note(title, (text or "").strip(), scope)
    found = []
    replaced = _replaced_here(scope, project)
    for n in _candidates(scope, project, skip, exclude):
        if not _current(n, replaced) or _related(new, n):
            continue
        pairs = conflicting_pairs(new, n)
        if pairs:
            found.append((n, pairs))
    found.sort(key=lambda x: (-len(x[1]), -jaccard(new.words, x[0].words)))
    return found[:SHOW_CONFLICTS]


def _where(n, scope=None):
    return "this game" if n.scope == "game" else "everything"


def _first_lines(n, count=3):
    lines = [s for s, _ in n.sents][:count]
    return ["  " + (s if len(s) <= 200 else s[:199].rstrip() + "…") for s in lines] or ["  (no text)"]


def duplicate_text(dup, scope, title=""):
    """The answer when a note is not filed because it duplicates one: first supersedes and force (nothing is lost),
    and replace (which overwrites the existing note) only when the two are a same-title near-copy."""
    n, j, same, copy = dup
    how = "the same title" if same else "%d%% the same words" % round(100 * j)
    renamed = " under a new title" if (title or "").casefold() == n.title.casefold() else ""
    advice = ("If your note replaces it, file again%s with supersedes: '%s' (the old note is kept, marked replaced). "
              "To keep both, file again with force: true." % (renamed, n.title))
    if same and copy:
        where = "" if n.scope == scope else "scope '%s', " % n.scope
        advice += (" If yours is only a corrected copy of it, file again with %sreplace: true and title '%s': this "
                   "overwrites the existing note, no copy is kept." % (where, n.title))
    return "\n".join([
        "Not saved: nothing was filed, a note like this exists already (%s)." % how,
        "Existing note '%s' (%s, %s):" % (n.title, _where(n), n.date or "no date")] + _first_lines(n) + [advice])


def conflicts_text(found, title):
    """What is added to the answer when a filed note may contradict others."""
    out = []
    for n, pairs in found:
        out.append("\n\nPossible contradiction with '%s' (%s, %s):" % (n.title, _where(n), n.date or "no date"))
        for mine, theirs, why in pairs:
            out.append("\n  [%s differ]\n  this note: %s\n  '%s': %s" % (why, mine, n.title, theirs))
    names = ", ".join("'%s'" % n.title for n, _ in found)
    out.append("\n\nResolve it: if this note replaces %s, file it again with replace: true, title '%s' and supersedes "
               "naming the older note; if the older note is right, file that one again with supersedes: '%s'; if both "
               "hold (different cases), say so and leave them." % (names, title, title))
    return "".join(out)


# ------------------------------------------------------------------ the sweep
BUCKET_CAP = 300  # a pair of words shared by this many sets already is too common to say two of them are alike


def _similar_pairs(sets, t, min_shared=2):
    """(i, j, jaccard) for the pairs of sets with Jaccard >= t and at least min_shared (>= 2) shared items, without
    comparing every pair: prefix filtering on word pairs. Ordered by how rare its words are, a set's first
    |x| - ceil(t*|x|) + 2 words always hold two words of any such partner's own first words, so sets are only
    compared when they share a pair of those (and each candidate is checked exactly). A pair of words shared by
    BUCKET_CAP sets is skipped: a likeness found only through such common words is not reported."""
    df = Counter()
    for s in sets:
        df.update(s)
    rank = {w: r for r, (w, _) in enumerate(sorted(df.items(), key=lambda kv: (kv[1], kv[0])))}
    size = len(rank) + 1
    index = defaultdict(list)
    out = []
    for i in sorted(range(len(sets)), key=lambda i: len(sets[i])):
        s = sets[i]
        n = len(s)
        if n < max(2, min_shared):
            continue
        toks = sorted(rank[w] for w in s)[:n - math.ceil(t * n - 1e-9) + 2]
        seen = set()
        for x in range(len(toks)):
            base = toks[x] * size
            for y in range(x + 1, len(toks)):
                bucket = index[base + toks[y]]
                if len(bucket) >= BUCKET_CAP:
                    continue
                for j in bucket:
                    if j in seen:
                        continue
                    seen.add(j)
                    sj = sets[j]
                    m = len(sj)
                    if m < t * n:
                        continue
                    inter = len(s & sj)
                    if inter >= min_shared and inter >= t * (n + m - inter):
                        out.append((j, i, inter / (n + m - inter)))
                bucket.append(i)
    return out


def _comparable(a, b):
    return a.root == b.root or a.scope == "all" or b.scope == "all"


def _days(date):
    try:
        return time.mktime(time.strptime(date[:10], "%Y-%m-%d")) / 86400
    except (TypeError, ValueError, OverflowError):
        return None


def _known_targets(notes):
    """Every name a [[link]] may point at: the scanned notes, the shipped vault notes, guides and team files."""
    names = {n.title.casefold() for n in notes} | {rbxnotes.RULES_NAME[:-3].casefold(),
                                                   rbxnotes.LESSONS_NAME[:-3].casefold(), "game"}
    try:
        if os.path.isdir(rbxvault.KB):
            names |= {os.path.basename(p)[:-3].casefold() for p in rbxvault.note_files()}
        for t, p in rbxvault.guide_files():
            names |= {t.casefold(), t.rsplit("/", 1)[-1].casefold(), os.path.basename(p)[:-3].casefold()}
        names |= {t.casefold() for t, _ in rbxvault.team_files(rbxvault.kit_team_dir())}
    except Exception:  # a shelf that does not list only makes more links look broken
        pass
    return names


class _Sets:
    """Union-find over note indexes (the near-duplicate clusters)."""

    def __init__(self):
        self.up = {}

    def find(self, x):
        self.up.setdefault(x, x)
        while self.up[x] != x:
            self.up[x] = self.up[self.up[x]]
            x = self.up[x]
        return x

    def join(self, a, b):
        self.up[self.find(a)] = self.find(b)


def analyse(notes, sup, now):
    """The sweep's findings over a list of Notes: {"duplicates", "conflicts", "linked", "stale", "broken"}."""
    by_title = defaultdict(list)
    for i, n in enumerate(notes):
        by_title[n.title.casefold()].append(i)
    old = [bool(n.superseded_by) or n.title.casefold() in sup for n in notes]

    # near-duplicates: the same title in two folders, or nearly the same words with no conflicting sentence
    sets = _Sets()
    alike = {}
    for ids in by_title.values():
        for a in ids:
            for b in ids:
                if a < b and _comparable(notes[a], notes[b]):
                    sets.join(a, b)
                    alike[(a, b)] = 1.0
    for a, b, j in _similar_pairs([n.words if len(n.words) >= DUP_MIN_WORDS else frozenset() for n in notes],
                                  DUP_JACCARD):
        a, b = min(a, b), max(a, b)
        if _comparable(notes[a], notes[b]) and not conflicting_pairs(notes[a], notes[b], 1):
            sets.join(a, b)
            alike[(a, b)] = max(alike.get((a, b), 0), j)
    groups = defaultdict(list)
    for a, b in alike:
        groups[sets.find(a)] += [a, b]
    duplicates = []
    for ids in groups.values():
        ids = sorted(set(ids), key=lambda i: notes[i].title.casefold())
        best = max(j for (a, b), j in alike.items() if a in ids)
        duplicates.append((ids, best))
    duplicates.sort(key=lambda g: (-len(g[0]), -g[1], notes[g[0][0]].title.casefold()))

    # possible contradictions: sentences on the same thing whose facts differ, in current notes
    sents, owner = [], []
    for i, n in enumerate(notes):
        if old[i]:
            continue
        for s, f in n.sents:
            if len(f[0]) >= SENT_SHARED:
                sents.append((s, f))
                owner.append(i)
    pairs = defaultdict(list)
    for x, y, _ in _similar_pairs([f[0] for _, f in sents], SENT_JACCARD, SENT_SHARED):
        a, b = owner[x], owner[y]
        if a == b or not _comparable(notes[a], notes[b]) or _related(notes[a], notes[b]):
            continue
        why = conflict(sents[x][1], sents[y][1])
        if why:
            if a > b:
                a, b, x, y = b, a, y, x
            if len(pairs[(a, b)]) < SHOW_PAIRS:
                pairs[(a, b)].append((sents[x][0], sents[y][0], why))
    conflicts = sorted(pairs.items(), key=lambda kv: (-len(kv[1]), notes[kv[0][0]].title.casefold()))

    # superseded notes still linked from current ones (the replacement itself may name what it replaces)
    linked = []
    for i, n in enumerate(notes):
        if old[i]:
            continue
        for t in n.links:
            new = sup.get(t)
            if new and new.casefold() != n.title.casefold():
                linked.append((i, t, new))

    # old notes that newer notes on the same topic exist for
    df = Counter()
    for n in notes:
        df.update(n.words)
    sig = [sorted((w for w in n.words if df[w] >= 2), key=lambda w: (df[w], w))[:TOPIC_RARE] for n in notes]
    post = defaultdict(list)
    for i, s in enumerate(sig):
        for w in s:
            post[w].append(i)
    days = [_days(n.date) for n in notes]
    cutoff = now / 86400 - STALE_DAYS
    stale = []
    for i, n in enumerate(notes):
        if old[i] or days[i] is None or days[i] >= cutoff or not sig[i]:
            continue
        counts = Counter()
        for w in sig[i]:
            for j in post[w]:
                if j != i:
                    counts[j] += 1
        need = min(TOPIC_SHARED, len(sig[i]))
        newer = []
        for j, c in counts.items():
            if (c >= need and not old[j] and days[j] is not None and days[j] > days[i]
                    and _comparable(n, notes[j]) and jaccard(n.words, notes[j].words) >= TOPIC_JACCARD):
                newer.append(j)
        if newer:
            newer.sort(key=lambda j: -days[j])
            stale.append((i, newer[:3], len(newer)))
    stale.sort(key=lambda s: (days[s[0]], notes[s[0]].title.casefold()))

    # broken [[links]]
    known = _known_targets(notes)
    broken = [(i, t) for i, n in enumerate(notes) for t in n.links if t not in known]
    return {"duplicates": duplicates, "conflicts": conflicts, "linked": linked, "stale": stale, "broken": broken}


def _name(n, games):
    return "'%s'" % n.title + ("" if n.scope == "all" or len(games) < 2 else " (game %s)" % games.get(n.root, "?"))


def report_text(notes, found, now, seconds, games):
    """The sweep's Markdown report."""
    day = time.strftime("%Y-%m-%d", time.localtime(now))
    per = Counter("everything" if n.scope == "all" else "game %s" % games.get(n.root, "?") for n in notes)
    out = ["# Vault upkeep %s" % day, "",
           "Scanned %d notes (%s) in %.1f s. This report only lists what to look at: no note was changed." % (
               len(notes), ", ".join("%s: %d" % kv for kv in sorted(per.items())) or "none", seconds),
           "To act on one: file the note that holds with rbx_vault 'note' and supersedes naming the outdated one "
           "(it stays, ranked below), update a note with replace: true, or move one to the trash with 'forget'.", ""]

    def section(title, rows):
        """rows: one list of lines per finding."""
        out.append("## %s (%d)" % (title, len(rows)))
        out.append("")
        for lines in rows[:REPORT_MAX]:
            out.extend(lines)
        if not rows:
            out.append("Nothing found.")
        if len(rows) > REPORT_MAX:
            out.append("- … and %d more" % (len(rows) - REPORT_MAX))
        out.append("")

    nm = lambda i: _name(notes[i], games)  # noqa: E731
    section("Near-duplicates", [["- %s: %s" % (
        "same title" if j >= 1.0 else "%d%% the same words" % round(100 * j), ", ".join(nm(i) for i in ids))]
        for ids, j in found["duplicates"]])
    rows = []
    for (a, b), pairs in found["conflicts"]:
        lines = ["- %s and %s" % (nm(a), nm(b))]
        for sa, sb, why in pairs:
            lines += ["  - %s differ:" % why, "    - %s: \"%s\"" % (notes[a].title, sa),
                      "    - %s: \"%s\"" % (notes[b].title, sb)]
        rows.append(lines)
    section("Possible contradictions", rows)
    section("Superseded notes still linked", [["- %s links to [[%s]], replaced by '%s'" % (
        nm(i), _shown_link(notes[i], t), new)] for i, t, new in found["linked"]])
    section("Old notes with newer ones on the same topic", [["- %s (%s): newer %s%s" % (
        nm(i), notes[i].date, ", ".join("%s (%s)" % (nm(j), notes[j].date) for j in newer),
        " and %d more" % (total - len(newer)) if total > len(newer) else "")] for i, newer, total in found["stale"]])
    section("Broken links", [["- %s links to [[%s]]" % (nm(i), _shown_link(notes[i], t))] for i, t in found["broken"]])
    return "\n".join(out).rstrip() + "\n"


def _shown_link(n, target):
    """A link target as the note writes it (wikilinks are casefolded)."""
    for m in rbxvault._WIKILINK.finditer(n.body):
        t = " ".join(m.group(1).split())
        if t.casefold().endswith(".md"):
            t = t[:-3]
        t = t.rsplit("/", 1)[-1]
        if t.casefold() == target:
            return t
    return target


def sweep(projects=(), now=None):
    """Scan the user's notes (and the given games' notes) and write the report. Returns {"path", "counts",
    "notes", "seconds"}."""
    start = time.time()
    now = time.time() if now is None else now
    roots = [("all", rbxnotes.folder("all", None))]
    games = {}
    for p in projects or ():
        if p and os.path.isdir(p):
            root = rbxnotes.folder("game", p)
            roots.append(("game", root))
            games[root] = rbxvault.game_name(p)
    notes = load_notes(roots)
    sup = rbxvault.superseded_map(
        ({"title": n.title, "supersedes": n.supersedes, "superseded_by": n.superseded_by} for n in notes),
        dict(rbxnotes.sidecars(None), **{k: v for p in projects or () if p for k, v in rbxnotes.sidecars(p).items()}))
    found = analyse(notes, sup, now)
    seconds = time.time() - start
    text = report_text(notes, found, now, seconds, games)
    folder = os.path.join(rbxnotes.home_notes(), "upkeep")
    path = os.path.join(folder, time.strftime("%Y-%m-%d", time.localtime(now)) + ".md")
    rbxnotes._atomic_write(path, text)
    counts = {"near_duplicates": len(found["duplicates"]), "contradictions": len(found["conflicts"]),
              "superseded_linked": len(found["linked"]), "stale": len(found["stale"]),
              "broken_links": len(found["broken"])}
    return {"path": path, "counts": counts, "notes": len(notes), "seconds": round(time.time() - start, 2)}


def summary_text(res):
    c = res["counts"]
    return ("Upkeep report written to %s (%d notes, %.1f s): %d near-duplicate cluster(s), %d possible "
            "contradiction(s), %d link(s) to superseded notes, %d old note(s) with newer ones on the same topic, %d "
            "broken link(s). It only lists them; read it, then fix what needs fixing with 'note' (supersedes or "
            "replace) or 'forget'." % (res["path"], res["notes"], res["seconds"], c["near_duplicates"],
                                       c["contradictions"], c["superseded_linked"], c["stale"], c["broken_links"]))


# ------------------------------------------------------------------ rbx upkeep
def cmd_upkeep(a):
    import json
    import sys
    res = sweep(projects=a.project or [])
    if a.json:
        sys.stdout.write(json.dumps(res, ensure_ascii=False) + "\n")
    else:
        print(summary_text(res))


def register(sub):
    p = sub.add_parser("upkeep", help="check the user's notes (duplicates, contradictions, stale and broken links) "
                                      "and write a report to <notes>/upkeep/YYYY-MM-DD.md")
    p.add_argument("--project", action="append", help="a game folder whose notes are checked too (repeatable)")
    p.add_argument("--json", action="store_true", help="print the result as JSON")
    p.set_defaults(fn=cmd_upkeep)
