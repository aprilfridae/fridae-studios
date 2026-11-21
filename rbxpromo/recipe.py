"""Promo recipe: load a JSON recipe and check it before anything renders.

  load_recipe("promo.json")   -> dict, or raises RecipeError with .problems
  validate(recipe, base_dir)  -> list of problems (empty = valid)
"""
import json, ntpath, os

STYLES = ("hype", "cinematic", "meme", "countdown", "update-log", "lyric", "showcase", "pov")
CAPTION_SOURCES = ("voice", "music", "none", "words")
HIGHLIGHTS = ("auto", "numbers", "off")


class RecipeError(Exception):
    def __init__(self, problems):
        self.problems = list(problems)
        super().__init__("; ".join(self.problems))


def _num(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool)


def _resolve(path, base_dir):
    # a Windows drive path like C:\\Users\\x is absolute on any OS
    return path if os.path.isabs(path) or ntpath.isabs(path) else os.path.join(base_dir, path)


def scene_duration(scene):
    """Seconds a scene takes on screen."""
    if "title" in scene:
        return float(scene.get("duration", 0))
    return (float(scene["out"]) - float(scene["in"])) / float(scene.get("speed", 1))


def recipe_duration(recipe):
    """Total seconds of all scenes."""
    return sum(scene_duration(s) for s in recipe.get("scenes", []))


def _tolerant_total(scenes):
    """Total of the scenes that parse, so one bad scene does not hide other checks."""
    total = 0.0
    for s in scenes:
        try:
            d = scene_duration(s)
        except (TypeError, ValueError, KeyError, ZeroDivisionError, AttributeError):
            continue
        if d > 0:
            total += d
    return total


def _check_scene(i, s, base_dir, problems):
    if not isinstance(s, dict):
        problems.append("scene %d: must be an object with 'clip' or 'title'." % i)
        return
    if "clip" in s:
        clip = s["clip"]
        if not isinstance(clip, str) or not clip:
            problems.append("scene %d: 'clip' must be a file path." % i)
        elif not os.path.isfile(_resolve(clip, base_dir)):
            problems.append("scene %d: clip file not found: %s" % (i, clip))
        a, b = s.get("in"), s.get("out")
        if not (_num(a) and _num(b)):
            problems.append("scene %d: 'in' and 'out' must be numbers (seconds)." % i)
        elif a >= b:
            problems.append("scene %d: 'in' must be before 'out' (got in=%s, out=%s)." % (i, a, b))
        speed = s.get("speed", 1)
        if not _num(speed) or not 0.25 <= speed <= 4:
            problems.append("scene %d: 'speed' must be between 0.25 and 4." % i)
        crop = s.get("crop")
        if crop is not None:
            if not isinstance(crop, dict):
                problems.append("scene %d: 'crop' must be an object like {\"x\": 0.5, \"track\": \"auto\"}." % i)
            else:
                x = crop.get("x", 0.5)
                if not _num(x) or not 0 <= x <= 1:
                    problems.append("scene %d: crop 'x' must be between 0 and 1." % i)
                if crop.get("track", "none") not in ("auto", "none"):
                    problems.append("scene %d: crop 'track' must be \"auto\" or \"none\"." % i)
    elif "title" in s:
        if not isinstance(s["title"], str) or not s["title"]:
            problems.append("scene %d: 'title' must be text." % i)
        d = s.get("duration")
        if not _num(d) or d <= 0:
            problems.append("scene %d: title scenes need a positive 'duration' in seconds." % i)
        if "style" in s and s["style"] not in STYLES:
            problems.append("scene %d: unknown style '%s'. Allowed: %s." % (i, s["style"], ", ".join(STYLES)))
    else:
        problems.append("scene %d: needs either 'clip' or 'title'." % i)


def _audio_length(path, base_dir, durations):
    if durations is not None:
        for key in (path, _resolve(path, base_dir)):
            if key in durations:
                return durations[key]
        return None
    try:
        from rbxpromo.probe import probe_file
        return float(probe_file(_resolve(path, base_dir))["duration"])
    except Exception:
        return None


def validate(recipe, base_dir, durations=None):
    """Return a list of problems (full sentences); empty means the recipe is valid."""
    problems = []
    if not isinstance(recipe, dict):
        return ["The recipe must be a JSON object."]
    if recipe.get("version") != 1:
        problems.append("'version' must be 1.")
    if recipe.get("size") != [1080, 1920]:
        problems.append("'size' must be [1080, 1920] (vertical).")
    if recipe.get("fps") not in (30, 60):
        problems.append("'fps' must be 30 or 60.")
    if recipe.get("style") not in STYLES:
        problems.append("Unknown style '%s'. Allowed styles: %s." % (recipe.get("style"), ", ".join(STYLES)))
    scenes = recipe.get("scenes")
    if not isinstance(scenes, list) or not scenes:
        problems.append("'scenes' must be a non-empty list.")
        scenes = []
    for i, s in enumerate(scenes):
        _check_scene(i, s, base_dir, problems)

    audio = recipe.get("audio") or {}
    if not isinstance(audio, dict):
        problems.append("'audio' must be an object.")
        audio = {}
    vol = audio.get("music_volume", 0.8)
    if not _num(vol) or not 0 <= vol <= 1:
        problems.append("audio 'music_volume' must be between 0 and 1.")
    total = _tolerant_total(scenes)
    for key in ("music", "voice"):
        p = audio.get(key)
        if p is None:
            continue
        if not isinstance(p, str) or not os.path.isfile(_resolve(p, base_dir)):
            problems.append("audio %s file not found: %s" % (key, p))
            continue
        length = _audio_length(p, base_dir, durations)
        if key == "voice" and length is not None and total > 0 and length > total + 0.05:
            problems.append("The voice is %.1f s but the video is %.1f s; the voice would be cut off. "
                            "Lengthen the scenes or shorten the voice." % (length, total))

    caps = recipe.get("captions")
    if caps is not None:
        if not isinstance(caps, dict):
            problems.append("'captions' must be an object.")
        else:
            if caps.get("source") not in CAPTION_SOURCES:
                problems.append("captions 'source' must be one of: %s." % ", ".join(CAPTION_SOURCES))
            if caps.get("highlight", "auto") not in HIGHLIGHTS:
                problems.append("captions 'highlight' must be one of: %s." % ", ".join(HIGHLIGHTS))
            if caps.get("source") == "words":
                w = caps.get("words")
                if not isinstance(w, str) or not os.path.isfile(_resolve(w, base_dir)):
                    problems.append("captions words file not found: %s" % w)
    for key, fields in (("end_card", ("title", "cta")), ("post", ("caption",))):
        v = recipe.get(key)
        if v is not None:
            if not isinstance(v, dict) or not all(isinstance(v.get(f), str) for f in fields):
                problems.append("'%s' must be an object with text fields: %s." % (key, ", ".join(fields)))
    return problems


def load_recipe(path):
    """Read a recipe file and validate it; paths inside resolve against its folder."""
    try:
        with open(path, encoding="utf-8") as f:
            recipe = json.load(f)
    except OSError as e:
        raise RecipeError(["Could not read recipe %s: %s" % (path, e)])
    except ValueError as e:
        raise RecipeError(["Recipe %s is not valid JSON: %s" % (path, e)])
    problems = validate(recipe, os.path.dirname(os.path.abspath(path)))
    if problems:
        raise RecipeError(problems)
    return recipe
