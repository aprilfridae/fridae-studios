// Voice generation (Kokoro).
//   node voice.mjs --warm                         download and load the model
//   node voice.mjs <text file> <out.wav> <voice>  speak the text (UTF-8 file) into a 24 kHz wav, prints {"ok":true,...}
import { KokoroTTS } from "kokoro-js";
import { env, RawAudio } from "@huggingface/transformers";
import { fileURLToPath } from "node:url";
import { readFileSync } from "node:fs";

const MODEL = "onnx-community/Kokoro-82M-v1.0-ONNX";
// models live in <engine>/hf-cache, beside renderer/, whether or not HF_HOME is set
env.cacheDir = process.env.HF_HOME || fileURLToPath(new URL("../../hf-cache", import.meta.url));

if (process.argv.includes("--warm")) {
  await KokoroTTS.from_pretrained(MODEL, { dtype: "q8" });
  console.log("voice model ready");
  // onnxruntime can abort while the process shuts down (mutex error on macOS) after the model is
  // already cached; the engine installer therefore trusts the line above rather than the exit code.
  process.exit(0);
}

// Sentences grouped up to MAX_CHARS; a longer sentence is cut at commas, then at spaces.
const MAX_CHARS = 250;
function chunkText(text) {
  const pieces = [];
  for (const sentence of text.replace(/\s+/g, " ").trim().match(/[^.!?]+[.!?]*["')\]]*\s*/g) || []) {
    let rest = sentence.trim();
    while (rest.length > MAX_CHARS) {
      let cut = rest.lastIndexOf(",", MAX_CHARS);
      if (cut < 40) cut = rest.lastIndexOf(" ", MAX_CHARS);
      if (cut < 1) cut = MAX_CHARS;
      pieces.push(rest.slice(0, cut + 1).trim());
      rest = rest.slice(cut + 1).trim();
    }
    if (rest) pieces.push(rest);
  }
  const chunks = [];
  for (const p of pieces) {
    if (chunks.length && chunks[chunks.length - 1].length + p.length < MAX_CHARS) chunks[chunks.length - 1] += " " + p;
    else chunks.push(p);
  }
  return chunks;
}

const [textFile, out, voice = "af_heart"] = process.argv.slice(2);
if (!textFile || !out) {
  console.error("usage: node voice.mjs --warm | node voice.mjs <text file> <out.wav> [voice]");
  process.exit(2);
}
const tts = await KokoroTTS.from_pretrained(MODEL, { dtype: "q8" });
// kokoro-js cuts a text off after ~510 tokens, so the script is spoken in chunks and joined
const parts = [];
for (const chunk of chunkText(readFileSync(textFile, "utf8"))) {
  const a = await tts.generate(chunk, { voice });
  parts.push(a.audio);
}
const data = new Float32Array(parts.reduce((n, p) => n + p.length, 0));
let at = 0;
for (const p of parts) { data.set(p, at); at += p.length; }
const audio = new RawAudio(data, 24000);
await audio.save(out);
console.log(JSON.stringify({ ok: true, file: out, sampling_rate: audio.sampling_rate }));
process.exit(0);
