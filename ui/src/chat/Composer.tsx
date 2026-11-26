/* The message box: the words on top; under them, what to add, which AI and model, how it works, and Send. An
   expert's chat is read only: one line takes the box's place there. */
import { AiLogo } from "../ui/AiLogo";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { noticeOf, pressOpenCode, pressSignIn } from "../ai";
import { H } from "./host";
import { poke, useTick } from "./chatStore";
import { keep } from "./keep";
import { C, EFFORTS, EFFORT_NAMES, accountWords, addClips, addFile, closePops, effortOf, fit, modeRows, modelOf, modelParts, modelsFor, ownLevel, pickEffort, pickModel, pickOpenCodeModel,
  removeAtt, send, setGeminiMode, setRunMode, setSelfTest, setText, switchAi, tellPick, text, togglePop } from "./compose";
import { FileTile } from "./Rows";
import { ToLatest, relayout } from "./Timeline";

const DOWN = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3.5l2.5 2.5L7 3.5" /></svg>;
/* the chosen row of a menu */
const TICK = <span class="tick"><svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 5.5l2.5 2.5 4.5-5.5" /></svg></span>;
const byId = (id: string) => document.getElementById(id);
const EFFORT_TIPS: Record<string, string> = { low: "Quick answers, least thinking", medium: "Some thinking", high: "Thinks things through (the usual choice)", xhigh: "Extra-high: more thinking for hard problems", max: "Thinks as long as it needs; slowest and uses the most" };

/* typing "/" offers the commands */
const COMMANDS = [
  ["/texture", "A surface material that tiles: stone, wood, metal"],
  ["/particle", "A white shape on clear, for particle effects"],
  ["/decal", "A coloured picture on clear: a crack, a stain, a sign"],
];
const slash = { on: false, sel: 0 };
const slashHits = () => { const v = text(); return /^\/[a-z]*$/i.test(v) ? COMMANDS.filter((c) => c[0].startsWith(v.toLowerCase())) : []; };

/* An expert's chat is its own log, read only: where the message box would be, one line says so, with Stop while it
   works, the way to the chat that started it, and Close. */
function ExpertBar({ h }: { h: any }) {
  const on = H.isHelper(h);
  if (!on) return <div class="xp-bar" id="xp-bar" hidden />;
  const of = H.agents.get(h.parent), [who] = H.splitExpert(h.name, h.expert), m = H.memberOf(h.id);
  return (
    <div class="xp-bar" id="xp-bar">
      {/* a member of a team's run can be written to on its Lead's team board */}
      <span id="xp-bar-text">{who + "'s own chat, read only. " + (m && of ? "To tell it something, write @" + H.bdName(m.name) + " on the team board beside “" + of.name + "”." : of ? "“" + of.name + "” started it and talks to it." : "The chat that started it is closed.")}</span>
      <button class="pill sm" id="xp-stop" hidden={!h.busy} onClick={() => api("/api/stop", { agent: h.id })}>Stop</button>
      <button class="pill sm" id="xp-lead" hidden={!(of || H.chatKnown(h.parent))} onClick={() => H.openChatById(h.parent)}>{of && H.leadsRun(of) ? "Open the Lead's chat" : "Open the chat that started it"}</button>
      <button class="linkish" id="xp-close" hidden={!!h.busy} title="Close this expert. Its chat stays saved, and it stays in the list to open again." onClick={() => api("/api/remove", { agent: h.id })}>Close</button>
    </div>
  );
}

/* One line about the chosen AI at the top of the picker's models: an AI that is not signed in says so, with the
   sign-in one press away (the words are ui/src/ai.ts). Empty, it takes no room. */
let presses = 0;
function Notice({ ai }: { ai: string }) {
  const s = noticeOf(ai);
  return <div class="notice" id="model-notice">{s && <span class="nt" key={presses + s.text}><span>{s.text}</span>{s.buttons.map(([label, action]) => <NoticeButton ai={ai} label={label} action={action} />)}</span>}</div>;
}
function NoticeButton({ ai, label, action }: { ai: string; label: string; action: string }) {
  const [off, setOff] = useState(false);
  /* a pressed button stays greyed until the notice is painted again with what the press did */
  const press = async () => { setOff(true); presses++; if (ai === "opencode") await pressOpenCode(action); else await pressSignIn(ai, action); };
  return <button class="pill sm" disabled={off} onClick={press}>{label}</button>;
}

/* The picker: the AIs down the left, the chosen AI's models on the right, effort under them. It is painted only while
   it is open (and stays as it was while it fades out). */
let pickerOpened = false;
const Picker = keep(PickerBody, () => C.pop !== "model");
function PickerBody({ p }: { p: any }) {
  const ai = H.chatAi(p), list = modelsFor(ai), chosen = modelOf(p), effort = effortOf(p), have = (H.status.ai || {}).installed || {};
  const [find, setFind] = useState({ ai: "", q: "" });
  const q = find.ai === ai ? find.q : "";
  /* a long list (Cursor offers hundreds) scrolls in a box of its own, with a filter, so effort stays in view */
  const words = list.length > 12 ? q.toLowerCase().split(/\s+/).filter(Boolean) : [];
  const shown = list.filter(([id, name]) => words.every((w) => (id + " " + name).toLowerCase().includes(w)));
  const own = ownLevel(ai, chosen);
  let group = "";
  return <>
    <div class="pk-ais" id="ai-pick" role="radiogroup" aria-label="Which AI this chat uses">
      {H.AIS.map(([id, name]: [string, string]) => (
        <button class="pk-ai" role="radio" aria-checked={id === ai ? "true" : "false"} onClick={() => switchAi(id, () => byId("model")!.focus())}
          title={id === ai ? "This chat uses " + name + "." : (p && p.started ? "Carry on this chat with " + name + ". It is handed the chat so far." : "Use " + name + " for this chat.") + (have[id] === false ? " " + name + " is not set up on this computer yet: Settings, Connections says how." : "")}>
          <AiLogo ai={id} size={14} /><span class="pk-ai-t"><b>{name}</b>{have[id] === false && <small>Not installed</small>}</span>
        </button>
      ))}
    </div>
    <div class="pk-models">
      <Notice ai={ai} />
      <div id="model-menu" role="listbox" aria-label="Model">
        {!list.length && <p class="sub" style="margin: 8px;">Its model list has not arrived yet.</p>}
        {list.length > 12 && <input class="mfind" type="search" placeholder="Find a model" aria-label="Find a model" value={q} onInput={(e) => setFind({ ai, q: e.currentTarget.value })} />}
        <div class="mlist">
          {list.length > 0 && !shown.length && <p class="sub" style="margin: 8px;">No model matches.</p>}
          {/* a heading per provider, the model under it */}
          {shown.map(([id, name, note]) => {
            const [g, rest] = modelParts(ai, name), head = g && g !== group ? ((group = g), <p class="mgroup">{g}</p>) : null;
            return <>{head}<button class="opt" role="option" aria-selected={id === chosen ? "true" : "false"}
              onClick={() => { if (ai === "opencode") { pickOpenCodeModel(id).then(() => tellPick(p)); return; } pickModel(ai, id); tellPick(p); }}>
              {TICK}<span><b>{rest}</b>{note && note !== id && <small>{note}</small>}</span></button></>;
          })}
        </div>
      </div>
      {/* a Cursor model carries its effort in its id (Codex 5.3 High...), and OpenCode's models their own: nothing to set apart */}
      <div class="pk-effort" id="effort-wrap" hidden={own}><h5>Effort</h5>
        <div class="seg" id="effort">{!own && EFFORTS.map(([id, name]) => <button class={id === effort ? "on" : ""} aria-pressed={id === effort ? "true" : "false"} title={EFFORT_TIPS[id]} onClick={() => { pickEffort(ai, id); tellPick(); }}>{name}</button>)}</div>
      </div>
    </div>
  </>;
}

export function Dock() {
  useTick();
  const p = H.agents.get(H.current), busy = !!(p && p.busy), helper = H.isHelper(p), st = H.status;
  const dock = useRef<HTMLDivElement>(null), box = useRef<HTMLTextAreaElement>(null), file = useRef<HTMLInputElement>(null);
  /* the message box floats over the chat: the chat keeps as much room at its end as the box is tall */
  useLayoutEffect(() => {
    C.box = box.current;
    const d = dock.current!, ro = new ResizeObserver(() => { (d.parentNode as HTMLElement).style.setProperty("--dock-h", d.offsetHeight + "px"); relayout(); });
    ro.observe(d); fit();
    /* a press anywhere else closes the menus (the path of the press, not where its target is now: a press inside a
       menu may repaint the row it landed on) */
    const out = (ev: MouseEvent) => { if (!ev.composedPath().some((n: any) => n.classList && n.classList.contains("pick"))) closePops(); };
    document.addEventListener("click", out);
    return () => { ro.disconnect(); document.removeEventListener("click", out); };
  }, []);
  /* the picker opens on the chosen model */
  useEffect(() => { if (C.pop === "model") { const on = byId("model-menu")!.querySelector('.mlist [aria-selected="true"]'); if (on) on.scrollIntoView({ block: "nearest" }); } }, [C.pop]);
  if (C.pop === "model") pickerOpened = true;

  const ai = H.chatAi(p), list = modelsFor(ai), chosen = modelOf(p), effort = effortOf(p), cur = list.find((m) => m[0] === chosen), own = ownLevel(ai, chosen);
  const something = !!text().trim() || C.pending.some((a) => a.name);
  const stop = busy && !something;   /* while it works: with text the button sends (it waits its turn), without it stops */
  const { rows, lead, noGame } = modeRows();
  const extras = [C.selfTest ? "Self-test" : "", C.geminiMode ? "Gemini textures" : ""].filter(Boolean), mode = rows.find((r) => r.on) || rows[0];
  /* one plain line under the message box when Cursor or OpenCode runs in Full auto or Studio and can do something there
     that Claude cannot (status.ai.gaps, from the app; none today for either, so this stays hidden) */
  const gaps = (((st.ai || {}).gaps || {})[ai]) || {}, gapMode = C.runMode === "studio" || lead ? "studio" : C.fullAuto ? "full" : "", gap = (gapMode && gaps[gapMode]) || "";
  /* the quiet line inside the box: what it is doing now, else what is selected in Studio */
  let now = null;
  if (busy) now = <span class="now">{p.now || "Starting"}</span>;
  else if (C.startingRun && p && C.startingRun === p.id) now = <span class="now">Starting the studio run…</span>;
  else {
    const pl = st.plugin;
    if (pl && pl.connected && pl.selection && pl.selection.count && H.isGameProj(p ? p.project : H.curProj)) {   /* Studio's selection: Roblox projects only */
      const it = pl.selection.items || [], first = it[0] ? it[0].path.split(".").pop() + " (" + it[0].class + ")" : "";
      now = <span class="sel" title="The agent is told what you have selected in Studio.">{"Selected in Studio: " + first + (pl.selection.count > 1 ? " +" + (pl.selection.count - 1) : "")}</span>;
    }
  }
  const hits = slash.on ? slashHits() : [];
  const pickSlash = (cmd: string) => { setText(cmd + " "); box.current!.focus(); slash.on = false; };
  const onKey = (e: KeyboardEvent) => {
    /* while the commands are offered these keys never reach "Enter sends the message" */
    if (slash.on && hits.length && !e.isComposing) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); slash.sel = (slash.sel + (e.key === "ArrowDown" ? 1 : -1) + hits.length) % hits.length; poke(); return; }
      if (e.key === "Enter" || e.key === "Tab") { e.preventDefault(); pickSlash(hits[slash.sel][0]); return; }
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); slash.on = false; poke(); return; }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); }
  };
  const pop = (name: string) => (ev: Event) => { ev.stopPropagation(); togglePop(name); };
  /* a choice closes the menu and hands the keyboard back to its button */
  const chose = (btn: string) => { closePops(); byId(btn)!.focus(); };
  return (
    <div class="dock" ref={dock}>
      <ToLatest />
      <ExpertBar h={p} />
      <div class="composer" hidden={helper}>
        <div class={"atts" + (C.pending.length ? " has" : "")} id="atts">
          {C.pending.map((a) => (
            <div class={"att" + (a.image ? "" : " doc") + (a.name ? "" : " wait")} title={a.label}>
              {a.image ? <img src={a.preview} alt="Attached image" /> : <FileTile label={a.label} />}
              <button aria-label={"Remove " + a.label} onClick={() => removeAtt(a)}><svg width="8" height="8" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M1.5 1.5l6 6M7.5 1.5l-6 6" /></svg></button>
            </div>
          ))}
        </div>
        <input type="file" id="file" ref={file} multiple hidden onChange={(e) => { [...(e.currentTarget.files || [])].forEach(addFile); e.currentTarget.value = ""; }} />
        <textarea id="box" ref={box} rows={1} placeholder={busy ? "Add to it while it works. Sent the moment it finishes." : "What should we build or test?"} autofocus
          onInput={() => { slash.on = true; slash.sel = 0; fit(); }} onBlur={() => { if (slash.on) { slash.on = false; poke(); } }} onKeyDown={onKey} />
        <div class="c-bar">
          <span class="pick">
            <button class="c-add" id="add" aria-haspopup="menu" aria-expanded={C.pop === "add" ? "true" : "false"} aria-label="Add files" title="Add images, documents or video clips. You can also paste them or drop them on the window." onClick={pop("add")}><svg width="12" height="12" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M5.5 1.5v8M1.5 5.5h8" /></svg></button>
            <div class={"menu" + (C.pop === "add" ? " on" : "")} id="add-menu" role="menu">
              <button class="opt" id="attach" role="menuitem" onClick={() => { chose("add"); file.current!.click(); }}><span class="tick" /><span><b>Images or documents</b></span></button>
              <button class="opt" id="add-clips" role="menuitem" onClick={async () => { chose("add"); const r = await api("/api/pick-files", { kind: "clips" }); addClips(r.paths || []); }}><span class="tick" /><span><b>Video clips</b><small>Used where they are, not copied.</small></span></button>
            </div>
          </span>
          <span class="pick">
            <button class="chip" id="model" aria-haspopup="dialog" aria-expanded={C.pop === "model" ? "true" : "false"} onClick={pop("model")}
              title={[H.aiName(ai), cur ? cur[1] : "", own ? "" : EFFORT_NAMES[effort]].filter(Boolean).join(" · ") + ". Press to change the AI, its model or the effort, from your next message."}>
              <AiLogo ai={ai} size={13} /><span id="model-name">{H.aiName(ai) + (cur ? " · " + modelParts(ai, cur[1])[1] : "") + accountWords(ai, p)}</span>{DOWN}</button>
            {/* the picker behind the button is only painted once it has been opened */}
            <div class={"menu picker" + (C.pop === "model" ? " on" : "")} id="settings-panel" role="dialog" aria-label="AI and model">{pickerOpened && <Picker p={p} />}</div>
          </span>
          <span class="pick">
            <button class="chip" id="mode" aria-haspopup="menu" aria-expanded={C.pop === "mode" ? "true" : "false"} onClick={pop("mode")}
              title={mode.name + ": " + mode.line + (C.runMode === "studio" && noGame && !lead ? " Studio is chosen, but it needs a project folder, so a message here goes as Ask me." : "") + (extras.length ? " Also on: " + extras.join(", ") + "." : "") + " Press to change how it works."}>
              <span id="mode-name">{mode.name}</span>{extras.length > 0 && <span class="extra">{"+" + extras.length}</span>}{DOWN}</button>
            <div class={"menu" + (C.pop === "mode" ? " on" : "")} id="mode-menu" role="menu" aria-label="How it works">
              <div id="run-mode" role="radiogroup" aria-label="How it works">
                {rows.map((r) => <button class="opt" role="radio" aria-checked={r.on ? "true" : "false"} title={r.title} disabled={r.off} onClick={() => { setRunMode(r.id); chose("mode"); poke(); }}>{TICK}<span><b>{r.name}</b><small>{r.line}</small></span></button>)}
              </div>
              <h5 class="rule">More</h5>
              <button class="switch" id="selftest" role="switch" aria-checked={C.selfTest ? "true" : "false"} title="It checks its work with screenshots and playtests and fixes what it finds, up to five rounds." onClick={() => setSelfTest(!C.selfTest)}><span class="track"><span class="knob" /></span><span><span class="swl">Self-test</span><small>Checks its own work and fixes it.</small></span></button>
              <button class="switch" id="gemini" role="switch" aria-checked={C.geminiMode ? "true" : "false"} title="For each texture it shows you a prompt to run in Gemini and waits for the image, instead of drawing its own." onClick={() => setGeminiMode(!C.geminiMode)}><span class="track"><span class="knob" /></span><span><span class="swl">Gemini textures</span><small>Asks you for each texture through Gemini.</small></span></button>
            </div>
          </span>
          <span id="now" title={busy ? p.now || "" : ""}>{now}</span>
          <button class={"send" + (stop ? " stop" : "")} id="send" aria-label={stop ? "Stop" : busy ? "Send when it finishes" : "Send"} title={stop ? "Stop" : busy ? "Send. It waits until the agent finishes, then goes straight to it." : ""} disabled={!busy && !something} onClick={() => send()}>
            <svg class="i-send" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M8 13V3M3.5 7.5 8 3l4.5 4.5" /></svg>
            <svg class="i-stop" width="12" height="12" viewBox="0 0 12 12"><rect width="12" height="12" rx="2.5" fill="currentColor" /></svg>
          </button>
        </div>
        <div class={"slash" + (hits.length ? " on" : "")}>
          {hits.map(([cmd, what], i) => <button class={i === slash.sel ? "sel" : ""} onMouseDown={(ev) => { ev.preventDefault(); pickSlash(cmd); }} onMouseMove={() => { if (slash.sel !== i) { slash.sel = i; poke(); } }}><code>{cmd}</code><span>{what + ". Then say what it shows."}</span></button>)}
          {hits.length > 0 && <div class="how">Up and Down to choose · Enter to pick · Esc to close</div>}
        </div>
      </div>
      <p class="aigap" id="ai-gap" hidden={!gap}>{gap}</p>
    </div>
  );
}
