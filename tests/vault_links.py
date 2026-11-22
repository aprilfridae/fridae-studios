"""Check the links inside a set of Markdown folders the way Obsidian resolves them: a measuring script, not a unit test.

Run:  .venv/bin/python tests/vault_links.py [folder ...]     (default: vault/)
Counts [[wikilinks]] (and ![[embeds]]) that match no file name in the folders, and relative Markdown links
[text](path) whose target file does not exist. Several folders are checked as one vault.
"""
import os
import re
import sys
from collections import Counter
from urllib.parse import unquote

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
WIKI = re.compile(r"!?\[\[([^\]|#^]+)(?:[#^][^\]|]*)?(?:\|[^\]]*)?\]\]")
MDLINK = re.compile(r"(?<!!)\[[^\]]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
FENCE = re.compile(r"```.*?```|~~~.*?~~~|`[^`\n]*`", re.S)


def check(folders):
    names, paths, md = set(), set(), []
    for top in folders:
        for root, dirs, files in os.walk(top):
            dirs[:] = [d for d in dirs if not d.startswith(".")]
            for f in files:
                p = os.path.join(root, f)
                paths.add(os.path.normcase(os.path.abspath(p)))
                names.add(f.lower())
                stem, ext = os.path.splitext(f)
                if ext.lower() == ".md":
                    names.add(stem.lower())
                    md.append(p)
    wiki, wiki_bad, rel, rel_bad = 0, Counter(), 0, Counter()
    for p in md:
        with open(p, encoding="utf-8", errors="replace") as f:
            text = FENCE.sub("", f.read())
        for m in WIKI.finditer(text):
            wiki += 1
            target = m.group(1).strip().replace("\\", "")
            base = os.path.basename(target).lower()
            if base not in names and os.path.splitext(base)[0] not in names:
                wiki_bad[target] += 1
        for m in MDLINK.finditer(text):
            target = m.group(1)
            if re.match(r"^[a-z][a-z0-9+.-]*:", target, re.I) or target.startswith(("#", "/", "<", "{{")):
                continue
            rel += 1
            path = unquote(target.split("#", 1)[0])
            if path and os.path.normcase(os.path.abspath(os.path.join(os.path.dirname(p), path))) not in paths:
                rel_bad[os.path.relpath(p, ROOT) + " -> " + target] += 1
    return {"files": len(md), "wikilinks": wiki, "wikilinks_broken": sum(wiki_bad.values()),
            "wiki_targets_broken": len(wiki_bad), "relative_links": rel, "relative_broken": sum(rel_bad.values()),
            "top_wiki_broken": wiki_bad.most_common(10), "relative_broken_list": sorted(rel_bad)}


def main():
    folders = [os.path.join(ROOT, f) for f in (sys.argv[1:] or ["vault"])]
    r = check(folders)
    print("Markdown files: %(files)d" % r)
    print("Wikilinks: %(wikilinks)d, broken %(wikilinks_broken)d (%(wiki_targets_broken)d targets)" % r)
    print("Relative links: %(relative_links)d, broken %(relative_broken)d" % r)
    for t, n in r["top_wiki_broken"]:
        print("  [[%s]] x%d" % (t, n))
    for x in r["relative_broken_list"]:
        print("  " + x)


if __name__ == "__main__":
    main()
