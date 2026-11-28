/* Settings, Knowledge, Knowledge index: Off, Light or Heavy. The mode is the app's setting and the control shows only
   what the app says it is (rag.mode), never a guess. Light has one Set up; Heavy five numbered steps, each with its dot
   (white done, amber needs you, red failed, hollow not yet) and a button that can always run it again. The status is
   asked every second while something runs, otherwise every 10 s, and only while the page is in view. */
import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { cell } from "../store";
import { quiet } from "../util";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Field } from "../ui/Field";
import { DotRow } from "../ui/Row";
import { shown, whenOpened, whenShown } from "./state";

let rag: any = null, ragFail = "", ragNote = "", ragSeq = 0, ragTimer = 0, ragFastUntil = 0, ragChoosing = false;
const changed = cell(0), paint = () => changed.poke(), testReset = cell(0);
let painted = false;   /* the status has been asked for at least once */
const HEAVY_LINE = "Fastest and smartest. Needs Ollama and Qdrant, which Fridae can set up for you.";
const WORK: Record<string, string> = { load: "Loading the models", scan: "Looking through your files", index: "Building the index" };
const DOING: Record<string, string> = { download: "Downloading", unpack: "Unpacking", start: "Starting Ollama", "ollama-pull": "Downloading the model", models: "Downloading the models" };
const STEPS: { id: string; name: string; go: string; doing: string; what: string; done?: string; after?: number[]; wait?: string }[] = [
  { id: "ollama-install", name: "Install Ollama", go: "Install", doing: "Installing...", what: "Ollama runs the AI model that reads your docs. Fridae downloads the free app from ollama.com into your Applications folder.", done: "Ollama is running." },
  { id: "ollama-pull", name: "Download the embedding model", go: "Download", doing: "Downloading...", what: "The model Ollama uses to find text by meaning. A one-time download.", done: "Downloaded.", after: [0], wait: "Needs Ollama running first (step 1)." },
  { id: "qdrant-install", name: "Install Qdrant", go: "Install", doing: "Installing...", what: "Qdrant keeps the index on your Mac. It goes in Fridae's own folder; nothing is installed system-wide.", done: "Installed." },
  { id: "models", name: "Start the reranker", go: "Start", doing: "Starting...", what: "Downloads the model that puts the best matches first (about 1.1 GB). A one-time download.", done: "Ready." },
  { id: "build", name: "Build the index", go: "Build", doing: "Building...", what: "Reads your notes, guides and project docs and builds the index. The first build can take 10 minutes or more on a big vault; after that only changed notes are read again.", after: [0, 1, 2, 3], wait: "Needs steps 1 to 4 first." },
];
const pct = (f: any) => Math.max(0, Math.min(100, Math.round((Number(f) || 0) * 100)));
const pieces = (n: any) => { n = Number(n) || 0; return n.toLocaleString() + (n === 1 ? " piece" : " pieces"); };
const sentence = (s: any) => { s = String(s || "").trim(); return s && !/[.!?]$/.test(s) ? s + "." : s; };
/* the service, only when it serves the chosen mode (just after a switch the old one may still answer) */
function service() { const s = rag && rag.service; return s && typeof s === "object" && (!s.mode || s.mode === rag.mode) ? s : null; }
/* what a running setup step is doing now, from its progress line ("ollama-install: download") */
function doing(su: any) {
  const d = String(su.detail || ""), word = d.slice(d.indexOf(":") + 1).trim();
  return (DOING[word] || DOING[su.step] || "Working") + " (" + pct(su.fraction) + "%).";
}
function building(s: any) { const pr = s.progress || {}; return (WORK[pr.step] || "Building the index") + " (" + pct(pr.fraction) + "%)."; }
function busy() {
  if (!rag) return false;
  const s = service(), p = rag.process || {}, su = rag.setup || {};
  return !!(su.running || (s && s.building) || p.starting || p.restarting || Date.now() < ragFastUntil);
}
export async function loadRag() {
  clearTimeout(ragTimer);
  const my = ++ragSeq;
  if (!shown("knowledge")) return;
  const r = await quiet("/api/rag/status");
  if (my !== ragSeq) return;
  if (r && typeof r === "object" && typeof r.mode === "string") { rag = r; ragFail = ""; } else ragFail = "Could not check the knowledge index.";
  painted = true; paint();
  ragTimer = setTimeout(loadRag, busy() ? 1000 : 10000) as unknown as number;
}
whenShown(["knowledge"], loadRag);
whenOpened(loadRag);
/* a press: the answer's error (if any) goes in the line under the section, and the status is followed closely for a bit */
async function ragPost(path: string, body: unknown) {
  const r = await quiet(path, body);
  ragNote = !r ? "Could not reach the app. Try again." : r.ok === false ? sentence(r.error || "That did not work. Try again.") : "";
  ragFastUntil = Date.now() + 6000;
  return r;
}
async function setup(step: string) { ragNote = ""; const r = await ragPost("/api/rag/setup", { step, mode: rag && rag.mode }); if (r && r.setup) rag.setup = r.setup; paint(); loadRag(); }
async function rebuild() { ragNote = ""; await ragPost("/api/rag/rebuild", {}); loadRag(); }
/* the problem the service or its process stopped with (shown in the line under the section) */
function stopped() { const s = service(), p = rag.process || {}; return !!((s && s.error && !s.building) || (p.error && !p.running) || (p.gave_up && !p.running)); }
/* the app stopped restarting the index by itself (5 failed tries in a row): only a press starts it again */
function gaveUp() { const p = (rag && rag.process) || {}; return !!(p.gave_up && !p.running); }

/* Light's line, bar and button. While another mode is chosen they are hidden, and stay as they last were. */
let lightWas = { text: "", bar: -1, go: "", off: false, drawn: false };
function Light({ still }: { still?: boolean }) {
  let act: (() => void) | null = null;
  if (!still) {
    const su = rag.setup || {}, ck = rag.check || {}, s = service(), p = rag.process || {}, mine = su.step === "models";
    let text: string, bar = -1, go = "";
    if (su.running && mine) { text = doing(su) + " This happens once."; bar = pct(su.fraction); go = "Setting up..."; }
    else if (mine && su.error && !ck.models) { text = "The setup did not finish: " + sentence(su.error); go = "Try again"; act = () => setup("models"); }
    else if (ck.error) text = "Could not check what is set up.";
    else if (!ck.models) { text = "Not set up yet. Set up downloads one small model (about 65 MB), once."; go = "Set up"; act = () => setup("models"); }
    else if (s && s.ready) { text = "Ready, " + pieces(s.chunks) + " indexed."; go = "Build again"; act = rebuild; }
    else if (s && s.building) { text = building(s) + " The first build can take 10 minutes or more on a big vault; after that only changed notes are read again."; bar = pct((s.progress || {}).fraction); }
    else if (stopped()) { text = gaveUp() ? "Fridae stopped trying to start the index." : "The index stopped with a problem."; go = "Try again"; act = rebuild; }
    else text = p.restarting ? "Starting again..." : "Starting...";
    lightWas = { text, bar, go, off: !act || !!su.running, drawn: true };
  }
  const v = lightWas;
  return <><p class="sub" id="rag-l-state">{v.text}</p><div class="eng-bar" id="rag-l-bar" hidden={v.bar < 0}><i style={v.bar >= 0 ? "width: " + v.bar + "%" : undefined}></i></div>
    <div class="act"><button class="pill" id="rag-l-go" hidden={!v.go} disabled={v.off} onClick={act || undefined}>{v.drawn ? v.go : "Set up"}</button></div></>;
}
function Steps() {
  const su = rag.setup || {}, ck = rag.check || {}, s = service(), p = rag.process || {};
  const done = [ck.ollama === "running", !!ck.ollama_model, !!ck.qdrant_binary, !!ck.models, !!(s && s.ready)];
  return <>{STEPS.map((st, i) => {
    const blocked = (st.after || []).some((j) => !done[j]);
    let dot = "", line = st.what, bar = -1, go = st.go, act = () => { setup(st.id); }, live = false;
    if (st.id === "build") {
      act = rebuild;
      if (s && s.building) { dot = "live"; line = building(s); bar = pct((s.progress || {}).fraction); live = true; }
      else if (done[i]) { dot = "ok"; line = "Ready, " + pieces(s.chunks) + " indexed."; }
      else if (blocked) line = st.wait!;
      else if (stopped()) { dot = "bad"; line = (gaveUp() ? "Fridae stopped trying to start the index." : "Could not build the index.") + " What went wrong is just below."; go = "Try again"; }
      else if (p.starting || p.restarting || (p.running && !s)) { dot = "live"; line = p.restarting ? "Starting again..." : "Starting..."; live = true; }
    } else if (su.running && su.step === st.id) { dot = "live"; line = doing(su); bar = pct(su.fraction); live = true; }
    else if (done[i]) { dot = "ok"; line = st.done!; }
    else if (su.step === st.id && su.error) { dot = "bad"; line = sentence(su.error); go = "Try again"; }
    else if (i === 0 && ck.ollama === "stopped") { dot = "miss"; line = "Open the Ollama app to start it."; go = "Check again"; act = () => { ragFastUntil = Date.now() + 6000; loadRag(); }; }
    else if (blocked) line = st.wait!;
    const label = live ? st.doing : go;
    return (
      <DotRow tag="li" dot={dot} name={(i + 1) + ". " + st.name}><small>{line}</small><div class="eng-bar" hidden={bar < 0}><i style={bar >= 0 ? "width: " + bar + "%" : undefined}></i></div>
        <div class="act"><button class="pill sm" disabled={live || blocked || (!!su.running && go !== "Check again")} aria-label={label.replace(/\.+$/, "") + ": " + st.name} onClick={act}>{label}</button></div></DotRow>
    );
  })}</>;
}
function Test() {
  const [q, setQ] = useState(""), [note, setNote] = useState(""), [hits, setHits] = useState<any[]>([]), [looking, setLooking] = useState(false);
  const input = useRef<HTMLInputElement>(null), reset = testReset.use();
  useEffect(() => { setNote(""); setHits([]); }, [reset]);   /* what the last test found goes with the mode it was found in */
  const test = async () => {
    const text = q.trim();
    if (!text) { setNote("Type something to look for."); input.current!.focus(); return; }
    setLooking(true); setNote("Looking..."); setHits([]);
    let r = null; try { r = await api("/api/rag/test", { q: text }); } catch (e) {}
    setLooking(false);
    if (!r || r.ok === false) { setNote(!r ? "Could not reach the app. Try again." : sentence(r.error || "The test did not work.")); return; }
    const found = Array.isArray(r.hits) && r.hits.length ? r.hits : (r.titles || []).map((title: string) => ({ title }));
    setNote(found.length ? "The best matches:" : "Nothing found. Try other words."); setHits(found);
  };
  const mode = rag ? rag.mode : "";
  return (
    <SettingsRow cls="rag-test" id="rag-test" hidden={(mode !== "light" && mode !== "heavy") || !(service() || {}).ready} title="Test it" desc="Type something and see what the index finds."
      ctl={<Field id="rag-q" inputRef={input} value={q} onInput={setQ} onEnter={test} placeholder="Type something to look up" label="Something to look up in the knowledge index"><button class="pill" id="rag-q-go" disabled={looking} onClick={test}>Test it</button></Field>}
      after={<><p class="sub" id="rag-q-note" role="status">{note}</p><ol class="rag-hits" id="rag-hits" aria-label="Top matches">{hits.map((h) => <li>{h.title || "(untitled)"}{h.heading && <span>{" › " + String(h.heading)}</span>}</li>)}</ol></>} />
  );
}
const MODES: [string, string][] = [["off", "Off"], ["light", "Light"], ["heavy", "Heavy"]];
export function Rag() {
  changed.use();
  /* `win`: Heavy cannot run on this system (Windows and Linux for now); an older app only sent `windows` */
  const mode: string = rag ? rag.mode : "", win = !!(rag && (rag.heavy_supported === undefined ? rag.windows : !rag.heavy_supported));
  const choose = async (m: string) => {
    if (ragChoosing || (rag && rag.mode === m)) return;
    ragChoosing = true; paint();
    await ragPost("/api/rag/mode", { mode: m });
    ragChoosing = false;
    testReset.set(testReset.get() + 1);
    await loadRag();
  };
  /* a radio group: the arrows move to the next choice and choose it; Home and End go to the first and last */
  const key = (e: KeyboardEvent) => {
    if (e.altKey || e.ctrlKey || e.metaKey) return;
    const all = [...(e.currentTarget as HTMLElement).querySelectorAll<HTMLButtonElement>("[role=radio]")].filter((b) => !b.disabled), at = all.indexOf(document.activeElement as HTMLButtonElement);
    if (at < 0) return;
    const to = e.key === "ArrowRight" || e.key === "ArrowDown" ? (at + 1) % all.length : e.key === "ArrowLeft" || e.key === "ArrowUp" ? (at - 1 + all.length) % all.length : e.key === "Home" ? 0 : e.key === "End" ? all.length - 1 : -1;
    if (to < 0) return;
    e.preventDefault(); all[to].focus(); choose(all[to].dataset.m!);
  };
  /* one plain line under the section: a press that failed, the service's or its process's error */
  const p = (rag && rag.process) || {}, s = service(), ck = (rag && rag.check) || {}, errs = [ragFail, ragNote];
  if (mode === "light" || mode === "heavy") errs.push(ck.error ? "Could not check what is set up: " + ck.error : "", p.error || (gaveUp() ? "Fridae stopped trying to start the index after 5 tries in a row. Press Try again." : ""), s && s.error);
  const lines: Record<string, string> = { off: "Simple word search (what Fridae does today).", light: "Works out of the box. Downloads one small model (about 65 MB) once.", heavy: win ? "Mac only for now." : HEAVY_LINE };
  return (
    <Group id="set-rag" find="search index ollama qdrant lookup" title="Knowledge index" titleId="rag-title">
      <SettingsRow id="set-rag-mode" title="How the AIs look things up" desc="Lets the AIs look up just the right bits of your notes, guides and project docs instead of reading whole files. Runs on this computer; nothing is sent anywhere."
        ctl={<div class="seg rag-seg" id="rag-seg" role="radiogroup" aria-labelledby="rag-title" aria-busy={painted || ragChoosing ? (ragChoosing ? "true" : "false") : undefined} onKeyDown={key}>
          {MODES.map(([m, name]) => {
            const on = m === mode;
            return <button role="radio" class={on ? "on" : ""} aria-checked={on ? "true" : "false"} tabindex={on || (!/^(off|light|heavy)$/.test(mode) && m === "off") ? 0 : -1} data-m={m} aria-describedby={"rag-d-" + m}
              disabled={m === "heavy" && win && !on} onClick={() => choose(m)}>{name}{m === "heavy" && <span class="rag-x"> (Ollama + Qdrant)</span>}</button>;
          })}
        </div>}
        after={<>
          <ul class="rag-modes" id="rag-modes">{MODES.map(([m, name]) => <li data-m={m} class={m === mode ? "on" : ""}><b>{name + ":"}</b> <span id={"rag-d-" + m}>{lines[m]}</span></li>)}</ul>
          <div class="rag-light" id="rag-light" hidden={mode !== "light"}><Light still={mode !== "light"} /></div>
          <ol class="rag-steps" id="rag-steps" hidden={mode !== "heavy"}>{mode === "heavy" && <Steps />}</ol>
          <p class="sub rag-err" id="rag-err" role="status">{[...new Set(errs.map(sentence).filter(Boolean))].join(" ")}</p>
        </>} />
      <Test />
    </Group>
  );
}
