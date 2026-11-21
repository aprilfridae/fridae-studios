// Speech recognition model (Whisper through transformers.js). Task 5 adds transcription.
//   node setup-whisper.mjs --warm   download the model into the cache and transcribe 1 s of silence
import { pipeline, env } from "@huggingface/transformers";
import { fileURLToPath } from "node:url";

// the _timestamped export carries the cross attentions that word-level timestamps need
const MODEL = "onnx-community/whisper-base.en_timestamped";
// models live in <engine>/hf-cache, beside renderer/, whether or not HF_HOME is set
env.cacheDir = process.env.HF_HOME || fileURLToPath(new URL("../../hf-cache", import.meta.url));

if (process.argv.includes("--warm")) {
  const asr = await pipeline("automatic-speech-recognition", MODEL, { dtype: "q8" });
  await asr(new Float32Array(16000), { return_timestamps: "word" });
  console.log("whisper model ready");
  process.exit(0);
}

console.error("usage: node setup-whisper.mjs --warm");
process.exit(2);
