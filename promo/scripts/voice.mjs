// Voice generation. Task 4 adds speech synthesis; for now only --warm (download and load the model).
import { KokoroTTS } from "kokoro-js";
import { env } from "@huggingface/transformers";

const MODEL = "onnx-community/Kokoro-82M-v1.0-ONNX";
if (process.env.HF_HOME) env.cacheDir = process.env.HF_HOME;

if (process.argv.includes("--warm")) {
  await KokoroTTS.from_pretrained(MODEL, { dtype: "q8" });
  console.log("voice model ready");
  // onnxruntime can abort while the process shuts down (mutex error on macOS) after the model is
  // already cached; the engine installer therefore trusts the line above rather than the exit code.
  process.exit(0);
}

console.error("usage: node voice.mjs --warm");
process.exit(2);
