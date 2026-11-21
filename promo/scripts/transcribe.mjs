// Word timing with Whisper (transformers.js).
//   node transcribe.mjs <16 kHz mono 16-bit wav>   prints one JSON line: {"ok":true,"words":[...],"duration":s}
// The model is English-only (base.en); other languages still run but come out as poor English guesses.
import { pipeline, env } from "@huggingface/transformers";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { windows } from "./windows.mjs";

const MODEL = "onnx-community/whisper-base.en_timestamped";
env.cacheDir = process.env.HF_HOME || fileURLToPath(new URL("../../hf-cache", import.meta.url));
const SILENT_RMS = 0.002; // below this Whisper invents text ("Thank you."), so no words are reported

export function readWav16(path) {
  const b = readFileSync(path);
  let p = 12, rate = 16000, data = null;
  while (p + 8 <= b.length) {
    const id = b.toString("ascii", p, p + 4), size = b.readUInt32LE(p + 4);
    if (id === "fmt ") rate = b.readUInt32LE(p + 12);
    if (id === "data") { data = b.subarray(p + 8, Math.min(b.length, p + 8 + size)); break; }
    p += 8 + size + (size & 1);
  }
  if (!data) throw new Error("no audio data in " + path);
  const n = data.length >> 1, out = new Float32Array(n);
  for (let i = 0; i < n; i++) out[i] = data.readInt16LE(i * 2) / 32768;
  return { samples: out, rate };
}

const path = process.argv[2];
if (!path) { console.error("usage: node transcribe.mjs <wav>"); process.exit(2); }
const { samples, rate } = readWav16(path);
const duration = samples.length / rate;
let sum = 0;
for (const v of samples) sum += v * v;
const rms = samples.length ? Math.sqrt(sum / samples.length) : 0;
let words = [];
if (rms >= SILENT_RMS) {
  const asr = await pipeline("automatic-speech-recognition", MODEL, { dtype: "q8" });
  // Whisper takes 30 s at a time, and transformers.js word times collapse after its own chunk
  // boundary, so the audio is cut into windows here (at the quietest moment near each end) and
  // the times are shifted back.
  for (const [from, to] of windows(samples, rate)) {
    const r = await asr(samples.subarray(from, to), { return_timestamps: "word" });
    const off = from / rate, limit = to / rate;
    for (const c of r.chunks || []) {
      const text = String(c.text).trim(), s0 = c.timestamp[0];
      if (!text || !Number.isFinite(s0)) continue;
      const e0 = Number.isFinite(c.timestamp[1]) ? Math.max(s0, c.timestamp[1]) : s0;
      // a word can never lie outside its window or before the one that came before it
      const start = Math.max(words.length ? words[words.length - 1].start : 0, Math.min(limit, off + s0));
      words.push({ text, start: +start.toFixed(3), end: +Math.max(start, Math.min(limit, off + e0)).toFixed(3) });
    }
  }
}
console.log(JSON.stringify({ ok: true, words, duration: +duration.toFixed(3) }));
// onnxruntime can abort while the process shuts down; the caller trusts the line above, not the exit code.
process.exit(0);
