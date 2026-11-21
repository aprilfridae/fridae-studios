// Voice generation (Kokoro).
//   node voice.mjs --warm                         download and load the model
//   node voice.mjs <text file> <out.wav> <voice>  speak the text (UTF-8 file) into a 24 kHz wav, prints {"ok":true,...}
import { KokoroTTS } from "kokoro-js";
import { env } from "@huggingface/transformers";
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

const [textFile, out, voice = "af_heart"] = process.argv.slice(2);
if (!textFile || !out) {
  console.error("usage: node voice.mjs --warm | node voice.mjs <text file> <out.wav> [voice]");
  process.exit(2);
}
const tts = await KokoroTTS.from_pretrained(MODEL, { dtype: "q8" });
const audio = await tts.generate(readFileSync(textFile, "utf8"), { voice });
await audio.save(out);
console.log(JSON.stringify({ ok: true, file: out, sampling_rate: audio.sampling_rate }));
process.exit(0);
