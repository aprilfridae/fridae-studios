"""How much text an agent takes in for a task: the old way (whole guides) against the new way, for both kinds of agent.

Run:  .venv/bin/python tests/knowledge_bench.py
A measuring script, not a unit test. It uses a temporary work folder for the index caches, never the real one.

Old way: the whole guides routing.txt (an ordinary chat) or the expert's file (a Studio expert) named for the task,
as those texts read before the knowledge lookup (commit 9b0e7fd).
New way, measured as the tools answer:
- chat (routing.txt): `workflow` whole, one rbx_vault search on shelf guides, reading its top three sections;
- expert (its file in vault/Fridae/Team): its rule guides whole (one rbx_guide call, at most two topics), one
  rbx_guide call with `outline: true` for its other guides, then the same search and three sections.
"""
import json
import os
import sys
import tempfile
import time
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.dirname(HERE))

# (task, guides the old expert file loaded whole, guides/notes a good answer should come from)
TASKS = [
    ("save player coins between sessions",
     ["workflow", "roblox-luau", "roblox-architecture", "roblox-security", "roblox-networking", "lihfolk-systems",
      "roblox-data-stores"],
     ["roblox-data-stores", "lihfolk-systems"]),
    ("make a shop menu that fits phones",
     ["workflow", "ui-design", "roblox-ui", "web-to-roblox", "unslop", "minimalist-skill"],
     ["roblox-ui", "ui-design", "web-to-roblox", "lihfolk-systems", "player-testing"]),
    ("a sword slash effect",
     ["workflow", "vfx", "vfx-basics", "motion-design", "roblox-characters-animation", "sound-design", "minimalist-skill",
      "lihfolk-vfx"],
     ["vfx", "vfx-basics", "lihfolk-vfx"]),
    ("playtest the tutorial as a new player",
     ["workflow", "playtest", "player-testing", "game-design"],
     ["playtest", "player-testing", "lihfolk-tutorial"]),
    ("price the first three upgrades",
     ["workflow", "economy-design", "roblox-monetization"],
     ["economy-design"]),
    ("review remotes for exploits",
     ["workflow", "critic", "roblox-security", "roblox-code-review"],
     ["roblox-security", "roblox-code-review", "roblox-networking"]),
]

# the old routing.txt's lists for the same tasks (an ordinary chat loaded these whole, in one call)
CHAT_OLD = {
    "save player coins between sessions": ["workflow", "roblox-luau", "roblox-architecture", "roblox-security",
                                           "roblox-networking", "roblox-data-stores", "lihfolk-systems"],
    "make a shop menu that fits phones": ["workflow", "ui-design", "roblox-ui", "web-to-roblox", "minimalist-skill",
                                          "emil-design-eng"],
    "a sword slash effect": ["workflow", "vfx", "vfx-basics", "lihfolk-vfx", "motion-design", "minimalist-skill"],
    "playtest the tutorial as a new player": ["workflow", "playtest", "roblox-testing", "lihfolk-tutorial",
                                              "player-testing"],
    "price the first three upgrades": ["workflow", "game-design", "economy-design"],
    "review remotes for exploits": ["workflow", "roblox-code-review", "roblox-security", "roblox-networking"],
}

# what each expert's file now says: (expert, guides whole, guides as outlines) for the task
EXPERT_NEW = {
    "save player coins between sessions": ("coder", ["workflow"],
                                           ["roblox-luau", "roblox-architecture", "roblox-security", "roblox-networking",
                                            "lihfolk-systems", "roblox-data-stores"]),
    "make a shop menu that fits phones": ("ui", ["workflow"],
                                          ["ui-design", "roblox-ui", "web-to-roblox", "unslop", "minimalist-skill"]),
    "a sword slash effect": ("effects", ["workflow"],
                             ["vfx", "vfx-basics", "motion-design", "roblox-characters-animation", "sound-design",
                              "minimalist-skill", "lihfolk-vfx"]),
    "playtest the tutorial as a new player": ("playtester", ["workflow"], ["playtest", "player-testing", "game-design"]),
    "price the first three upgrades": ("economy", ["workflow"], ["economy-design", "roblox-monetization"]),
    "review remotes for exploits": ("security", ["workflow", "critic"], ["roblox-security", "roblox-code-review"]),
}


def old_chars(guides):
    import rbxkit_mcp
    return sum(len(rbxkit_mcp._one_guide(g) or "") for g in guides)


def guide_call(**a):
    import rbxkit_mcp
    with mock.patch.dict(os.environ, {"RBXKIT_AGENT_ROLE": "worker"}):
        return len(rbxkit_mcp.t_guide(a)["content"][0]["text"])


def lookup_chars(task):
    """One search on shelf guides plus reading its top three sections."""
    import rbxvault
    text = rbxvault.run("search", {"query": task, "shelf": "guides"})
    total = len(text)
    reads = [json.loads(line.split("read: ", 1)[1]) for line in text.splitlines() if line.strip().startswith("read: ")]
    for args in reads[:3]:
        total += len(rbxvault.run("read", args))
    return total


def check_expert_files():
    """The bench's expert model must be what the files say: every guide it loads is named in the file."""
    import rbxroster
    for task, (expert, whole, outline) in EXPERT_NEW.items():
        text = " ".join(rbxroster.expert(expert)["text"].split())
        for g in whole + outline:
            if g != "minimalist-skill":  # "the style guide GAME.md's Art direction names"
                assert "`%s`" % g in text, (expert, g)


def main():
    import rbxpaths
    rbxpaths.WORK = tempfile.mkdtemp(prefix="knowledge-bench ")
    os.environ["RBXKIT_PROJECT"] = ""
    import rbxvault
    t = time.time()
    rbxvault.load_sections()
    t_sec = time.time() - t
    t = time.time()
    rbxvault.load_index()
    t_vault = time.time() - t
    check_expert_files()
    print("Index build (cold): guides+team sections %.2f s, vault notes %.2f s" % (t_sec, t_vault))
    print()
    print("| Task | Chat old | Chat new | Chat % | Expert | Expert old | Expert new | Expert % |")
    print("|---|---:|---:|---:|---|---:|---:|---:|")
    sums = [0, 0, 0, 0]
    for task, expert_old, _ in TASKS:
        look = lookup_chars(task)
        c_old = old_chars(CHAT_OLD[task])
        c_new = guide_call(topic="workflow") + look
        expert, whole, outline = EXPERT_NEW[task]
        e_old = old_chars(expert_old)
        e_new = guide_call(topics=whole) + guide_call(topics=outline, outline=True) + look
        sums = [a + b for a, b in zip(sums, (c_old, c_new, e_old, e_new))]
        print("| %s | %s | %s | %d%% | %s | %s | %s | %d%% |" % (
            task, format(c_old, ","), format(c_new, ","), round(100 * c_new / c_old), expert, format(e_old, ","),
            format(e_new, ","), round(100 * e_new / e_old)))
    print("| **Total** | %s | %s | %d%% | | %s | %s | %d%% |" % (
        format(sums[0], ","), format(sums[1], ","), round(100 * sums[1] / sums[0]), format(sums[2], ","),
        format(sums[3], ","), round(100 * sums[3] / sums[2])))


if __name__ == "__main__":
    main()
