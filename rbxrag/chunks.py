"""What the knowledge index holds, and the token-aware chunks it is made of.

A retrieval unit is one of rbxvault's sections (parse_sections: the same parser the keyword search and the reads use),
vault notes included, so a hit maps straight onto rbx_vault's `read` arguments. A section longer than the model's
window is split by real tokens, on paragraph and then line boundaries, and a single over-long line is cut by tokens.

Every chunk starts with two lines of context, embedded with it (no model writes them; they cost nothing to make):
  <title> › <heading>
  Context: [<note H1 > parent > heading> |] <shelf or game> | <kind>, <source>, <YYYY-MM> [| <the note's gist>]
(the path only when it says more than the first line; the gist not in the opening, which already starts with it)
and carries the note's labels (rbxvault.note_labels: game, kind, source, date) for the search filters.
"""
import math
import os
import re
import uuid

import rbxvault
from rbxpaths import KIT

LIMIT = 480  # tokens per chunk at most: a 512-token window less a 32-token margin
SEP = " › "
GIST_WORDS = 30  # the gist in a chunk's context line, at most (about 40 tokens)
LABEL_KEYS = ("game", "kind", "source", "date")
_RELATED = re.compile(r"<!-- related:start -->.*?<!-- related:end -->", re.S)
_TLDR_MARKS = re.compile(r"<!-- tldr:(start|end) -->")


def make_token_counter(tokenizer_json=None):
    """A function giving the number of tokens in a text. With a model's tokenizer.json it counts the model's real
    tokens (special tokens included, never truncated); without one it is a pessimistic estimate of one token per
    three bytes, for tests and for when no model is present."""
    if tokenizer_json:
        from tokenizers import Tokenizer
        tok = Tokenizer.from_file(tokenizer_json)
        tok.no_truncation()  # a model's tokenizer.json usually truncates at its window, which would hide overflow
        tok.no_padding()
        return lambda text: len(tok.encode(text).ids)
    return lambda text: math.ceil(len(text.encode("utf-8")) / 3)


# ------------------------------------------------------------------ the corpus
def _team(project):
    """[(title, path, from the game?)] for the team shelf: the kit's experts, a game's overrides replacing them."""
    kit = rbxvault.team_files(rbxvault.kit_team_dir())
    if not project:
        return [(t, p, False) for t, p in kit]
    mine = rbxvault.team_files(os.path.join(project, ".fridae", "team", "experts"))
    ids = {t for t, _ in mine}
    return [(t, p, False) for t, p in kit if t not in ids] + [(t, p, True) for t, p in mine]


SKILL_KEY = "#skill"  # a skill's index key is its SKILL.md's path plus this: a guide's SKILL.md is also on the guides shelf


def real_path(key):
    """The file behind a corpus path (a skill's index key without its suffix)."""
    return key[:-len(SKILL_KEY)] if key.endswith(SKILL_KEY) else key


def _skills(project):
    """[(id, index key, from the game?)] for the skills shelf: one entry per skill of the library (the project's own
    skills only with a project)."""
    import rbxskills
    return [(s["id"], os.path.join(s["dir"], "SKILL.md") + SKILL_KEY, s["origin"] == "project")
            for s in rbxskills.library(project or None) if os.path.isfile(os.path.join(s["dir"], "SKILL.md"))]


def corpus_files(project):
    """(shelf, title, path) for everything the index holds: the vault notes, Fridae's guides, the team files (with the
    game's overrides when a project is given), the user's notes and, with a project, the game's own notes. Nothing
    else on disk."""
    out = []
    if os.path.isdir(rbxvault.KB):
        out += [("vault", os.path.basename(p)[:-3], p) for p in rbxvault.note_files()]
    out += [("guides", t, p) for t, p in rbxvault.guide_files()]
    out += [("team", t, p) for t, p, _ in _team(project)]
    out += [("yours", t, p) for t, p in rbxvault.yours_files()]
    if project:
        out += [("game", t, p) for t, p in rbxvault.game_files(project)]
    out += [("skills", t, k) for t, k, _ in _skills(project)]
    return out


def project_files(project):
    """The paths of corpus_files(project) that belong to the game (its notes and its team overrides)."""
    if not project:
        return set()
    return ({p for _, p, mine in _team(project) if mine} | {p for _, p in rbxvault.game_files(project)}
            | {k for _, k, mine in _skills(project) if mine})


# ------------------------------------------------------------------ chunks
def chunk_id(shelf, path, i, part):
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "%s|%s|%s|%s" % (shelf, path, i, part)))


def _rel(shelf, path):
    if shelf == "vault":
        return rbxvault.rel_to(path, rbxvault.KB)
    if shelf in ("guides", "team"):
        try:
            if os.path.commonpath([os.path.abspath(KIT), os.path.abspath(path)]) == os.path.abspath(KIT):
                return os.path.relpath(path, KIT).replace(os.sep, "/")
        except ValueError:
            pass
    return path


def _hard_split(line, count, budget):
    """Cut one line into pieces of at most budget tokens, preferring a cut at a space."""
    out = []
    rest = line
    while rest:
        if count(rest) <= budget:
            out.append(rest)
            break
        lo, hi = 1, len(rest)
        while lo < hi:  # the longest prefix that fits
            mid = (lo + hi + 1) // 2
            if count(rest[:mid]) <= budget:
                lo = mid
            else:
                hi = mid - 1
        cut = lo
        space = rest.rfind(" ", int(cut * 0.8), cut)
        if space > 0:
            cut = space + 1
        out.append(rest[:cut])
        rest = rest[cut:]
    return out


def _units(text, count, budget):
    """(separator, piece, tokens) pieces of a text, each within budget: paragraphs, else lines, else cut lines."""
    first = True
    for para in re.split(r"\n[ \t]*\n", text):
        if not para.strip():
            continue
        sep = "" if first else "\n\n"
        first = False
        n = count(para)
        if n <= budget:
            yield sep, para, n
            continue
        for line in para.split("\n"):
            if not line.strip():
                continue
            n = count(line)
            if n <= budget:
                yield sep, line, n
                sep = "\n"
                continue
            for piece in _hard_split(line, count, budget):
                yield sep, piece, count(piece)
                sep = "\n"
            sep = "\n"


def split_text(text, prefix, count, limit=LIMIT):
    """Bodies such that prefix + body is at most limit tokens each (a text that fits is one body)."""
    text = text.strip()
    if count(prefix + text) <= limit:
        return [text]
    budget = max(8, limit - count(prefix))
    bodies, cur, cur_n = [], "", 0
    for sep, piece, n in _units(text, count, budget):
        if cur and cur_n + n > budget:
            bodies.append(cur)
            cur, cur_n = "", 0
        cur = (cur + sep + piece) if cur else piece
        cur_n += n
    if cur:
        bodies.append(cur)
    out = []
    for b in bodies:  # token counts are not exactly additive: check each body and cut it finer when it is over
        if count(prefix + b) <= limit or budget <= 8:
            out.append(b)
        else:
            out += split_text(b, prefix, count, limit - max(4, (count(prefix + b) - limit) + 4))
    return out


def context_line(shelf, title, label, section, labels, gist):
    """A chunk's second line: where it sits in its note (H1 > H2 > H3, when that says more than the first line),
    which shelf or game, its labels and the note's gist."""
    path = ""
    if section["parents"] or (label and label != title):
        trail = [label or title] + list(section["parents"]) + ([section["heading"]] if section["level"] else [])
        path = " > ".join(t for n, t in enumerate(trail) if not n or t != trail[n - 1])[:200]
    where = "game %s" % labels["game"] if labels.get("game") not in (None, "", "all") else shelf
    tags = ", ".join(x for x in (labels.get("kind"), labels.get("source"), (labels.get("date") or "")[:7]) if x)
    bits = [path, where, tags] + ([gist] if gist else [])
    return rbxvault.CONTEXT_MARK + " | ".join(b for b in bits if b) + "\n"


def _prefix(shelf, title, label, s, labels, gist, count_tokens, limit):
    """The two context lines of a section's chunks, shortened (gist first) so they never take more than a quarter of
    the window."""
    for g, t, h in ((gist, title, s["heading"]), (" ".join(gist.split()[:12]), title, s["heading"]),
                    ("", title[:80], s["heading"][:120])):
        prefix = "%s%s%s\n" % (t, SEP, h) + context_line(shelf, t, label[:80], dict(s, heading=h), labels, g)
        if count_tokens(prefix) <= limit // 4:
            return prefix
    return "%s%s%s\n" % (title[:80], SEP, s["heading"][:120])  # a freak heading must not eat the window


def chunks_for_file(shelf, title, path, count_tokens, limit=LIMIT, project="", meta=None):
    """The chunks of one file: one per section with text, split by tokens when a section is too long for the model,
    each with its context lines and labels. meta (a dict), when given, gets the file's [[links]] ("links"). An
    unreadable or non-UTF-8 file gives no chunks. A skill (shelf skills) is one chunk, its summary, whatever its size."""
    if shelf == "skills":
        return _skill_chunks(title, real_path(path), count_tokens, limit, project)
    try:
        text = rbxvault.read(path)
    except (OSError, UnicodeDecodeError):
        return []
    if "\x00" in text:
        return []
    p = rbxvault.parse_sections(text)
    if meta is not None:
        meta["links"] = rbxvault.wikilinks(p["body"])
    rel = _rel(shelf, path)
    labels = rbxvault.note_labels(shelf, title, path, p["fm"], text, project)
    gist = p["description"] or rbxvault.note_gist(p["fm"], _TLDR_MARKS.sub("", _RELATED.sub("", p["body"])),
                                                  GIST_WORDS)
    label = p["label"] or title
    out = []
    for i, s in enumerate(p["sections"]):
        own = p["body"][s["start"]:s["own_end"]]
        if s["level"]:
            own = own.split("\n", 1)[1] if "\n" in own else ""
        if shelf == "vault":
            own = _TLDR_MARKS.sub("", _RELATED.sub("", own))
        if not s["level"] and p["description"]:
            own = p["description"] + "\n\n" + own
        if not own.strip():
            continue
        lab = rbxvault.section_labels(labels, s["heading"], s["parents"])
        # the opening already holds the description or the first sentence the gist is made of
        g = "" if not s["level"] else gist
        prefix = _prefix(shelf, title, label, s, lab, g, count_tokens, limit)
        for part, body in enumerate(split_text(own, prefix, count_tokens, limit)):
            c = {"id": chunk_id(shelf, path, i, part), "shelf": shelf, "title": title,
                 "heading": s["heading"], "path": path, "rel": rel, "i": i, "part": part,
                 "text": prefix + body, "project": project or ""}
            c.update({k: lab.get(k) or "" for k in LABEL_KEYS})
            out.append(c)
    return out


def _skill_chunks(title, path, count_tokens, limit, project=""):
    """The skills shelf's one chunk for a skill: its summary (rbxskills.summary), never the body of its SKILL.md."""
    import rbxskills
    try:
        fm, _ = rbxskills.front(rbxskills._read(path, 16000))
    except (OSError, ValueError):
        return []
    m = rbxskills.meta(fm, os.path.dirname(path), "user")
    text = rbxskills.summary(m)
    while count_tokens(text) > limit and len(text) > 40:  # a freak front matter must not outgrow the window
        text = text[:int(len(text) * 0.8)].rstrip()
    c = {"id": chunk_id("skills", path, 0, 0), "shelf": "skills", "title": title, "heading": "", "path": path,
         "rel": path, "i": 0, "part": 0, "text": text, "project": project or ""}
    c.update({k: "" for k in LABEL_KEYS})
    return [c]
