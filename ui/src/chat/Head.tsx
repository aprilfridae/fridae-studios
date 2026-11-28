/* The bar over the tabs (the project's name and its tabs), and the bar above the conversation: the chat's name (press
   it for the project's chats), its state in a few words, and the run's controls while the project has a run going. */
import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { ago } from "../util";
import { H } from "./host";
import { poke, useTick } from "./chatStore";
import { closePops } from "./compose";
import { isHome } from "../remote/route";
import { TabShape } from "./tab";

const DOWN = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3.5l2.5 2.5L7 3.5" /></svg>;
const CLOSE = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"><path d="M1.5 1.5l6 6M7.5 1.5l-6 6" /></svg>;

/* ---------- the bar over the tabs: the project's name and its tabs (a game also has Promos) ---------- */
export function TopBar() {
  useTick();
  const view = H.view, curProj = H.curProj, page = H.isPage(view), game = H.isGameProj(curProj), nav = useRef<HTMLElement>(null);
  const pr = (H.projects.projects || []).find((x: any) => x.path === curProj);
  /* another computer's chats with no project ("<machine>~") are that computer's No project */
  const name = view === "newproj" ? "New project" : view === "all" ? "Overview" : view === "review" ? "Review" : curProj && !isHome(curProj) ? (pr ? pr.name : H.prName(curProj)) : "No project";
  /* the chosen tab's background slides from the last one to it */
  useEffect(() => { requestAnimationFrame(() => H.slideMark(nav.current)); }, [view, curProj, game, page]);
  return <>
    <button class="pname" id="b-proj" title={curProj && !page ? "The project's folder, notes and git history" : ""} disabled={!curProj || isHome(curProj) || page} onClick={() => H.openProject()}>{name}</button>
    <span class="grow" />
    <nav class="nav" id="tabs" ref={nav} hidden={page}>
      <button class={"tab" + (view === "chat" ? " on" : "")} id="v-chat" onClick={() => H.setView("chat")}>Chat</button>
      <button class={"tab" + (view === "promos" ? " on" : "")} id="v-promos" title="Promo videos made from your gameplay clips" hidden={!game} onClick={() => H.setView("promos")}>Promos</button>
    </nav>
  </>;
}

/* ---------- the project's chats: one small menu over the conversation ---------- */
/* a press that must be meant: the first one asks, the second (within a few seconds) does it */
function useSure(ms: number): [string, (id: string) => boolean] {
  const [sure, set] = useState(""), t = useRef(0);
  return [sure, (id) => { if (sure === id) { set(""); return true; } set(id); clearTimeout(t.current); t.current = window.setTimeout(() => set(""), ms); return false; }];
}
function ConvMenu({ close, rename }: { close(): void; rename(): void }) {
  const key = H.curProj || "", agents = H.agents, st = H.status, saved = H.saved, cur = agents.get(H.current);
  const [sureClose, askClose] = useSure(4000), [sureDel, askDel] = useSure(3000);
  const open: any[] = H.projectChats(key);
  const earlier = (saved.chats || []).filter((c: any) => !c.parent && c.role !== "research" && (c.project || "") === key && !agents.has(c.id)).slice(0, 30);
  const gone = (saved.deleted || []).filter((c: any) => !c.parent && c.role !== "research" && (c.project || "") === key).slice(0, 30);
  const onKey = (ev: KeyboardEvent) => { if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); ev.stopPropagation(); (ev.currentTarget as HTMLElement).click(); } };
  return <>
    <button class="cv-new" title={"Start another chat for " + (key ? "this " + H.thingOf(key) : "no project") + ". Chats that are working keep going at the same time."}
      disabled={(typeof st.slots_used === "number" ? st.slots_used : agents.size) >= (st.max_agents || 8)} onClick={() => { close(); H.showDraft(key); }}>
      <svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M5.5 1.5v8M1.5 5.5h8" /></svg>New chat
    </button>
    {open.length > 0 && <h5>Open</h5>}
    {open.map((p) => {
      /* closing a chat stops it and closes the experts it started: say so first when any of them is working */
      const xs = [...agents.values()].filter((h: any) => h.parent === p.id && h.busy), sure = sureClose === p.id;
      const x = (ev: Event) => { ev.stopPropagation(); if ((p.busy || xs.length) && !askClose(p.id)) return; api("/api/remove", { agent: p.id }); };
      return (
        <button class={"cv-row" + (p.id === H.current ? " on" : "")} key={p.id} onClick={() => { close(); H.select(p.id); }}>
          <span class={"dot" + (p.busy ? " live" : "")} /><span><b>{p.name}</b><small>{[H.aiName(H.chatAi(p)), p.busy ? p.now || "Working" : p.last].filter(Boolean).join(" · ")}</small></span>
          <span class="x" role="button" tabIndex={0} aria-label="Close chat" data-sure={sure ? "1" : undefined} onClick={x}
            title={sure ? (xs.length ? "At work: " + xs.map((h: any) => H.splitExpert(h.name, h.expert).join(", ")).join("; ") : "") : "Close this chat. It is not deleted: it stays under Earlier."}>
            {sure ? (p.busy && !xs.length ? "stop it?" : "This also stops " + xs.length + (xs.length === 1 ? " expert" : " experts") + " at work. Close?") : CLOSE}
          </span>
        </button>
      );
    })}
    {earlier.length > 0 && <h5>Earlier</h5>}
    {earlier.map((c: any) => {
      const sure = sureDel === c.id;
      /* a second press means it: the chat moves to Deleted (it can be put back) */
      const del = async (ev: Event) => { ev.stopPropagation(); if (!askDel(c.id)) return; await api("/api/delete-chat", { id: c.id, confirm: "delete" }); H.loadHistory(); };
      return (
        <button class="cv-row past" key={c.id} title={c.name} onClick={() => { close(); H.openSaved(c.id); }}>
          <span><b>{c.name}</b><small>{ago(c.updated) + " · " + c.messages + (c.messages === 1 ? " message" : " messages")}</small></span>
          <span class="x" role="button" tabIndex={0} data-sure={sure ? "1" : undefined} title="Delete this chat. It can be put back from Deleted, at the bottom of this list." onKeyDown={onKey} onClick={del}>{sure ? "Delete it?" : "Delete"}</span>
        </button>
      );
    })}
    {gone.length > 0 && (
      <details class="cv-gone" onClick={(ev) => ev.stopPropagation()}>
        <summary>Deleted ({gone.length})</summary>
        {gone.map((c: any) => (
          <div class="cv-row past" key={c.id}><span><b>{c.name}</b><small>{ago(c.updated)}</small></span>
            <button class="x" onClick={async (ev) => { ev.stopPropagation(); await api("/api/restore-chat", { id: c.id }); H.loadHistory(); }}>Put back</button></div>
        ))}
      </details>
    )}
    {cur && !H.isHelper(cur) && <div class="cv-foot"><button class="linkish" onClick={rename}>Rename this chat</button></div>}
  </>;
}

/* the chat's state in a few words, which is the way to Review when that is where to go */
function metaOf(p: any): [string, boolean, string] {
  const path = p && p.project && p.project.path, r = path && H.rvAll[path], run = r && r.run;
  const live = p && p.overnight && p.overnight > Date.now() / 1000;
  let text = "", review = false;
  if (!p) text = "";
  else if (H.isHelper(p)) { const m = H.memberOf(p.id), sw = m && H.SW[m.status]; text = [p.runsOn || H.aiName(H.chatAi(p)), sw && !(p.busy && m.status === "working") ? sw.word : H.xpState(p)[0]].join(" · "); }
  else if (H.leadsRun(p)) { text = H.runState(r, run)[0]; review = true; }
  else if (p.studio && p.studioWaiting) { text = "Waiting for you"; review = true; }
  else if (p.busy) text = "Working";
  else if (live) text = "Overnight until " + new Date(p.overnight * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  else if (p.last === "Needs you" || p.last === "Stopped") text = p.last;
  else if (p.last === "Needs attention") text = "Stopped with an error";
  return [text, review, path || ""];
}

/* The run's state as one button at the top right of the Lead's chat: a pause icon while the run is paused, a play icon
   while it goes. A press flips it, and the icon morphs at once: the two pause bars reshape into the two halves of the
   play triangle, and back. A run with nothing to flip (finished) shows its state in words instead. */
function RunPlay({ path }: { path: string }) {
  const r = H.rvAll[path], run = r && r.run;
  const [state, why, action, more] = run ? H.runState(r, run) : ["", "", "", ""];
  const [want, setWant] = useState<{ going: boolean; at: number } | null>(null), [err, setErr] = useState("");
  const going = action === "pause";
  /* the morph shows the press at once; the run's own state takes over when it agrees, or after a few seconds */
  const shown = want && want.going !== going && Date.now() - want.at < 6000 ? want.going : going;
  useEffect(() => { if (want && want.going === going) setWant(null); }, [going]);
  if (!action) return <span>{state}</span>;
  const press = async () => {
    setErr(""); setWant({ going: !going, at: Date.now() });
    let res; try { res = await api("/api/studio-run", { path, action }); } catch (e) { res = { ok: false, error: "Could not reach the app." }; }
    if (!res || !res.ok) { setWant(null); setErr((res && res.error) || "That did not work."); }
    H.loadReview(path);
  };
  const tip = state + ". " + (more || why) + (going ? " Press to pause the run." : " Press to resume the run.") + (err ? " " + err : "");
  /* while the run waits on the user, the way to Review stays beside the button */
  return <>
    {state === "Waiting for you" && <button class="linkish" title="Open Review" onClick={() => H.openInReview(path)}>{state}</button>}
    <button class={"run-play" + (shown ? " going" : "") + (err ? " err" : "")} type="button" onClick={press} title={tip}
      aria-label={(going ? "The run is going (" + state + "). Pause it" : "The run is paused. Resume it")} aria-pressed={shown ? "false" : "true"}>
      <span class="rp-ico" aria-hidden="true"><i /><i /></span>
    </button>
  </>;
}

/* what the conversation's own menus do when another opens, or Escape is pressed */
export const head = { closeConv: () => {}, closeRun: (_refocus?: boolean) => {} };

export function ChatHead() {
  useTick();
  const p = H.agents.get(H.current);
  const [open, setOpen] = useState(false), [ever, setEver] = useState(false), [renaming, setRenaming] = useState(false);
  const input = useRef<HTMLInputElement>(null), done = useRef(false);
  const close = () => setOpen(false);
  head.closeConv = close;
  useEffect(() => {
    const out = (ev: MouseEvent) => { if (!(ev.target as Element).closest(".conv")) setOpen(false); };
    document.addEventListener("click", out);
    return () => document.removeEventListener("click", out);
  }, []);
  useEffect(() => { if (renaming && input.current) { input.current.value = p ? p.name : ""; input.current.focus(); input.current.select(); } }, [renaming]);
  const rename = () => { if (!p) return; close(); done.current = false; setRenaming(true); };
  const finish = async (save: boolean) => {
    if (done.current) return; done.current = true;
    const name = input.current ? input.current.value.trim() : ""; setRenaming(false);
    if (save && p && name && name !== p.name) { p.name = name; poke(); await api("/api/rename", { agent: p.id, name }); }
  };
  const [text, review, path] = metaOf(p);
  /* the bar is tucked away behind a small tab at the top of the chat, and slides out when the tab is pressed; while
     it moves (and while tucked) it is clipped, and once out its menus may hang below it again */
  const [out, setOut] = useState(() => { try { return localStorage.getItem("headout") === "1"; } catch (e) { return false; } });
  const [moving, setMoving] = useState(false);
  const fold = () => { const v = !out; setOut(v); setMoving(true); setTimeout(() => setMoving(false), 320); if (!v) { close(); closePops(); } try { localStorage.setItem("headout", v ? "1" : "0"); } catch (e) {} };
  return (
    <div class={"headfold" + (out ? " out" : "") + (moving || !out ? " clip" : "")}>
    <div class="hf-in" inert={!out}>
    <div class="chathead">
      <span class="conv">
        <button class="title" id="chat-title" aria-haspopup="menu" aria-expanded={open ? "true" : "false"} title="This project's chats" hidden={renaming}
          onClick={(ev) => { ev.stopPropagation(); if (!open) { setEver(true); H.loadHistory(); } setOpen(!open); }}><span id="chat-name">{p ? p.name : "New chat"}</span>{DOWN}</button>
        {renaming && <input class="title-edit" ref={input} aria-label="Chat name" onBlur={() => finish(true)}
          onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); finish(true); } else if (e.key === "Escape") { e.stopPropagation(); finish(false); } }} />}
        <div class={"menu down" + (open ? " on" : "")} id="conv-menu" role="menu">{ever && <ConvMenu close={close} rename={rename} />}</div>
      </span>
      <span class="meta" id="chat-meta">{p && H.leadsRun(p) && path ? <RunPlay path={path} /> : !text ? null : review ? <button class="linkish" title="Open Review" onClick={() => H.openInReview(path)}>{text}</button> : text}</span>
      <RunBar />
    </div>
    </div>
    <button class="hf-tab" aria-expanded={out ? "true" : "false"} aria-label={out ? "Tuck the chat's bar away" : "Show the chat's bar"} title={out ? "Tuck the bar away" : (p ? p.name + ": " : "") + "show the chat's bar"} onClick={fold}><TabShape w={112} h={16} />{DOWN}</button>
    </div>
  );
}

/* ---------- the run's controls: one button at the top of a chat of a project with a run going, and behind it Pause or
   Resume, where the run waits for approval, experts at once, Undo this run and End this run ---------- */
const cpName = (c: any) => (c.stage === "start" ? "Start of the run" : "After " + H.stageName(c.stage).toLowerCase()) + " · " + ago(c.at || 0);
function UndoRun({ path, shut }: { path: string; shut(): void }) {
  const run = H.rvAll[path] && H.rvAll[path].run, seen = new Set<string>();
  const list = useRef<any[]>(((run && run.checkpoints) || []).slice().reverse().filter((c: any) => c && c.stage && !seen.has(c.stage) && seen.add(c.stage))).current;
  const [pick, setPick] = useState(list.length ? list[0].stage : ""), [busy, setBusy] = useState(""), [said, setSaid] = useState(""), [files, setFiles] = useState(false), [undone, setUndone] = useState(false);
  const run1 = async (filesOnly: boolean) => {
    setBusy(filesOnly ? "files" : "go");
    let res; try { res = await api("/api/studio-run", { path, action: "undo", stage: pick, files_only: filesOnly }); } catch (e) { res = { ok: false, error: "Could not reach the app." }; }
    setBusy("");
    if (res && res.ok) { setUndone(true); setSaid(res.text || " "); }
    else { setSaid((res && (res.error || res.text)) || "Could not undo the run."); if (res && res.files_only) setFiles(true); }
    H.loadReview(path);
  };
  const some = list.length > 0 && !undone;
  return (
    <div class="ask night-ask run-undo enter">
      <h4>{undone ? "The run was undone" : "Undo this run?"}</h4>
      <p>{said ? said.trim() : list.length
        ? "Goes back to the checkpoint you pick. The project's files go back to how they were then (the run's versions stay in git history). In Studio, the agents' changes since then are undone step by step, stopping at the first step that is yours; and the text of every script goes back to how it was at the checkpoint, your own and other chats' script edits included. First, the scripts as they are now are saved in the game's folder, in .fridae/before-undo/ under the time of the undo (the reply names the folder). Studio must still be open on the same place, without a restart since the checkpoint, with the current Fridae plugin. Later stages are to do again, review items filed since then move to the archive, and the run is left paused. The task cards are kept."
        : "This run has no checkpoints yet. One is taken when the run starts and after each finished stage."}</p>
      {some && <div class="cp-list" role="group" aria-label="Checkpoints, newest first">
        {list.map((c) => <button type="button" aria-pressed={pick === c.stage ? "true" : "false"} onClick={() => setPick(c.stage)}
          title={[c.commit ? "the project's files" : "", c.studio ? "Studio's scripts" : ""].filter(Boolean).join(" and ") || "No files or scripts were saved here: only the run's stages and review go back"}>{cpName(c)}</button>)}
      </div>}
      <div class="act">
        {some && <button class="pill" disabled={!!busy} onClick={() => run1(false)}>{busy === "go" ? "Undoing..." : "Undo this run"}</button>}
        {/* shown only after the app refused Studio's part (closed, restarted since, an old plugin, the scripts not
            saved): the files go back, Studio is left as it is */}
        {some && <button class="pill" hidden={!files} disabled={!!busy} onClick={() => run1(true)}>{busy === "files" ? "Undoing..." : "Undo the files only"}</button>}
        <button class="pill" disabled={!!busy} onClick={shut}>{some ? "Keep it" : "Close"}</button>
      </div>
    </div>
  );
}
/* "End this run": a confirm, then the run is switched off. Nothing is deleted, and a new run may start. */
function EndRun({ path, shut }: { path: string; shut(): void }) {
  const [busy, setBusy] = useState(false), [said, setSaid] = useState("");
  const go = async () => {
    setBusy(true);
    let r; try { r = await api("/api/studio-run", { path, action: "end" }); } catch (e) { r = { ok: false, error: "Could not reach the app." }; }
    if (r && r.ok) { shut(); if (H.rvAll[path] && H.rvAll[path].run) H.rvAll[path].run.on = false; poke(); } else { setBusy(false); setSaid((r && r.error) || "Could not end the run."); }
    H.loadReview(path);
  };
  return (
    <div class="ask night-ask enter">
      <h4>End this run?</h4>
      <p>{said || "The Lead and the experts it started are stopped now, and nothing new starts. The " + H.thingOf(path) + "'s files, review items, checkpoints and task cards stay as they are. Afterwards you can start a new run for it."}</p>
      <div class="act"><button class="pill" disabled={busy} onClick={go}>{busy ? "Ending..." : "End this run"}</button><button class="pill" disabled={busy} onClick={shut}>Keep it</button></div>
    </div>
  );
}
function RunBar() {
  const path = H.view === "chat" && H.curProj ? H.curProj : "", r = path ? H.rvAll[path] : null, run = r && r.run && r.run.on ? r.run : null;
  const [open, setOpen] = useState(false), [box, setBox] = useState({ path: "", undo: 0, end: 0, err: "" }), [toggling, setToggling] = useState(false);
  const more = useRef<HTMLButtonElement>(null), gates = useRef<HTMLDivElement>(null), max = useRef<HTMLDivElement>(null), setBusy = useRef(false), n = useRef(0);
  /* another project in front: what was open for the last one goes (an End or Undo card would act on it) */
  const mine = box.path === path ? box : { path, undo: 0, end: 0, err: "" };
  const shown = open && !!run && box.path === path;
  head.closeRun = (refocus) => { if (!shown) return; setOpen(false); if (refocus && more.current) more.current.focus(); };
  useEffect(() => {
    const out = (ev: MouseEvent) => { if (!(ev.target as Element).closest("#runbar")) setOpen(false); };
    document.addEventListener("mousedown", out);
    return () => document.removeEventListener("mousedown", out);
  }, []);
  /* where the run waits and how many experts work at once follow the run, except while a change is on its way */
  useEffect(() => {
    if (!run || setBusy.current || !gates.current || !max.current) return;
    const g = run.gates || [];
    gates.current.querySelectorAll("input").forEach((i) => (i.checked = g.includes(i.value)));
    max.current.querySelectorAll("button").forEach((b) => b.classList.toggle("on", +b.dataset.n! === (run.max_experts || 2)));
  });
  const [, why, action] = run ? H.runState(r, run) : ["", "", ""];
  const settings = async (body: any) => {
    setBusy.current = true; setBox({ ...mine, err: "" });
    let res; try { res = await api("/api/studio-run", Object.assign({ path, action: "settings" }, body)); } catch (e) { res = { ok: false, error: "Could not reach the app." }; }
    setBusy.current = false;
    if (!res.ok) setBox((b) => ({ ...b, err: res.error || "Could not change the settings." }));
    H.loadReview(path);
  };
  const toggle = async () => {
    setToggling(true); setBox({ ...mine, err: "" });
    let res; try { res = await api("/api/studio-run", { path, action }); } catch (e) { res = { ok: false, error: "Could not reach the app." }; }
    setToggling(false);
    if (!res.ok) setBox((b) => ({ ...b, err: res.error || "That did not work." }));
    H.loadReview(path);
  };
  return (
    <span class="run-ctl" id="runbar" hidden={!run} onKeyDown={(ev) => { if (ev.key === "Escape" && shown) { ev.preventDefault(); ev.stopPropagation(); head.closeRun(true); } }}>
      <button class="icon" id="run-more" ref={more} aria-label="The run" title="The run: pause, settings, undo, end" aria-haspopup="dialog" aria-expanded={shown ? "true" : "false"}
        onClick={(ev) => { ev.stopPropagation(); if (shown) { setOpen(false); return; } closePops(); head.closeConv(); if (box.path !== path) setBox(mine); setOpen(true); }}>
        <svg aria-hidden="true" width="16" height="16" viewBox="0 0 16 16" fill="currentColor"><circle cx="3.5" cy="8" r="1.25" /><circle cx="8" cy="8" r="1.25" /><circle cx="12.5" cy="8" r="1.25" /></svg>
      </button>
      <div class={"menu down run-menu" + (shown ? " on" : "")} id="run-menu" role="dialog" aria-label="The run">
        <button class="pill" data-r="rv-toggle" hidden={!action} disabled={toggling} title={why} onClick={toggle}>{action === "resume" ? "Resume the run" : "Pause the run"}</button>
        <span class="pr-label">Wait for my approval after</span>
        <div class="rv-gates" data-r="rv-gates" ref={gates} onChange={() => settings({ gates: [...gates.current!.querySelectorAll("input:checked")].map((i) => (i as HTMLInputElement).value) })}>
          {H.STAGE_NAMES.map(([id, name]: [string, string]) => <label class="check-row"><input type="checkbox" value={id} /><span>{name}</span></label>)}
        </div>
        <span class="pr-label">Experts at once</span>
        <div class="seg" data-r="rv-max" ref={max} onClick={(e) => { const b = (e.target as Element).closest("button"); if (!b) return; max.current!.querySelectorAll("button").forEach((x) => x.classList.toggle("on", x === b)); settings({ max_experts: +b.dataset.n! }); }}>
          {[1, 2, 3, 4].map((k) => <button data-n={k}>{k}</button>)}
        </div>
        <div class="act"><button class="pill sm" data-r="rv-undo" title="Put the project back as it was at one of the run's checkpoints" onClick={() => setBox({ ...mine, undo: ++n.current })}>Undo this run</button><button class="pill sm" data-r="rv-end" title="Stop this run so a new one can start. The project's files, review items and checkpoints stay." onClick={() => setBox({ ...mine, end: ++n.current })}>End this run</button></div>
        <div data-r="rv-undo-box">{mine.undo > 0 && <UndoRun key={mine.undo} path={path} shut={() => setBox((b) => ({ ...b, undo: 0 }))} />}</div>
        <div data-r="rv-end-box">{mine.end > 0 && <EndRun key={mine.end} path={path} shut={() => setBox((b) => ({ ...b, end: 0 }))} />}</div>
        <p class="rv-err" data-r="rv-err">{mine.err}</p>
      </div>
    </span>
  );
}
