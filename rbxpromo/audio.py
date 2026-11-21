"""Local voiceover (Kokoro) and word timing (Whisper) for the promo maker.

Both run in the engine's Node (promo/scripts/voice.mjs, transcribe.mjs). Results are cached in
engine_dir()/cache/ so the same text or the same file is never processed twice.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import tempfile

import rbxpaths
from rbxpromo import engine

_CACHE_VERSION = "1"  # bump when the models or the output format change
_TIMEOUT = 900


def _cache_dir():
    d = os.path.join(engine.engine_dir(), "cache")
    os.makedirs(d, exist_ok=True)
    return d


def _key(*parts):
    h = hashlib.sha256(_CACHE_VERSION.encode() + b"\0")
    for p in parts:
        h.update(p if isinstance(p, bytes) else str(p).encode("utf-8"))
        h.update(b"\0")
    return h.hexdigest()


def _file_key(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def _atomic_copy(src, dst):
    os.makedirs(os.path.dirname(os.path.abspath(dst)), exist_ok=True)
    shutil.copyfile(src, dst + ".tmp")
    os.replace(dst + ".tmp", dst)


def _run_node(script, *args):
    """Run an engine script; success is its JSON line with "ok" (onnxruntime may abort at exit afterwards)."""
    engine._require_ready()
    cmd = engine.node_cmd(os.path.join("scripts", script), *args)
    try:
        r = subprocess.run(cmd, cwd=os.path.join(engine.engine_dir(), "renderer"), env=engine.node_env(),
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8",
                           errors="replace", timeout=_TIMEOUT, creationflags=rbxpaths.no_window())
    except (OSError, subprocess.TimeoutExpired) as e:
        raise engine.EngineError(f"{script} could not run: {e}")
    for line in reversed((r.stdout or "").splitlines()):
        if line.startswith("{"):
            try:
                data = json.loads(line)
            except ValueError:
                continue
            if data.get("ok"):
                return data
    tail = "\n".join(((r.stderr or "") + (r.stdout or "")).strip().splitlines()[-10:])
    raise engine.EngineError(f"{script} failed (exit {r.returncode}). If the engine is half-installed run "
                             f"`python3 -m rbxpromo.engine install`.\n{tail}")


def _decode_16k(path, out_wav):
    r = subprocess.run([engine.ffmpeg(), "-y", "-v", "error", "-i", path, "-vn", "-ac", "1", "-ar", "16000",
                        "-c:a", "pcm_s16le", "-f", "wav", out_wav],
                       env=engine.tool_env(), capture_output=True)
    if r.returncode != 0:
        raise ValueError(f"Could not read audio from {path}: " + r.stderr.decode("utf-8", "replace").strip()[-300:])


def _to_pcm16(src, dst):
    """Kokoro writes 32-bit float wav; 16-bit PCM at the same 24 kHz opens everywhere."""
    r = subprocess.run([engine.ffmpeg(), "-y", "-v", "error", "-i", src, "-c:a", "pcm_s16le", "-f", "wav", dst],
                       env=engine.tool_env(), capture_output=True)
    if r.returncode != 0:
        raise engine.EngineError("Could not convert the voice to wav: " + r.stderr.decode("utf-8", "replace").strip()[-300:])


def transcribe(path, lang="en"):
    """Word timing of an audio (or video) file: {"words": [{"text","start","end"}], "duration": seconds}.

    The speech model is English-only (whisper base.en). Any `lang` still runs, but non-English speech
    comes out as poor English guesses. A silent file gives no words."""
    if not os.path.isfile(path):
        raise ValueError(f"Audio file not found: {path}")
    engine._require_ready()
    cached = os.path.join(_cache_dir(), "words-" + _key("transcribe", _file_key(path)) + ".json")
    data = rbxpaths.read_json(cached)
    if isinstance(data, dict) and "words" in data and "duration" in data:
        return data
    with tempfile.TemporaryDirectory() as tmp:
        wav = os.path.join(tmp, "in16k.wav")
        _decode_16k(path, wav)
        out = _run_node("transcribe.mjs", wav)
    data = {"words": out["words"], "duration": out["duration"]}
    rbxpaths.write_json(cached, data)
    return data


def _norm(s):
    return re.sub(r"[^a-z0-9]", "", s.lower())


def align_to_script(text, words):
    """Put Whisper's timing onto the words of a known script.

    Whisper sometimes glues words together ("worldplay"); the voiceover script is known, so each script
    word gets the time span of its letters. If the letters of the script and of Whisper's words do not
    match, Whisper's own words are returned unchanged."""
    tokens = [t for t in str(text).split() if _norm(t)]
    if not words or "".join(_norm(t) for t in tokens) != "".join(_norm(w["text"]) for w in words):
        return words
    times = []  # (start, end) of every letter, spread evenly over its Whisper word
    for w in words:
        n = len(_norm(w["text"]))
        for i in range(n):
            times.append((w["start"] + (w["end"] - w["start"]) * i / n,
                          w["start"] + (w["end"] - w["start"]) * (i + 1) / n))
    out, pos = [], 0
    for t in tokens:
        m = len(_norm(t))
        out.append({"text": t, "start": round(times[pos][0], 3), "end": round(times[pos + m - 1][1], 3)})
        pos += m
    return out


def voice(text, out_wav, voice="af_heart"):
    """Speak `text` with the local Kokoro voice into `out_wav` (24 kHz wav).
    Returns {"file", "duration", "words": [{"text","start","end"}]}."""
    if not str(text).strip():
        raise ValueError("There is no text to speak.")
    engine._require_ready()
    key = _key("voice", text, voice)
    cached_wav = os.path.join(_cache_dir(), "voice-" + key + ".wav")
    if not os.path.isfile(cached_wav):
        with tempfile.TemporaryDirectory() as tmp:
            txt, raw, wav = (os.path.join(tmp, n) for n in ("text.txt", "raw.wav", "voice.wav"))
            with open(txt, "w", encoding="utf-8") as f:
                f.write(str(text))
            _run_node("voice.mjs", txt, raw, voice)
            if not os.path.isfile(raw):
                raise engine.EngineError("The voice model finished without writing a sound file.")
            _to_pcm16(raw, wav)
            _atomic_copy(wav, cached_wav)
    words = transcribe(cached_wav)
    heard = sum(len(_norm(w["text"])) for w in words["words"])
    wanted = sum(len(_norm(t)) for t in str(text).split())
    if wanted and heard < 0.8 * wanted:
        raise engine.EngineError(f"The voice covered only part of the script ({heard} of {wanted} letters were "
                                 "recognised). Shorten the text or try again.")
    _atomic_copy(cached_wav, out_wav)
    return {"file": out_wav, "duration": words["duration"], "words": align_to_script(text, words["words"])}
