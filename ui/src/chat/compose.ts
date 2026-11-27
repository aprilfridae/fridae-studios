/* What the message box knows and does: the AI's model and effort, how it works (Ask me, Full auto, Overnight,
   Studio and the two extras), the files waiting to go with the next message, and sending. */
import { api, TOKEN } from "../api";
import { H } from "./host";
import { poke } from "./chatStore";
import { at, nameOf, online } from "../remote/route";
import { boxMachine } from "../remote/remote";

export const MODELS: [string, string, string][] = [
  ["fable", "Fable", "Most capable; best for big or tricky builds"],
  ["opus", "Opus", "Strong all-rounder"],
  ["sonnet", "Sonnet", "Fast and capable; good everyday choice"],
  ["haiku", "Haiku", "Fastest and cheapest; small, simple jobs"],
];
export const EFFORTS: [string, string][] = [["low", "Low"], ["medium", "Medium"], ["high", "High"], ["xhigh", "Extra"], ["max", "Max"]];
export const EFFORT_NAMES: Record<string, string> = { low: "low effort", medium: "medium effort", high: "high effort", xhigh: "extra-high effort", max: "max effort" };
// one choice instead of overlapping switches: ask (waits for you, asks before shell commands), full (never asks,
// keeps going until done), night (full auto for up to 24 hours, past "done" and "needs you")
/* [id, name, the one line its row in the menu says, the whole of it (the row's tooltip)] */
export const RUN_MODES: [string, string, string, string][] = [
  ["ask", "Ask me", "Does what you ask, then waits for you.", "Does what you ask, then waits for you. Asks before running programs you haven't allowed."],
  ["full", "Full auto", "Never asks, and keeps going until done.", "Never asks permission: every tool and command just runs, and it keeps going until the job is done. It still won't spend money, publish or delete your work unless you ask."],
  ["night", "Overnight", "Full auto for up to 24 hours without you.", "Full auto for up to 24 hours without you: questions wait for you in Review, it picks its own next job, and the computer stays awake. For a Roblox game, leave Studio open. Pressing stop ends it."],
  ["studio", "Studio", "A Lead and its experts build the project, stage by stage.", "A Lead takes this project through its stages and hands each part to an expert. Finished work and decisions wait for you in Review."],
];
export type Att = { image: boolean; label: string; preview: string; name: string | null; failed: boolean };
const get = (k: string) => { try { return localStorage.getItem(k); } catch (e) { return null; } };
const keep = (k: string, v: string) => { try { localStorage.setItem(k, v); } catch (e) {} };
const json = (k: string) => { try { return JSON.parse(localStorage.getItem(k) || "{}"); } catch (e) { return {}; } };

export const C = {
  runMode: get("runmode") || (get("overnight") === "1" ? "night" : get("fullauto") === "1" || get("autopilot") === "1" ? "full" : "ask"),
  autoPilot: false, overNight: false, fullAuto: false,
  selfTest: get("selftest") === "1", geminiMode: get("gemini") === "1",
  chosenModel: json("models") as Record<string, string>, chosenEffort: json("efforts") as Record<string, string>,
  pending: [] as Att[],
  pop: "",               /* the menu of the message box that is open: "add", "model", "mode" or none */
  startingRun: "",       /* the chat a studio run is being started from, until its first event or an error */
  box: null as HTMLTextAreaElement | null,
};
if (!RUN_MODES.some((m) => m[0] === C.runMode)) C.runMode = "ask";
/* Studio sends its own way (see studioSend); other messages sent while it is chosen go in full auto */
export function applyMode() { C.fullAuto = C.runMode !== "ask"; C.overNight = C.runMode === "night"; C.autoPilot = false; }
applyMode();
export function setRunMode(id: string) { C.runMode = id; applyMode(); keep("runmode", id); }
export function setSelfTest(v: boolean) { C.selfTest = v; keep("selftest", v ? "1" : "0"); poke(); }
export function setGeminiMode(v: boolean) { C.geminiMode = !!v; keep("gemini", C.geminiMode ? "1" : "0"); poke(); }
export const MAX_ATTACH = 10, MAX_BYTES = 30 * 1024 * 1024;
const cur = () => H.agents.get(H.current);
export const note = (text: string) => H.say(text);

/* ---------- the three menus in the message box: one open at a time ---------- */
export function closePops(but?: string) { if (C.pop && C.pop !== but) { C.pop = ""; poke(); } }
export function togglePop(name: string) { C.pop = C.pop === name ? "" : name; poke(); return C.pop === name; }

/* ---------- the AI's model and effort ---------- */
/* Each AI has its own models: Claude's are fixed, Gemini's, ChatGPT's, Cursor's and OpenCode's are whatever their tools list on this computer. */
export function modelsFor(ai: string): [string, string, string][] {
  if (ai === "claude") return MODELS;
  const listed = (((H.status.ai || {}).models || {})[ai]) || [];
  return listed.map(([id, name]: [string, string]) => [id, name.replace(/^Gemini /, ""), id]);
}
/* The model the next message asks for: the one picked for this AI, else the AI's set default (Opus, Gemini 4 Carbon,
   the newest GPT Astra, Cursor's Auto). There is no "whatever the program is set to". */
export function modelOf(p: any): string {
  /* OpenCode's pick is kept by the app (status.ai.default_models.opencode is that pick, or the app's choice while
     there is none: a model of your plan, else a free one), so experts follow it too. With no such model the app
     names none and OpenCode chooses: never the first one listed, which may be billed per use */
  const ai = H.chatAi(p), list = modelsFor(ai), pick = ai === "opencode" ? "" : C.chosenModel[ai];
  if (list.some((m) => m[0] === pick)) return pick;
  const dflt = ai === "claude" ? "opus" : (((H.status.ai || {}).default_models || {})[ai]) || "";
  if (ai === "opencode") return list.some((m) => m[0] === dflt) ? dflt : "";
  return list.some((m) => m[0] === dflt) ? dflt : list.length ? list[0][0] : "";
}
export function effortOf(p: any): string { return C.chosenEffort[H.chatAi(p)] || "high"; }
export function pickModel(ai: string, id: string) { C.chosenModel[ai] = id; keep("models", JSON.stringify(C.chosenModel)); poke(); }
export function pickEffort(ai: string, id: string) { C.chosenEffort[ai] = id; keep("efforts", JSON.stringify(C.chosenEffort)); poke(); }
/* The model picked for OpenCode goes to the app's settings: shown at once, then confirmed by the next status. */
export async function pickOpenCodeModel(id: string) {
  const st = H.status;
  if (st.ai && st.ai.default_models) st.ai.default_models.opencode = id;
  poke();
  let r = null; try { r = await api("/api/opencode", { action: "model", model: id }); } catch (e) {}
  if (!r || !r.ok) note((r && r.error) || "Could not save that pick. Try again.");
  H.refresh();
}
/* A pick this window kept for OpenCode before the app did is handed to the app, and dropped here only when the app
   has taken it, has one already, or no longer lists it. */
let ocPickMoving = false;
export async function moveOpenCodePick() {
  const old = C.chosenModel.opencode, oc = (H.status.ai || {}).opencode, list = modelsFor("opencode");
  if (ocPickMoving || !old || !oc || !list.length) return;
  ocPickMoving = true;
  let settled = !!oc.pick || !list.some((m) => m[0] === old), sent = false;
  if (!settled) { try { const r = await api("/api/opencode", { action: "model", model: old }); settled = !!(r && r.ok); sent = settled; } catch (e) {} }
  if (settled) { delete C.chosenModel.opencode; keep("models", JSON.stringify(C.chosenModel)); }
  ocPickMoving = false;
  if (sent) H.refresh();
}
/* the app is told the chat in front's model and effort when either is picked or its AI is switched, so the app's own
   messages to it (a run's wakes, board news) and its waiting messages use them at once (/api/chat-model) */
export function tellPick(p?: any) { p = p || cur(); if (!p || !p.started) return; api("/api/chat-model", { agent: p.id, model: modelOf(p) || null, effort: effortOf(p) }).catch(() => {}); }
/* no effort to set: the AI's models carry their own, or this model names its level (Gemini's "gemini-4-neon-high") */
export function ownLevel(ai: string, model: string) { return H.caps(ai).effort === false || (ai === "gemini" && /-(low|medium|high|xhigh|max)$/.test(model || "")); }
/* OpenCode's models come from several providers ("OpenCode Go · glm-5.3"): [the provider, the model], or ["", name] */
export function modelParts(ai: string, name: string): [string, string] {
  const at = H.caps(ai).grouped_models ? name.indexOf(" · ") : -1;
  return at < 0 ? ["", name] : [name.slice(0, at), name.slice(at + 3)];
}
/* More accounts for an AI (status.ai.accounts; Settings, Connections adds and removes them): in a chat, a few words
   say which account it is on, only while there is more than one. */
export function accountWords(ai: string, p: any) { return ((((H.status.ai || {}).accounts || {})[ai] || {}).accounts || []).length && p ? " · account " + (p.account || 1) : ""; }
/* The picker's left side: a chat that has started carries on with the other AI, handed the chat so far. */
export async function switchAi(id: string, focusModel: () => void) {
  const p = cur();
  if (!p) { H.setDraftAi(id); poke(); H.paintNeeds(); return; }
  if (H.chatAi(p) === id) return;
  /* a switch that is refused says why where it can be seen: the picker closes and the keyboard is back on its button */
  const refuse = (why: string) => { closePops(); focusModel(); note(why); };
  if (p.busy) { refuse("Wait until it finishes, or stop it, then switch."); return; }
  let r; try { r = await api("/api/ai", { agent: p.id, choice: id }); } catch (e) { r = { ok: false, error: "Could not reach the app." }; }
  if (!r.ok) { refuse(r.error || "Could not switch."); return; }
  p.ai = id; poke(); H.paintNeeds(); tellPick(p);
}

/* ---------- how it works ---------- */
/* the project the next message belongs to: the chat's own, or on a project's empty page (a chat not made yet) that project */
export function gameInFront() {
  const p = cur();
  if (p) return p.project || null;
  return H.curProj && H.draftOn ? H.frontProj() : null;
}
/* Studio needs a project folder, and helper agents on the chat's AI; "" when it can be used */
export function studioWhy() {
  const g = gameInFront();
  if (!g) return "Needs a project folder: start one first.";
  const r = H.rvAll[g.path];
  return r && r.helpers === false ? "Needs helper agents, and they don't work on this chat's AI." : "";
}
/* what Studio's row says for the chat in front: what its next message will do */
export function studioLine(p: any) {
  const g = gameInFront(); if (!g) return "";
  const r = H.rvAll[g.path], run = r && r.run;
  if (H.runActive(run)) return p && run.lead === p.id ? "This chat leads " + g.name + "'s run. Answer it in Review." : "Another chat leads " + g.name + "'s run.";
  return C.runMode === "studio" ? "Your next message starts a studio run for " + g.name + "." : "";
}
/* the mode button and its menu: the four ways it works, one line each */
export function modeRows() {
  const p = cur(), ai = H.chatAi(p);
  // a mode this chat's AI cannot keep to is never left selected (a saved choice, or the chat's AI switched): Ask me
  if (ai && C.runMode !== "ask" && !H.modeKept(ai, C.runMode)) { C.runMode = "ask"; applyMode(); }
  const why = studioWhy(), lead = H.leadsRun(p), noGame = !gameInFront();
  /* with no project folder a message goes as Ask me whatever is chosen, so that is what the button says */
  const shown = lead ? "studio" : C.runMode === "studio" && noGame ? "ask" : C.runMode;
  const rows = RUN_MODES.map(([id, name, line, whole]) => {
    let title = lead && id !== "studio" ? "This chat leads the studio run, so what you send here goes in Studio mode." : whole, off = "";
    /* a mode that cannot be used says why on its own row */
    if (id === "studio" && why) { off = why; title = "Studio: " + why; }
    if (!H.modeKept(ai, id)) { off = "Not offered: " + H.aiName(ai) + " cannot keep to it here."; title = H.aiName(ai) + " cannot keep to this mode by its own rules here, so it is not offered. Messages go as Ask me."; }
    return { id, name, line: off || (id === "studio" && studioLine(p)) || line, title, off: !!off, on: id === shown };
  });
  return { rows, lead, noGame };
}

/* ---------- the message box's words ---------- */
export const text = () => (C.box ? C.box.value : "");
export function fit() { const b = C.box; if (b) { b.style.height = "auto"; b.style.height = Math.min(b.scrollHeight, 168) + "px"; } poke(); }
export function setText(v: string, caretAtEnd?: boolean) { const b = C.box; if (!b) return; b.value = v; fit(); if (caretAtEnd) { b.focus(); b.setSelectionRange(v.length, v.length); } }
export function focusBox() { if (C.box) C.box.focus(); }

/* ---------- sending ---------- */
/* the first message of a game's chat: a new game is talked through first, an existing one is read */
export function kickoffChat(p: any, kind: string) {
  if (!p) return;
  const said = kind === "new" && p.seed ? "This is a new game, starting from this idea out of market research:\n\n" + p.seed + "\n\nInterview me to sharpen it before building anything."
    : kind === "new" ? "This is a new game. Interview me about the idea before building anything."
    : "This is a game I am already working on. Read through it and get up to speed.";
  api("/api/send", { agent: p.id, text: said, kickoff: kind, model: modelOf(p) || null, effort: effortOf(p), selftest: false, gemini: false, attachments: [] });
  p.seed = null;
}
/* a message the app writes for the user after a card's press ("You may now run git..."), in the chat's own mode */
export function sendApp(p: any, said: string) {
  api("/api/send", { agent: p.id, text: said, app: true, studio: !!p.studio, selftest: C.selfTest, gemini: C.geminiMode, autopilot: C.autoPilot, overnight: C.overNight || !!p.studio, full: C.fullAuto || !!p.studio, model: modelOf(p) || null, effort: effortOf(p), attachments: [] });
}
/* a computer that cannot be reached gets nothing: the words stay in the box, and one line says why */
function offline() {
  const m = boxMachine();
  if (online(m)) return false;
  note(nameOf(m) + " can't be reached right now, so nothing was sent. Your message is still here.");
  return true;
}
export function send() {
  const p = cur();
  if (offline()) return;
  if (!p) { sendFromDraft(); return; }
  if (H.isHelper(p)) return;   /* an expert's chat is read only */
  const said = text().trim();
  if (p.busy && !said && !C.pending.some((a) => a.name)) { api("/api/stop", { agent: p.id }); return; }
  const ready = C.pending.filter((a) => a.name);
  if (!said && !ready.length) return;
  if (C.pending.some((a) => !a.name && !a.failed)) return;  /* an upload is still in flight */
  const names = ready.map((a) => a.name);
  if (C.runMode === "studio" || H.leadsRun(p)) { studioSend(p, said, ready, C.runMode === "studio"); return; }
  C.box!.value = ""; C.pending = []; fit();
  plainSend(p, said, names);
}
/* the first message on an empty page makes the chat (tied to the project in front), then goes as usual */
async function sendFromDraft() {
  if (H.curProj === null || !H.draftOn) return;
  if (!text().trim() && !C.pending.some((a) => a.name)) return;
  if (C.pending.some((a) => !a.name && !a.failed)) return;
  const id = await H.ensureChat(H.curProj);
  if (!id) return;
  if (H.current === id && H.agents.get(id)) send();
}
/* a message in the chosen ordinary mode (Ask me, Full auto, Overnight) */
function plainSend(p: any, said: string, names: (string | null)[]) {
  /* a chat started from a researched idea carries the idea's details with its first message */
  const full = p.seed ? said + "\n\n" + p.seed : said; p.seed = null;
  api("/api/send", { agent: p.id, text: full, selftest: C.selfTest, gemini: C.geminiMode, autopilot: C.autoPilot, overnight: C.overNight, full: C.fullAuto, model: modelOf(p) || null, effort: effortOf(p), attachments: names });
}
/* Studio: in the Lead's own chat a message goes to it in Studio mode; in another chat of a project with no run going,
   it starts one with this chat as the Lead and the message as the idea; while another chat leads, nothing is sent.
   `chosen`: Studio is the mode picked under the box. Only then may a message start a run: a chat that only looked like
   the Lead (a run ended a moment ago) sends in the chosen ordinary mode once the fresh run says it leads nothing. */
async function studioSend(p: any, said: string, ready: Att[], chosen: boolean) {
  const names = ready.map((a) => a.name);
  C.box!.value = ""; C.pending = []; fit();
  const back = (why: string, link?: string) => {
    if (!text().trim()) C.box!.value = said;
    if (!C.pending.length) C.pending = ready;
    fit();
    if (link) noteReview(why, link); else note(why);
  };
  const pr = p.project;
  /* no project folder: this message goes as Ask me, and the app says so in the chat */
  if (!pr) {
    const full = p.seed ? said + "\n\n" + p.seed : said; p.seed = null;
    api("/api/send", { agent: p.id, text: full, studio: true, selftest: C.selfTest, gemini: C.geminiMode, autopilot: false, overnight: false, full: false, model: modelOf(p) || null, effort: effortOf(p), attachments: names });
    return;
  }
  let r; try { r = await api("/api/review?path=" + encodeURIComponent(pr.path)); } catch (e) { r = null; }
  if (!r || !r.ok) return back((r && r.error) || "Could not reach the app.");
  H.rvAll[pr.path] = r;
  const full = p.seed ? said + "\n\n" + p.seed : said;
  if (H.runActive(r.run) && r.run.lead === p.id) {
    p.seed = null;
    api("/api/send", { agent: p.id, text: full, studio: true, selftest: C.selfTest, gemini: C.geminiMode, autopilot: false, overnight: true, full: true, model: modelOf(p) || null, effort: effortOf(p), attachments: names });
    return;
  }
  if (!chosen) { poke(); plainSend(p, said, names); return; }
  if (H.runActive(r.run)) return back("This project's studio run is led by another chat.", pr.path);
  if (names.length) return back("Send files once the run has started: the message that starts it is words only.");
  if (!said) return back("Say what to make, then send.");
  C.startingRun = p.id; poke();
  let res; try { res = await api("/api/studio-run", { action: "start", path: pr.path, idea: full, agent: p.id }); } catch (e) { res = { ok: false, error: "Could not reach the app." }; }
  if (!res.ok) { C.startingRun = ""; return back(res.error || "Could not start the studio run."); }
  if (res.agent && res.agent !== p.id) C.startingRun = "";
  p.seed = null;
  if (res.warning) note(res.warning);
  H.loadReview(pr.path);
}
/* a passing word over the message box, with a way to Review */
function noteReview(said: string, path: string) {
  const b = document.createElement("button"); b.className = "linkish"; b.textContent = "Open Review";
  b.onclick = () => { if (b.parentElement) b.parentElement.classList.remove("on"); H.openInReview(path); };
  H.say(said, b);
}

/* ---------- attachments: paste, drop or pick any file ---------- */
const isImage = (f: File) => !!f.type && f.type.startsWith("image/");
/* the texture the chat in front waits for: a picture given now is its answer */
function waitingAsk() { const p = cur(); const ask = p && p.asks ? Object.keys(p.asks) : []; return ask.length ? ask[0] : null; }
export function removeAtt(a: Att) { C.pending = C.pending.filter((b) => b !== a); poke(); }
export function addFile(file: File | null) {
  if (!file) return;
  if (H.view === "chat" && H.isHelper(cur())) { note("An expert's chat is read only. Open the chat that started it to send it a file."); return; }
  if (/^(video|audio)\//.test(file.type || "")) { note("For videos and music, press + and pick Video clips: they are used where they are, not copied."); return; }
  const image = isImage(file);
  const ask = image ? waitingAsk() : null;
  if (ask) { const r = new FileReader(); r.onload = () => api("/api/texture", { id: ask, action: "image", data: r.result }); r.readAsDataURL(file); return; }
  if (C.pending.length >= MAX_ATTACH) { note("Up to " + MAX_ATTACH + " files per message."); return; }
  if (file.size > MAX_BYTES) { note((file.name || "That file") + " is over 30 MB."); return; }
  const a: Att = { image, label: file.name || "pasted image", preview: "", name: null, failed: false };
  C.pending.push(a); poke();
  const reader = new FileReader();
  reader.onload = async () => {
    if (image) { a.preview = reader.result as string; poke(); }
    try {
      /* the file goes to the computer the message will go to (ui/src/remote) */
      const r = await api(at(boxMachine(), "/api/attach"), { data: reader.result, filename: file.name || "" });
      if (r.ok) { a.name = r.name; a.image = r.kind === "image"; } else a.failed = true;
    } catch (e) { a.failed = true; }
    if (a.failed) { C.pending = C.pending.filter((b) => b !== a); note("Could not attach " + a.label + "."); }
    poke();
  };
  reader.onerror = () => { C.pending = C.pending.filter((b) => b !== a); poke(); note("Could not read " + a.label + "."); };
  reader.readAsDataURL(file);
}
/* Videos and music go in the message as [clip: <path>] lines: the agent uses them where they are, nothing is copied. */
export function addClips(paths: string[]) {
  const b = C.box; if (!b || !paths.length) return;
  const lines = paths.map((p) => "[clip: " + p + "]").join("\n");
  b.value = b.value.replace(/\s+$/, "") + (b.value.trim() ? "\n" : "") + lines + "\n";
  fit(); b.focus();
}
/* A file the app attached itself: it was dropped together with videos, so the page never saw the drop. */
export async function attachedHere(e: any) {
  if (!e.ok) { note(e.error || "Could not attach " + (e.label || "that file") + "."); return; }
  const url = "/att/" + encodeURIComponent(e.name) + "?t=" + TOKEN, ask = e.kind === "image" ? waitingAsk() : null;
  if (ask) {
    const blob = await (await fetch(url)).blob(), r = new FileReader();
    r.onload = () => api("/api/texture", { id: ask, action: "image", data: r.result }); r.readAsDataURL(blob); return;
  }
  if (C.pending.length >= MAX_ATTACH) { note("Up to " + MAX_ATTACH + " files per message."); return; }
  C.pending.push({ image: e.kind === "image", label: e.label, preview: e.kind === "image" ? url : "", name: e.name, failed: false });
  poke();
}
export async function copyText(said: string) {
  try { await navigator.clipboard.writeText(said); return true; } catch (e) {}
  try { return (await api("/api/copy", { text: said })).ok; } catch (e) { return false; }
}
