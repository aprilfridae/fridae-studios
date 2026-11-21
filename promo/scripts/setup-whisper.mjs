// Install whisper.cpp and one model into <dir>.   node setup-whisper.mjs <dir> [model]
import path from "node:path";
import { installWhisperCpp, downloadWhisperModel } from "@remotion/install-whisper-cpp";

const WHISPER_VERSION = "1.5.5";
const dir = process.argv[2];
const model = process.argv[3] || "base.en";
if (!dir) {
  console.error("usage: node setup-whisper.mjs <dir> [model]");
  process.exit(2);
}

await installWhisperCpp({ to: path.join(dir, "whisper.cpp"), version: WHISPER_VERSION, printOutput: false });
await downloadWhisperModel({ model, folder: dir, printOutput: false });
console.log("whisper ready");
