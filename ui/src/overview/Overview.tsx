/* Overview: the live graph of every project, its chats and its experts, over the whole page. Behind a tab at its top
   (the same pull-down bar as a chat's) is how much is going, each project in a few words, and the way to stop it all. */
import { useEffect, useRef } from "preact/hooks";
import { api } from "../api";
import { store, useStore, type State } from "../store";
import { runActive } from "../review/model";
import { skillsWaiting } from "../settings/skills-data";
import { projState } from "../sidebar/model";
import { openSetup } from "../windows";
import { attach, tick, watch } from "./graph";
import { PullBar } from "../chat/PullBar";
import { H } from "../chat/host";
import { host } from "../host";
import "./overview.css";

const plural = (n: number, one: string, many: string) => n + (n === 1 ? one : many);
/* how much is going, in one line; nothing is worked out while Overview is not in front */
function totals(s: State) {
  if (s.nav.view !== "all") return "";
  const list = s.projects, st = list.map((x) => projState(s, x.path));
  /* at work: something waits for the user, a chat is working, or a run is going */
  const going = list.filter((x, i) => st[i].n || st[i].working || runActive(((s.review[x.path] || {}) as any).run)).length;
  const working = s.chats.filter((p) => p.busy).length, wait = st.reduce((n, x) => n + x.n, 0);
  return [!list.length ? "No projects yet" : going ? plural(going, " project at work", " projects at work") : "Nothing going", working ? plural(working, " agent working", " agents working") : "", wait ? plural(wait, " thing waits for you", " things wait for you") : ""].filter(Boolean).join(" · ");
}
/* each project in a few words, those with something going first: whether a chat of it is working, what waits for the
   user, and its studio run's state */
function projects(s: State) {
  if (s.nav.view !== "all") return [];
  return s.projects.map((x) => {
    const st = projState(s, x.path), r: any = s.review[x.path] || {}, run = r.run;
    const busy = s.chats.filter((p) => p.busy && p.project === x.path).length;
    const going = runActive(run), rs = going && H.runState ? H.runState(r, run) : null;
    return { path: x.path, name: x.name, busy, n: st.n, run: rs ? rs[0] : "", why: rs ? rs[1] : "", live: !!(busy || st.n || going) };
  }).sort((a, b) => +b.live - +a.live);
}
/* the bar behind the Overview's tab: how much is going and the way to stop it all, then each project in one row */
function OvBar() {
  const text = useStore(totals), working = useStore((s) => s.nav.view === "all" && s.chats.some((p) => p.busy)), n = skillsWaiting.use(), last = useRef("");
  /* read as one string, so the bar is painted again only when what it says changes */
  const list: ReturnType<typeof projects> = JSON.parse(useStore((st) => JSON.stringify(projects(st))));
  if (text) last.current = text;
  const stop = () => { for (const p of store.get().chats) if (p.busy) api("/api/stop", { agent: p.id }); };
  return (
    <PullBar keep="ovout" what="what is going" bar="ovbar" cls="ov-fold">
      <div class="ob-head">
        <p class="sub"><span id="ap-totals">{text || last.current}</span><span id="ap-sk" hidden={!n}> · <button class="linkish" id="ap-sk-go" title="Open Settings, Skills" onClick={() => openSetup("skills")}>{plural(n, " decision waits", " decisions wait")}</button></span></p>
        <button class="pill sm" id="ap-stop-all" hidden={!working} onClick={stop}>Stop everything</button>
      </div>
      {list.length > 0 && <ul class="fold-pills" aria-label="Every project">
        {list.map((x) => (
          <li key={x.path}>
            <button class={"fp" + (x.live ? " live" : "")} title={"Open " + x.name + (x.why ? ". " + x.why : "")} onClick={() => host.showProject(x.path)}>{x.name}</button>
          </li>
        ))}
      </ul>}
    </PullBar>
  );
}
/* what the graph shows, behind one small "?" in its corner. It closes like a menu: on Escape (the keyboard goes back
   to the "?") and on a press anywhere else. */
function Key() {
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const esc = (ev: KeyboardEvent) => { const key = ref.current; if (key && ev.key === "Escape" && key.open) { const inside = key.contains(document.activeElement); key.open = false; if (inside) key.querySelector("summary")!.focus(); } };
    const away = (ev: MouseEvent) => { const key = ref.current; if (key && key.open && !key.contains(ev.target as Node)) key.open = false; };
    document.addEventListener("keydown", esc); document.addEventListener("mousedown", away);
    return () => { document.removeEventListener("keydown", esc); document.removeEventListener("mousedown", away); };
  }, []);
  return (
    <details class="o-key" id="o-key" ref={ref}><summary aria-label="What the graph shows" title="What the graph shows">?</summary>
      <p id="o-key-text">Square: a project. Dot: a chat. Small dot: an expert, tied to its Lead.<br />Bright and pulsing: working. Grey: open. Dim: saved.<br />A faint expert is parked; a ring means its work is being checked.<br />Click a project for its Review, a chat to open it.<br />Drag to arrange, scroll to zoom, double-click to fit.</p>
    </details>
  );
}
/* the canvas: the graph draws on it (graph.ts) while Overview is in front and the window can be seen */
function Graph() {
  const cv = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const let_go = attach(cv.current!);
    /* the store changed: Overview came or went, or there is something else to show. Nothing is drawn again by
       Preact for it: the graph looks for itself, and only while it can be seen. */
    const pick = () => { const s = store.get(); return [s.chats, s.steps, s.saved, s.projects, s.review, s.teams, s.nav.chat, s.scale]; };
    let view = store.get().nav.view, had = pick();
    const off = store.subscribe(() => {
      const s = store.get(), now = pick();
      if (s.nav.view !== view) { view = s.nav.view; had = now; watch(); }
      else if (view === "all" && now.some((v, i) => v !== had[i])) { had = now; tick(); }
    });
    return () => { off(); let_go(); };
  }, []);
  return <canvas id="o-canvas" ref={cv} role="img" aria-label="Graph of every project, its chats and the experts at work" aria-describedby="o-key-text"></canvas>;
}
export function Overview() {
  return (
    <div class="ov">
      <div class="o-graph"><Graph /><Key /><OvBar /></div>
    </div>
  );
}
