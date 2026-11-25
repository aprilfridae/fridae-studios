"""The shared Fridae vault: what was downloaded from it, as the base record remembers it.

Task 1 holds only the reading of that record; the download and the updates grow here.
"""
import os

import rbxpaths

BASE_NAME = "base.json"


def base_path():
    return os.path.join(rbxpaths.KNOWLEDGE_DIR, ".fridae-vault", BASE_NAME)


def read_base():
    """The base record (<Knowledge>/.fridae-vault/base.json), every field filled: a missing or broken file, or a
    field of the wrong type, gives that field's empty value."""
    base = {"format": 1, "release": "", "files": {}, "deleted": [], "declined": [], "renamed": {}, "waiting": {},
            "patch": {}}
    data = rbxpaths.read_json(base_path())
    if not isinstance(data, dict):
        return base
    for key, empty in base.items():
        val = data.get(key)
        if key == "format":
            if isinstance(val, int) and not isinstance(val, bool):
                base[key] = val
        elif key == "release":
            if isinstance(val, str):
                base[key] = val
        elif isinstance(val, type(empty)):
            base[key] = val
    return base
