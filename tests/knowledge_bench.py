"""How much text an agent takes in for a task: the old way (whole guides) against the new way (one search, top 3 sections).

Run:  .venv/bin/python tests/knowledge_bench.py
It is a measuring script, not a unit test. It uses a temporary work folder for the index caches, never the real one.
The guide lists under "old" are what routing.txt or the expert file told an agent to load whole for that task.
"""
import os
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

# (task, guides the old way loaded whole, guides/notes a good answer should come from)
TASKS = [
    ("save player coins between sessions",
     ["workflow", "roblox-luau", "roblox-architecture", "roblox-security", "roblox-networking", "lihfolk-systems",
      "roblox-data-stores"],  # coder expert file + routing (saving)
     ["roblox-data-stores", "lihfolk-systems"]),
    ("make a shop menu that fits phones",
     ["workflow", "ui-design", "roblox-ui", "web-to-roblox", "unslop", "minimalist-skill"],  # ui expert + a style guide
     ["roblox-ui", "ui-design", "web-to-roblox", "lihfolk-systems", "player-testing"]),
    ("a sword slash effect",
     ["workflow", "vfx", "vfx-basics", "motion-design", "roblox-characters-animation", "sound-design", "lihfolk-vfx"],
     ["vfx", "vfx-basics", "lihfolk-vfx"]),  # effects expert (+ lihfolk-vfx for combat effects)
    ("playtest the tutorial as a new player",
     ["workflow", "playtest", "player-testing", "game-design", "lihfolk-tutorial"],  # playtester + routing (tutorials)
     ["playtest", "player-testing", "lihfolk-tutorial"]),
    ("price the first three upgrades",
     ["workflow", "economy-design", "roblox-monetization"],  # economy expert
     ["economy-design"]),
    ("review remotes for exploits",
     ["workflow", "critic", "roblox-security", "roblox-code-review"],  # security expert
     ["roblox-security", "roblox-code-review", "roblox-networking"]),
]


def old_chars(guides):
    import rbxkit_mcp
    return sum(len(rbxkit_mcp._one_guide(g) or "") for g in guides)


def new_chars(task):
    """Characters of one search (shelf guides) plus reading its top three sections."""
    import json
    import rbxvault
    text = rbxvault.run("search", {"query": task, "shelf": "guides"})
    total = len(text)
    reads = [json.loads(line.split("read: ", 1)[1]) for line in text.splitlines() if line.strip().startswith("read: ")]
    for args in reads[:3]:
        total += len(rbxvault.run("read", args))
    return total


def main():
    import rbxpaths
    rbxpaths.WORK = tempfile.mkdtemp(prefix="knowledge-bench ")
    import rbxkit_mcp
    import rbxvault
    t = time.time()
    rbxvault.load_sections()
    t_sec = time.time() - t
    t = time.time()
    rbxvault.load_index()
    t_vault = time.time() - t
    workflow = len(rbxkit_mcp.guide_text("workflow"))
    print("Index build (cold): guides+team sections %.2f s, vault notes %.2f s" % (t_sec, t_vault))
    print()
    print("| Task | Old: whole guides | New: workflow + search + 3 sections | Old without workflow | New without workflow |")
    print("|---|---:|---:|---:|---:|")
    sums = [0, 0, 0, 0]
    for task, guides, _ in TASKS:
        old = old_chars(guides)
        new = new_chars(task)
        row = [old, workflow + new, old - workflow, new]
        sums = [a + b for a, b in zip(sums, row)]
        print("| %s | %s | %s | %s | %s |" % ((task,) + tuple(format(x, ",") for x in row)))
    print("| **Total** | %s | %s | %s | %s |" % tuple(format(x, ",") for x in sums))
    print()
    print("New way is %.0f%% of the old (%.0f%% leaving out workflow, which both ways load whole)." % (
        100 * sums[1] / sums[0], 100 * sums[3] / sums[2]))


if __name__ == "__main__":
    main()
