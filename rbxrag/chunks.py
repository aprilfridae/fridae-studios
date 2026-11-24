"""What the knowledge index holds, and the token-aware chunks it is made of.

A retrieval unit is one of rbxvault's sections (parse_sections: the same parser the keyword search and the reads use),
vault notes included, so a hit maps straight onto rbx_vault's `read` arguments. A section longer than the model's
window is split by real tokens, on paragraph and then line boundaries, and a single over-long line is cut by tokens.
"""
import math
import os
import re
import uuid

import rbxvault
from rbxpaths import KIT

LIMIT = 480  # tokens per chunk at most: a 512-token window less a 32-token margin
SEP = " › "
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
    return out


def project_files(project):
    """The paths of corpus_files(project) that belong to the game (its notes and its team overrides)."""
    if not project:
        return set()
    return {p for _, p, mine in _team(project) if mine} | {p for _, p in rbxvault.game_files(project)}


# ------------------------------------------------------------------ chunks
def chunk_id(shelf, path, i, part):
    return str(uuid.uuid5(uuid.NAMESPACE_URL, "%s|%s|%s|%s" % (shelf, path, i, part)))


def _rel(shelf, path):
    if shelf == "vault":
        return os.path.relpath(path, rbxvault.KB).replace(os.sep, "/")
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


def chunks_for_file(shelf, title, path, count_tokens, limit=LIMIT, project=""):
    """The chunks of one file: one per section with text, split by tokens when a section is too long for the model.
    An unreadable or non-UTF-8 file gives no chunks."""
    try:
        text = rbxvault.read(path)
    except (OSError, UnicodeDecodeError):
        return []
    if "\x00" in text:
        return []
    p = rbxvault.parse_sections(text)
    rel = _rel(shelf, path)
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
        prefix = "%s%s%s\n" % (title, SEP, s["heading"])
        if count_tokens(prefix) > limit // 4:  # a freak heading must not eat the window
            prefix = "%s%s%s\n" % (title[:80], SEP, s["heading"][:120])
        for part, body in enumerate(split_text(own, prefix, count_tokens, limit)):
            out.append({"id": chunk_id(shelf, path, i, part), "shelf": shelf, "title": title,
                        "heading": s["heading"], "path": path, "rel": rel, "i": i, "part": part,
                        "text": prefix + body, "project": project or ""})
    return out
