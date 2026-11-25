"""A three-way line merge of Fridae's own, so Pull newest needs no git. merge3 gives the clean combination of the
user's file and the vault's new one against the version both started from, or None when they clash. It never writes
conflict marks: a clash is asked about, never left inside a note."""
import difflib

MERGE_MAX = 1 << 20
# pictures and models: never combined, whatever their bytes
BINARY_EXT = (".blend", ".fbx", ".glb", ".gltf", ".obj", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".tga", ".bmp",
              ".psd", ".tif", ".tiff", ".ico")


def _text(data):
    """Whether data is UTF-8 text with no NUL."""
    if b"\0" in data:
        return False
    try:
        data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    return True


def is_binary(path, data):
    """True for a file that is never combined: a picture or model by its extension, or bytes that are not text."""
    ext = path[path.rfind("."):].lower() if "." in path.rsplit("/", 1)[-1] else ""
    return ext in BINARY_EXT or not _text(data)


def _key(line):
    """A line as compared: without its line ending, so a file re-saved with Windows endings still lines up."""
    if line.endswith(b"\r\n"):
        return line[:-2]
    if line.endswith(b"\n"):
        return line[:-1]
    return line


def _hunks(base_keys, lines):
    """The changes from base to one side: (i1, i2, the side's lines for base[i1:i2]), and for each base line outside
    them, the side's own line (base index -> side line)."""
    keys = [_key(x) for x in lines]
    hunks, same = [], {}
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(None, base_keys, keys, autojunk=False).get_opcodes():
        if tag == "equal":
            for t in range(i2 - i1):
                same[i1 + t] = lines[j1 + t]
        else:
            hunks.append((i1, i2, lines[j1:j2]))
    return hunks, same


def merge3(local, base, new):
    """The clean three-way merge of three byte strings, or None: changes that touch the same or neighbouring base
    lines (unless they are the same change), any side over MERGE_MAX, or any side that is not UTF-8 text."""
    for d in (local, base, new):
        if len(d) > MERGE_MAX or not _text(d):
            return None
    if local == base or local == new:
        return new if local == base else local
    if new == base:
        return local
    base_lines = base.splitlines(keepends=True)
    base_keys = [_key(x) for x in base_lines]
    mine, same = _hunks(base_keys, local.splitlines(keepends=True))
    theirs, _ = _hunks(base_keys, new.splitlines(keepends=True))
    keep = []
    for b in theirs:
        dup = False
        for a in mine:
            if a[0] <= b[1] and b[0] <= a[1]:  # overlapping or touching
                if (a[0], a[1]) == (b[0], b[1]) and [_key(x) for x in a[2]] == [_key(x) for x in b[2]]:
                    dup = True  # the same change on both sides: taken once, from the user's file
                else:
                    return None
        if not dup:
            keep.append(b)
    out, pos = [], 0
    for i1, i2, lines in sorted(mine + keep, key=lambda h: (h[0], h[1])):
        out.extend(same[i] for i in range(pos, i1))
        out.extend(lines)
        pos = i2
    out.extend(same[i] for i in range(pos, len(base_lines)))
    return b"".join(out)
