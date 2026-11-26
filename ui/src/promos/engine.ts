/* The promo engine: downloaded on first use. Its install runs in the background and resumes where it stopped. What it
   says goes in the store: Settings, Connections shows the longer words, the top of the Promos tab the short ones. */
import { api } from "../api";
import { store } from "../store";

const STEPS: Record<string, string> = { node: "Node.js", renderer: "the Remotion renderer", browser: "the headless browser", whisper: "the speech recognition model", voice: "the voice model" };
const ORDER = Object.keys(STEPS);
let engine: any = null, checkError = "", job: any = {};

function paint() {
  const label = (s: string) => STEPS[s] || "the promo engine";
  let text: string, short: string, button = "", error = "", bar = -1, ready = false;
  if (job.running) {
    const at = Math.max(0, ORDER.indexOf(job.step)), f = job.fraction || 0;
    text = short = job.step ? "Installing " + label(job.step) + " (part " + (at + 1) + " of " + ORDER.length + ", " + Math.round(f * 100) + "%)." : "Starting the install...";
    bar = Math.round(((at + f) / ORDER.length) * 100); button = "Installing...";
  } else if (job.error) {
    text = short = "Could not install " + label(job.step) + ". Retry continues where it stopped.";
    error = job.error; button = "Retry";
  } else if (!engine) {
    text = checkError ? "Could not check the promo engine." : "Checking...";
    short = checkError ? text : "Checking the promo engine...";
    error = checkError;
  } else if (engine.ready) {
    text = "Ready. Promo videos are made on this computer.";
    short = "Promo engine ready"; ready = true;
  } else {
    const all = engine.missing.length === ORDER.length;
    text = (all ? "Not installed" : "Partly installed (missing " + engine.missing.map(label).join(", ") + ")") + ": about " + engine.download_mb + " MB " + (all ? "to download" : "left to download") + ". Promos need it.";
    short = "Promo engine " + (all ? "not installed" : "partly installed") + " (about " + engine.download_mb + " MB" + (all ? "" : " left") + ")";
    button = all ? "Install" : "Continue install";
  }
  /* where it lives is a tooltip: a long path, and nothing to act on */
  const title = engine && engine.folder ? "The engine's folder: " + engine.folder + ". Nothing is installed system-wide." : "";
  store.set({ engine: { text, short, button, error, bar, ready, running: !!job.running, title } });
}
export async function loadEngine() {
  try { const r = await api("/api/promo-engine", { action: "status" }); engine = r.engine || null; checkError = r.error || ""; job = r.install || {}; } catch (e) {}
  paint();
}
/* the app's promo_engine event: a step of the install, its end, or what stopped it */
export function onEngine(e: any) {
  if (e.failed) job = { running: false, step: e.step, error: e.error };
  else if (e.done || e.stopped) { job = {}; loadEngine(); }   /* stopped: the app is closing; the row then shows what is left */
  else job = { running: true, step: e.step, fraction: e.fraction };
  paint();
}
export async function engineInstall() {
  job = { running: true }; paint();
  const r = await api("/api/promo-engine", { action: "install" });
  if (r && r.install) { job = r.install; paint(); }
}
export const engineReady = () => !!(engine && engine.ready);
export const engineKnown = () => !!engine;
