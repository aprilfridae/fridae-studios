"""The knowledge exam's pure parts (no models, no services): the stage cache, the scores, the judge's verdicts, the
failure classes, the not-found threshold, the estimates and the report's tables. exam.py runs the exam with them;
tests/test_exam.py checks them."""
import hashlib
import json
import math
import os
import re
import tempfile

NOT_FOUND = "not in the knowledge base"  # the Librarian's own words (librarian.rs NOT_FOUND)
GUIDE_MARK = "Recommended guide:"  # how the Librarian points at a guide (librarian.rs)
FACT_STATES = ("present", "absent", "contradicted")


# ------------------------------------------------------------------ hashing and the stage cache
def file_hash(paths, extra=""):
    """A short hash of these files' names and contents (missing files count as missing) and extra."""
    h = hashlib.sha256(extra.encode())
    for p in sorted(paths):
        h.update(os.path.basename(p).encode() + b"\0")
        try:
            with open(p, "rb") as f:
                h.update(hashlib.sha1(f.read()).digest())
        except OSError:
            h.update(b"missing")
    return h.hexdigest()[:12]


def write_json(path, obj):
    """Write JSON atomically (a killed run never leaves half a file)."""
    os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
    fd, tmp = tempfile.mkstemp(prefix=".tmp-", suffix=".json", dir=os.path.dirname(path) or ".")
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(obj, f, indent=1, ensure_ascii=False)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.remove(tmp)
        except OSError:
            pass
        raise


def read_json(path, default=None):
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return default


class StageCache:
    """<root>/<stage>-<key>/: one JSON file per finished unit of a stage, and JSONL files that grow one record at a
    time (so a killed run keeps every answer it already has). The key holds the code version and the corpus, so a
    change to either starts that stage afresh while the old results stay where they were."""

    def __init__(self, root, stamp=None):
        self.root = root
        self.stamp = stamp  # added to every unit saved (the code commit measured)

    def dir(self, stage, key):
        d = os.path.join(self.root, "%s-%s" % (stage, key))
        os.makedirs(d, exist_ok=True)
        return d

    def path(self, stage, key, name, ext=".json"):
        return os.path.join(self.dir(stage, key), re.sub(r"[^A-Za-z0-9_.=-]+", "_", name) + ext)

    def get(self, stage, key, name):
        return read_json(self.path(stage, key, name))

    def put(self, stage, key, name, obj):
        if self.stamp and isinstance(obj, dict):
            obj.setdefault("_measured", self.stamp)
        write_json(self.path(stage, key, name), obj)
        return obj

    def records(self, stage, key, name):
        """The records of a JSONL unit, by their "id" (the last one wins; a torn last line is skipped)."""
        out = {}
        try:
            with open(self.path(stage, key, name, ".jsonl"), encoding="utf-8") as f:
                for line in f:
                    try:
                        r = json.loads(line)
                    except ValueError:
                        continue
                    if isinstance(r, dict) and r.get("id") is not None:
                        out[r["id"]] = r
        except OSError:
            pass
        return out

    def append(self, stage, key, name, record):
        if self.stamp and isinstance(record, dict):
            record.setdefault("_measured", self.stamp.get("commit") if isinstance(self.stamp, dict) else self.stamp)
        with open(self.path(stage, key, name, ".jsonl"), "a", encoding="utf-8") as f:
            f.write(json.dumps(record, ensure_ascii=False) + "\n")
            f.flush()


# ------------------------------------------------------------------ small numbers
def pct(xs, p):
    xs = sorted(x for x in xs if x is not None)
    if not xs:
        return None
    return xs[min(len(xs) - 1, max(0, int(math.ceil(p / 100 * len(xs))) - 1))]


def mean(xs):
    xs = [x for x in xs if x is not None]
    return sum(xs) / len(xs) if xs else None


def rough_tokens(text):
    """About four characters a token."""
    return math.ceil(len(text or "") / 4)


def is_gold(title, gold):
    t = (title or "").casefold()
    return any(t == g.casefold() or t.endswith("/" + g.casefold()) for g in gold or ())


def first_rank(titles, gold, k=None):
    """1-based rank of the first gold title (None when none is in the first k)."""
    for i, t in enumerate(titles[:k] if k else titles, 1):
        if is_gold(t, gold):
            return i
    return None


def dedupe(xs):
    seen, out = set(), []
    for x in xs:
        if x not in seen:
            seen.add(x)
            out.append(x)
    return out


def hit_rates(ranks, ks=(1, 3, 5)):
    """{"n", "hit@1", "hit@3", "hit@5", "mrr"} over 1-based ranks (None: not found)."""
    n = len(ranks)
    out = {"n": n}
    for k in ks:
        out["hit@%d" % k] = sum(1 for r in ranks if r is not None and r <= k) / n if n else None
    out["mrr"] = sum(1.0 / r for r in ranks if r) / n if n else None
    return out


# ------------------------------------------------------------------ the "not found" threshold
def best_threshold(pos, neg):
    """The score threshold that best tells answerable questions (pos: their best scores) from questions the
    knowledge base cannot answer (neg), "found" meaning score >= threshold. Picks the highest balanced accuracy
    ((share of pos kept + share of neg refused) / 2), ties to the lower threshold. Returns {"threshold", "balanced",
    "kept", "refused"} or None without both kinds."""
    pos = [x for x in pos if x is not None]
    neg = [x for x in neg if x is not None]
    if not pos or not neg:
        return None
    best = None
    for t in sorted(set(pos) | set(neg)):
        kept = sum(1 for x in pos if x >= t) / len(pos)
        refused = sum(1 for x in neg if x < t) / len(neg)
        bal = (kept + refused) / 2
        if best is None or bal > best["balanced"] + 1e-12:
            best = {"threshold": t, "balanced": bal, "kept": kept, "refused": refused}
    return best


# ------------------------------------------------------------------ words
_STOP = set("""a an the and or of to in on for with by at from is are be it its this that as into than then so not no
do does did can could should would will we our you your they their them he she his her i my me if when what which who
how why where there here up out over under about all any each more most other some such only own same too very just
also now one two use used using get set""".split())


def words(text):
    return [w for w in re.findall(r"[a-z0-9]+(?:[.,][0-9]+)?", (text or "").casefold()) if w not in _STOP]


def overlap(fact, text):
    """Share of a fact's content words (numbers count as words) found in text."""
    fw = set(words(fact))
    if not fw:
        return 0.0
    return len(fw & set(words(text))) / len(fw)


def says_not_found(answer):
    return NOT_FOUND in (answer or "").casefold()


def guide_pointed(answer, guides):
    """Whether the answer points at one of these guides (by name), or at any guide when none is given."""
    a = (answer or "").casefold()
    guides = [guides] if isinstance(guides, str) else list(guides or [])
    if not guides:
        return GUIDE_MARK.casefold() in a
    return any(g.casefold() in a or g.casefold().split("/")[-1] in a for g in guides)


# ------------------------------------------------------------------ the judge
JUDGE_SYSTEM = """You grade answers written by a knowledge-base assistant for a Roblox game studio. You are strict and \
literal. You never use tools. You reply with JSON only, no prose before or after it."""

JUDGE_PROMPT = """Grade each answer below against its answer key.

For each item:
- "facts": one verdict per key fact, in the order given:
  "present" if the answer states that fact (other wording is fine; every number, name and limit in the fact must \
match),
  "contradicted" if the answer states something that conflicts with it (for example a different number),
  "absent" otherwise.
- "wrong": one true/false per wrong fact, in the order given: true if the answer asserts it (or something with the \
same meaning), false otherwise. Mentioning it only to reject it counts as false.
- "abstained": true if the answer's main message is that the knowledge base does not contain the answer (for example \
"not in the knowledge base"), even if it adds side remarks; false if it gives an answer.
- "guide_pointer": true if the answer directs the reader to one of the guides listed in "guide" (by its name or an \
obvious form of it), false otherwise. Use false when no guide is expected.
- "note": at most 15 words on the main problem, or "" when there is none.

Judge only the text of the answer. Do not reward length or style. Do not use your own knowledge to fill gaps.

Reply with exactly this JSON shape:
{"verdicts": [{"key": "<item key>", "facts": ["present"|"absent"|"contradicted", ...], "wrong": [true|false, ...], \
"abstained": true|false, "guide_pointer": true|false, "note": "..."}]}

Items:
"""


def judge_items_text(items):
    """The items part of a judge prompt: each item's key, question, what kind of answer is expected, its key facts,
    wrong facts, expected guide and the answer to grade."""
    out = []
    for it in items:
        out.append(json.dumps({"key": it["key"], "question": it["question"], "expect": it["expect"],
                               "facts": it.get("facts") or [], "wrong": it.get("wrong") or [],
                               "guide": it.get("guide") or [], "answer": it.get("answer") or ""},
                              ensure_ascii=False))
    return "\n".join(out)


def extract_json(text):
    """The first JSON object in text (a fenced block or bare), or None."""
    if not text:
        return None
    m = re.search(r"```(?:json)?\s*(\{.*?\})\s*```", text, re.S)
    cands = [m.group(1)] if m else []
    i = text.find("{")
    if i >= 0:
        cands.append(text[i:text.rfind("}") + 1])
    for c in cands:
        try:
            v = json.loads(c)
            if isinstance(v, dict):
                return v
        except ValueError:
            continue
    return None


def parse_verdicts(text, items):
    """{key: verdict} for the items the judge graded validly: a verdict's facts list must have one state per key
    fact (from FACT_STATES), its wrong list one boolean per wrong fact. Items with no valid verdict are left out."""
    v = extract_json(text)
    if not v or not isinstance(v.get("verdicts"), list):
        return {}
    by_key = {it["key"]: it for it in items}
    out = {}
    for r in v["verdicts"]:
        if not isinstance(r, dict) or r.get("key") not in by_key:
            continue
        it = by_key[r["key"]]
        facts = r.get("facts") if isinstance(r.get("facts"), list) else None
        wrong = r.get("wrong") if isinstance(r.get("wrong"), list) else None
        if facts is None or len(facts) != len(it.get("facts") or []):
            continue
        facts = [str(f).strip().lower() for f in facts]
        if any(f not in FACT_STATES for f in facts):
            continue
        if wrong is None or len(wrong) != len(it.get("wrong") or []) or any(not isinstance(w, bool) for w in wrong):
            continue
        out[r["key"]] = {"facts": facts, "wrong": wrong, "abstained": bool(r.get("abstained")),
                         "guide_pointer": bool(r.get("guide_pointer")), "note": str(r.get("note") or "")[:200]}
    return out


def score(entry, verdict):
    """One answer's score from its verdict. entry: the answer key (expect, facts, wrong). Returns {"correct" (the
    answer does its job: all key facts and no wrong one; abstains on a negative; points at the guide), "recall"
    (share of key facts present), "wrong" (any wrong fact or contradiction), "abstained", "false_not_found"}."""
    exp = entry["expect"]
    facts, wrong = verdict["facts"], verdict["wrong"]
    present = sum(1 for f in facts if f == "present")
    bad = any(wrong) or any(f == "contradicted" for f in facts)
    if exp == "not_found":
        return {"correct": verdict["abstained"], "recall": None, "wrong": bad or not verdict["abstained"],
                "abstained": verdict["abstained"], "false_not_found": False}
    if exp == "guide":
        ok = verdict["guide_pointer"]
        return {"correct": ok and not bad, "recall": present / len(facts) if facts else None, "wrong": bad,
                "abstained": verdict["abstained"], "false_not_found": verdict["abstained"] and not ok}
    recall = present / len(facts) if facts else None
    return {"correct": present == len(facts) and not bad and not verdict["abstained"], "recall": recall,
            "wrong": bad, "abstained": verdict["abstained"], "false_not_found": verdict["abstained"]}


# ------------------------------------------------------------------ why an answer failed
FAILURES = ("retrieval miss", "writer omission", "check dropped a true claim", "distractor or stale won",
            "format failure", "guide handling")


def classify(entry, verdict, rec, gold, stale_old=None):
    """Why an answer that is not correct went wrong (None when it is correct or has no verdict). entry: the answer
    key; verdict: the judge's; rec: the answer record (answer, passages: [[{shelf, title, ...}]] per part, and for
    the Librarian writer_raw, format_ok, claims_raw, claims_quoted, check_noes, failed_writer); gold: the titles that
    hold the answer; stale_old: the stale pair's older title (its old fact counts as a wrong fact).
    Order matters: the first cause that explains the failure is the one counted."""
    if verdict is None:
        return None
    s = score(entry, verdict)
    if s["correct"]:
        return None
    exp = entry["expect"]
    titles = [h.get("title") for part in rec.get("passages") or [] for h in part]
    found = any(is_gold(t, gold) for t in titles) if gold else None
    librarian = "writer_raw" in rec
    if librarian and (rec.get("failed_writer") or (rec.get("writer_raw") and rec.get("format_ok") is False)):
        return "format failure"
    if exp == "guide":
        return "guide handling"
    if exp == "not_found":
        return "distractor or stale won"
    if stale_old and any(verdict["wrong"]):
        return "distractor or stale won"
    if found is False:
        return "retrieval miss"
    missing = [f for f, v in zip(entry["facts"], verdict["facts"]) if v != "present"]
    if librarian and missing:
        raw = " ".join(c.get("claim", "") + " " + c.get("quote", "") for c in rec.get("claims_raw") or [])
        kept = " ".join(c.get("claim", "") + " " + c.get("quote", "") for c in rec.get("claims_quoted") or [])
        answer = rec.get("answer") or ""
        for f in missing:
            if overlap(f, raw) >= 0.6 and (overlap(f, kept) < 0.6 or rec.get("check_noes")) and overlap(f, answer) < 0.6:
                return "check dropped a true claim"
    if any(verdict["wrong"]) or any(v == "contradicted" for v in verdict["facts"]):
        return "distractor or stale won"
    return "writer omission"


# ------------------------------------------------------------------ estimates
def fmt_secs(s):
    if s is None:
        return "?"
    s = int(round(s))
    if s < 90:
        return "%d s" % s
    if s < 5400:
        return "%d min" % round(s / 60)
    return "%.1f h" % (s / 3600)


# ------------------------------------------------------------------ Markdown
def f2(x, digits=2, unit=""):
    if x is None:
        return "–"
    if isinstance(x, float):
        return ("%." + str(digits) + "f%s") % (x, unit)
    return "%s%s" % (x, unit)


def pc(x):
    return "–" if x is None else "%d%%" % round(100 * x)


def delta(a, b):
    """b - a in percentage points."""
    if a is None or b is None:
        return "–"
    d = round(100 * (b - a))
    return "%+d" % d


def md_table(headers, rows):
    out = ["| " + " | ".join(headers) + " |", "|" + "|".join("---" for _ in headers) + "|"]
    for r in rows:
        out.append("| " + " | ".join(str(c).replace("|", "\\|").replace("\n", " ") for c in r) + " |")
    return "\n".join(out)
