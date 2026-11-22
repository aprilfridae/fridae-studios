"""Inventory of the bundled Roblox vault, to decide what is fat: a measuring script, not a unit test.

Run:  .venv/bin/python tests/vault_inventory.py [--json out.json]
Prints notes and bytes per top-level folder, exact duplicates, near-duplicates (one note's text inside another's),
stubs, link-list pages, broken [[wikilinks]], and the notes that no broad query returns in its top 10.
Uses a temporary work folder for the index cache.
"""
import hashlib
import json
import os
import re
import sys
import tempfile
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

QUERIES = [
    # the bench tasks
    "save player coins between sessions", "make a shop menu that fits phones", "a sword slash effect",
    "playtest the tutorial as a new player", "price the first three upgrades", "review remotes for exploits",
    # scripting
    "module script structure", "strict type checking errors", "coroutines and task.spawn", "connection memory leak",
    "OOP classes in luau", "string formatting and patterns", "random numbers and seeds", "event bus signals",
    # data
    "data store throttling budget", "leaderboard top players", "migrate old save data", "player data reset",
    "memorystore matchmaking queue", "trading items between players safely",
    # ui
    "health bar hud", "button hover animation", "inventory grid ui", "notification toast popup",
    "gamepad menu selection", "loading screen", "settings menu", "font and text scaling",
    # building and art
    "build a lobby map", "terrain and water", "lighting for a horror map", "low poly trees in blender",
    "import fbx model into studio", "materials and surface appearance", "level design for an obby",
    # vfx and audio
    "fireball projectile effect", "particle emitter settings", "beams and trails", "screen shake camera",
    "background music and sound effects", "footstep sounds",
    # animation
    "animate a custom rig", "walk cycle animation", "cutscene camera", "ragdoll on death",
    # gameplay
    "npc pathfinding enemy", "round based game loop", "pet hatching eggs", "quests and daily rewards",
    "combat hitbox detection", "vehicle physics car", "tycoon droppers and conveyors", "rebirth system",
    # monetisation and live ops
    "gamepass purchase receipt", "developer products robux", "premium payouts", "limited time event",
    "analytics funnel retention", "game icon and thumbnails click through",
    # performance
    "lag on phones", "streaming enabled", "too many parts performance", "server memory usage",
    # publishing and platform
    "publish the game and age rating", "teleport between places", "text chat filtering", "badges",
    "content maturity questionnaire",
    # debugging
    "infinite yield possible", "rubber banding movement", "remote event not firing", "script not running",
    "data not saving when player leaves",
]


def body_of(text):
    text = re.sub(r"^---\n.*?\n---\n", "", text, flags=re.S)
    text = re.sub(r"<!-- related:start -->.*?<!-- related:end -->", "", text, flags=re.S)
    text = re.sub(r"<!-- tldr:start -->.*?<!-- tldr:end -->", "", text, flags=re.S)
    return text


def norm(text):
    return " ".join(re.sub(r"[^\w\s]", " ", text.lower()).split())


def main():
    import rbxpaths
    rbxpaths.WORK = tempfile.mkdtemp(prefix="vault-inventory ")
    import rbxvault
    kb = rbxvault.KB
    files = rbxvault.note_files()
    root_extra = [os.path.join(kb, f) for f in sorted(os.listdir(kb)) if f.endswith(".md") and f in rbxvault.SKIP_ROOT]
    all_md = files + root_extra
    out = {}

    per = defaultdict(lambda: [0, 0])
    total_bytes = 0
    for root, dirs, fs in os.walk(kb):
        for f in fs:
            p = os.path.join(root, f)
            rel = os.path.relpath(p, kb)
            top = rel.split(os.sep)[0] if os.sep in rel else "(root)"
            n = os.path.getsize(p)
            total_bytes += n
            per[top][1] += n
            if f.endswith(".md"):
                per[top][0] += 1
    out["folders"] = {k: {"notes": v[0], "bytes": v[1]} for k, v in sorted(per.items())}
    out["total"] = {"md": len(all_md), "indexed": len(files), "bytes": total_bytes}

    texts = {p: rbxvault.read(p) for p in all_md}
    bodies = {p: norm(body_of(t)) for p, t in texts.items()}
    rel = lambda p: os.path.relpath(p, kb).replace(os.sep, "/")

    by_hash = defaultdict(list)
    for p, b in bodies.items():
        by_hash[hashlib.sha1(b.encode()).hexdigest()].append(rel(p))
    out["exact_duplicates"] = [v for v in by_hash.values() if len(v) > 1]

    # near-duplicates: shingles of 8 words; a note whose shingles are >= 90 % inside another note's
    sh = {}
    for p, b in bodies.items():
        w = b.split()
        sh[p] = {" ".join(w[i:i + 8]) for i in range(max(0, len(w) - 7))}
    inv = defaultdict(set)
    for p, s in sh.items():
        for x in s:
            inv[x].add(p)
    near = []
    for p, s in sh.items():
        if len(s) < 20:
            continue
        c = Counter(q for x in s for q in inv[x] if q != p)
        for q, n in c.most_common(3):
            if n / len(s) >= 0.9:
                near.append({"note": rel(p), "inside": rel(q), "share": round(n / len(s), 2),
                             "bytes": len(texts[p].encode())})
    out["near_duplicates"] = near

    stubs = []
    for p, t in texts.items():
        b = body_of(t)
        prose = re.sub(r"^#.*$|\[\[[^\]]*\]\]|^\s*[-*]\s*$", "", b, flags=re.M)
        if len(norm(prose)) < 400:
            stubs.append({"note": rel(p), "chars": len(norm(prose))})
    out["stubs"] = sorted(stubs, key=lambda x: x["chars"])

    linky = []
    for p, t in texts.items():
        b = body_of(t)
        lines = [l for l in b.splitlines() if l.strip() and not l.startswith("#")]
        if not lines:
            continue
        link_lines = sum(1 for l in lines if "[[" in l and len(re.sub(r"\[\[[^\]]*\]\]", "", l).strip(" -*|:·,")) < 40)
        if link_lines / len(lines) >= 0.6 and len(lines) >= 5:
            linky.append({"note": rel(p), "link_line_share": round(link_lines / len(lines), 2),
                          "bytes": len(t.encode())})
    out["link_lists"] = linky

    titles = {os.path.basename(p)[:-3] for p in all_md}
    other = set()
    for root, _, fs in os.walk(kb):
        for f in fs:
            other.add(f)
            other.add(os.path.splitext(f)[0])
    broken = Counter()
    count = 0
    for p, t in texts.items():
        for m in re.finditer(r"\[\[([^\]|#]+)", t):
            count += 1
            target = m.group(1).strip()
            if target not in titles and target not in other and os.path.basename(target) not in titles:
                broken[target] += 1
    out["wikilinks"] = {"total": count, "broken": sum(broken.values()), "broken_targets": len(broken),
                        "top_broken": broken.most_common(15)}

    idx = rbxvault.load_index()
    seen = Counter()
    for q in QUERIES:
        for _, d in rbxvault.search(q, 10, idx)[0]:
            seen[d["path"]] += 1
    out["queries"] = len(QUERIES)
    never = [d["path"] for d in idx["docs"] if d["path"] not in seen]
    nf = Counter(p.split("/")[0] if "/" in p else "(root)" for p in never)
    out["never_top10"] = {"count": len(never), "by_folder": dict(nf.most_common())}
    out["never_top10_notes"] = never

    if "--json" in sys.argv:
        with open(sys.argv[sys.argv.index("--json") + 1], "w", encoding="utf-8") as f:
            json.dump(out, f, indent=1, ensure_ascii=False)
    print("Notes: %d Markdown files (%d indexed), %.1f MB in all" % (len(all_md), len(files), total_bytes / 1e6))
    print("\n| Folder | Notes | KB |\n|---|---:|---:|")
    for k, v in out["folders"].items():
        print("| %s | %d | %d |" % (k, v["notes"], v["bytes"] // 1024))
    print("\nExact duplicates:", out["exact_duplicates"] or "none")
    print("Near-duplicates (>= 90%% of one note inside another): %d" % len(near))
    for x in near:
        print("  %s  inside  %s  (%.0f%%, %d B)" % (x["note"], x["inside"], 100 * x["share"], x["bytes"]))
    print("Stubs (< 400 chars of prose): %d" % len(stubs))
    for x in out["stubs"]:
        print("  %s (%d)" % (x["note"], x["chars"]))
    print("Link-list pages: %d" % len(linky))
    for x in linky:
        print("  %s (%.0f%% link lines, %d B)" % (x["note"], 100 * x["link_line_share"], x["bytes"]))
    print("Wikilinks: %(total)d, broken %(broken)d (%(broken_targets)d targets)" % out["wikilinks"])
    print("Never in a top 10 over %d queries: %d notes" % (len(QUERIES), len(never)))
    for k, v in nf.most_common(20):
        print("  %s: %d" % (k, v))


if __name__ == "__main__":
    main()
