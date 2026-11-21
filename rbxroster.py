"""Studio mode's team: the roster of experts and each expert's instruction file.

The kit ships team/roster.json (a list of entries with id, name, job, stage, guides, tools, kind, writes) and
team/experts/<id>.md. A game may override either under <project>/.fridae/team/: a project roster.json entry replaces
only the fields it gives for its id (or adds a new expert), and a project experts/<id>.md replaces that expert's text.
This module only loads and merges; it knows nothing about runs.
"""
import json
import os
import re

import rbxpaths
from rbxpaths import KitError

# an expert's instructions go into its chat's standing prompt (and its saved chat): keep them a sensible size
MAX_TEXT = 16000
KEYS = ("id", "name", "job", "stage", "guides", "tools", "kind", "writes")
_ID = re.compile(r"^[a-z0-9][a-z0-9-]*$")


def kit_dir():
    return os.path.join(rbxpaths.KIT, "team")


def project_dir(project):
    return os.path.join(project, ".fridae", "team")


def _read(path, required):
    """The list of entries in one roster.json, or [] when an optional file is absent. Anything malformed raises."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except FileNotFoundError:
        if required:
            raise KitError("The team roster is missing: %s." % path)
        return []
    except (OSError, ValueError) as e:
        raise KitError("The team roster %s could not be read: %s." % (path, e))
    if not isinstance(data, list):
        raise KitError("The team roster %s must be a JSON list of experts." % path)
    for e in data:
        if not isinstance(e, dict) or not isinstance(e.get("id"), str) or not _ID.match(e["id"]):
            raise KitError("Every entry in the team roster %s needs an 'id' of lowercase letters, digits and dashes "
                           "(found %r)." % (path, e.get("id") if isinstance(e, dict) else e))
    return data


def _no_duplicates(entries, path):
    seen = set()
    for e in entries:
        if e["id"] in seen:
            raise KitError("The team roster %s lists the expert %r twice." % (path, e["id"]))
        seen.add(e["id"])


def _check(entry, path):
    """Raise KitError naming the file, the expert and the field when an entry's values are not usable."""
    import rbxkit_mcp  # lazy: the guide list lives in the tool server
    import rbxstage  # lazy: rbxstage imports this module

    def bad(field, why):
        raise KitError("The team roster %s: expert %r, field %r %s." % (path, entry["id"], field, why))

    for k in ("name", "job", "stage"):
        if not isinstance(entry.get(k), str) or not entry[k].strip():
            bad(k, "must be a non-empty string")
    if entry["stage"] not in rbxstage.STAGES and entry["stage"] != "all":
        bad("stage", "must be one of %s or all (found %r)" % (", ".join(rbxstage.STAGES), entry["stage"]))
    for k in ("guides", "tools"):
        v = entry.get(k)
        if not isinstance(v, list) or not all(isinstance(x, str) and x for x in v):
            bad(k, "must be a list of names")
    unknown = [g for g in entry["guides"] if g not in rbxkit_mcp.GUIDES]
    if unknown:
        bad("guides", "names guides that do not exist: %s" % ", ".join(unknown))
    kinds = ("worker", "critic", "lead") if entry["id"] == "lead" else ("worker", "critic")
    if entry.get("kind") not in kinds:
        bad("kind", "must be %s (found %r)" % (" or ".join(kinds), entry.get("kind")))
    if not isinstance(entry.get("writes"), bool):
        bad("writes", "must be true or false (found %r)" % (entry.get("writes"),))


def load(project=None):
    """The roster: the kit's entries, with a project's entries merged in by id (new ids are added at the end)."""
    path = os.path.join(kit_dir(), "roster.json")
    kit = _read(path, True)
    _no_duplicates(kit, path)
    for e in kit:
        missing = [k for k in KEYS if k not in e]
        if missing:
            raise KitError("The team roster %s: expert %r is missing %s." % (path, e["id"], ", ".join(missing)))
    roster = [{k: e[k] for k in KEYS} for e in kit]
    source = {e["id"]: path for e in roster}
    if project:
        path = os.path.join(project_dir(project), "roster.json")
        mine = _read(path, False)
        _no_duplicates(mine, path)
        by_id = {e["id"]: e for e in roster}
        for e in mine:
            if e["id"] in by_id:
                by_id[e["id"]].update({k: v for k, v in e.items() if k in KEYS})
            else:
                missing = [k for k in KEYS if k not in e]
                if missing:
                    raise KitError("The team roster %s adds the expert %r, which needs %s." % (path, e["id"], ", ".join(missing)))
                entry = {k: e[k] for k in KEYS}
                roster.append(entry)
                by_id[e["id"]] = entry
            source[e["id"]] = path
    for e in roster:
        _check(e, source[e["id"]])
    return roster


def expert(expert_id, project=None):
    """One roster entry plus 'text', its instruction file (the project's copy when there is one)."""
    roster = load(project)
    found = [e for e in roster if e["id"] == (expert_id or "").strip()]
    if not found:
        raise KitError("No expert %r. The team is: %s." % (expert_id, ", ".join(e["id"] for e in roster)))
    entry = dict(found[0])
    paths = [os.path.join(kit_dir(), "experts", entry["id"] + ".md")]
    if project:
        paths.insert(0, os.path.join(project_dir(project), "experts", entry["id"] + ".md"))
    for p in paths:
        try:
            with open(p, encoding="utf-8") as f:
                entry["text"] = f.read()
            if len(entry["text"]) > MAX_TEXT:
                raise KitError("The instructions for %s (%s) are %d characters long; keep them to %s or fewer." % (
                    entry["id"], p, len(entry["text"]), format(MAX_TEXT, ",")))
            return entry
        except FileNotFoundError:
            continue
        except OSError as e:
            raise KitError("The instructions for %s could not be read (%s): %s." % (entry["id"], p, e))
    raise KitError("The expert %r has no instruction file (looked for %s)." % (entry["id"], " and ".join(paths)))
