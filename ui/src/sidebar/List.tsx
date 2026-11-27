/* The sidebar's list: Overview, Review, each project, the chats and experts of the one in front, No project and New
   project. It is one list that scrolls as a whole. Rows keep their elements from one draw to the next (keys), so the
   keyboard stays on the row it was on, and only the rows whose values changed are drawn again (rows.tsx). */
import { useEffect, useLayoutEffect, useReducer, useRef } from "preact/hooks";
import { useStore } from "../store";
import { drawn, endEase, sideCalm } from "./ease";
import { grow, redraw, setXpOpen, xpTab } from "./fold";
import { rowsOf, type Row } from "./model";
import { closeMenu, menu, openMenu } from "./ProjectMenu";
import { AddRow, AllRow, ChatRow, ExpertRowView, MoreRowView, NoneRow, ProjectRow, ReviewRow } from "./rows";

function draw(r: Row, menuAt: string): any {
  switch (r.t) {
    case "all": return <AllRow key={r.key} on={r.on} busy={r.busy} />;
    case "review": return <ReviewRow key={r.key} on={r.on} n={r.n} k={r.k} strong={r.strong} />;
    case "proj": return <ProjectRow key={r.key} path={r.path} name={r.name} cls={r.cls} n={r.n} strong={r.strong} working={r.working} menu={menuAt === r.path} ais={r.ais} where={r.where} />;
    case "chat": return <ChatRow key={r.key} id={r.id} name={r.name} on={r.on} live={r.live} count={r.count} open={r.open} tip={r.tip} ai={r.ai} where={r.where} />;
    case "none": return <NoneRow key={r.key} path={r.path} cls={r.cls} busy={r.busy} ais={r.ais} where={r.where} />;
    /* a project and, the one in front, its chats: a box of its own */
    case "pbox": return <div key={r.key} class={"pbox" + (r.on ? " on" : "") + (r.off ? " off" : "")} data-box={r.path}>{r.rows.map((x) => draw(x, menuAt))}</div>;
    case "add": return <AddRow key={r.key} on={r.on} />;
    case "expert": return <ExpertRowView key={r.key} id={r.id} of={r.of} cls={r.cls} dot={r.dot} who={r.who} job={r.job} small={r.small} turnTip={r.turnTip} tip={r.tip} on={r.on} tab={r.tab} />;
    case "more": return <MoreRowView key={r.key} id={r.id} text={r.text} open={r.open} tip={r.tip} />;
    /* a chat's experts: a window (.s-kids) over its rows (.s-in), which eases open and shut as a whole (ease.ts) */
    case "box": return <div key={r.key} class={r.cls} id={r.domId} role="group" aria-label={r.label} inert={r.inert}><div class="s-in">{r.rows.map((x) => draw(x, menuAt))}</div></div>;
  }
}

export function List() {
  const nav = useStore((s) => s.nav), chats = useStore((s) => s.chats), projects = useStore((s) => s.projects), review = useStore((s) => s.review), reviewDone = useStore((s) => s.reviewDone);
  const saved = useStore((s) => s.saved), teams = useStore((s) => s.teams), ais = useStore((s) => s.ais), machines = useStore((s) => s.machines), defaultAi = useStore((s) => { const d = ((s.status || {}).ai || {}).default; return s.ais.some((a) => a[0] === d) ? d : "claude"; });
  const m = menu.use(), [, tick] = useReducer((n: number) => n + 1, 0), list = useRef<HTMLElement>(null), front = useRef<string | null>(null), addWas = useRef<number | null>(null);
  redraw.use();
  /* an expert leaves the list half an hour after its last work, with nothing else happening: looked at twice a minute */
  useEffect(() => { const t = setInterval(() => { if (!document.hidden) tick(0); }, 30000); return () => clearInterval(t); }, []);
  /* an expert in front is shown in the sidebar, under its chat */
  useEffect(() => {
    const p = chats.find((x) => x.id === nav.chat);
    if (p && p.helper && p.parent && nav.view === "chat") { setXpOpen(p.parent, true); if (grow.size) redraw.poke(); }
  }, [nav.chat]);
  /* a list is about to open: an ease still going ends first, and where New project is now is noted (it is pinned,
     so it goes its own way) */
  addWas.current = null;
  if (grow.size && list.current) { endEase(); const add = !sideCalm() && list.current.querySelector(".proj.add"); if (add) addWas.current = add.getBoundingClientRect().top; }
  /* the row the keyboard is on, noted before anything moves (an expert's row may change lists) */
  const at = document.activeElement as HTMLElement | null, had = list.current && at && list.current.contains(at) ? at.dataset.k || "" : "";
  const rows = rowsOf({ nav, chats, projects, review, reviewDone, saved, teams, ais, machines, defaultAi }, Date.now());
  useLayoutEffect(() => {
    const el = list.current; if (!el) return;
    const now = document.activeElement as HTMLElement | null;
    if (had && (!now || !el.contains(now) || now.dataset.k !== had)) {
      const again = [...el.querySelectorAll<HTMLElement>("[data-k]")].find((n) => n.dataset.k === had);
      if (again) again.focus({ preventScroll: true });
    }
    drawn(addWas.current);
    /* a newly marked chat is brought into sight */
    const f = nav.view === "chat" ? nav.chat : null;
    if (f !== front.current) { front.current = f; const n = el.querySelector(".s-row.on"); if (n) n.scrollIntoView({ block: "nearest" }); }
  });
  const menuKey = (ev: KeyboardEvent) => {
    if (!(ev.key === "ContextMenu" || (ev.key === "F10" && ev.shiftKey))) return false;
    const row = (ev.target as Element).closest<HTMLElement>(".proj"); if (!row || !(row.dataset.k || "").startsWith("p:")) return false;
    ev.preventDefault(); ev.stopPropagation(); openMenu(row.dataset.k!.slice(2), row);
    return true;
  };
  /* the arrow keys go from row to row; right and left show and hide a chat's experts (and its earlier ones), and
     left on an expert goes to its chat */
  const keys = (ev: KeyboardEvent) => {
    if (menuKey(ev)) return;
    if (!["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(ev.key) || ev.altKey || ev.ctrlKey || ev.metaKey) return;
    const all = [...list.current!.querySelectorAll("button")].filter((b) => !b.closest("[inert]")), me = document.activeElement as HTMLElement, i = all.indexOf(me as HTMLButtonElement);
    if (i < 0) return;
    if (ev.key === "ArrowDown" || ev.key === "ArrowUp") { const to = all[i + (ev.key === "ArrowDown" ? 1 : -1)]; if (to) { ev.preventDefault(); to.focus(); } return; }
    const chat = me.closest(".s-chat"), tw = (me.matches(".s-more") ? me : chat && chat.querySelector<HTMLElement>(".s-tw")) as HTMLElement | null;
    if (tw && (tw.getAttribute("aria-expanded") === "true") !== (ev.key === "ArrowRight")) { ev.preventDefault(); tw.click(); }
    else if (ev.key === "ArrowLeft" && me.dataset.of) { const to = all.find((n) => n.dataset.k === me.dataset.of); if (to) { ev.preventDefault(); to.focus(); } }
  };
  const context = (ev: MouseEvent) => {
    const row = (ev.target as Element).closest<HTMLElement>(".proj"); if (!row || !(row.dataset.k || "").startsWith("p:")) return;
    ev.preventDefault();
    /* from the keyboard there is no pointer: the menu opens under the row */
    openMenu(row.dataset.k!.slice(2), ev.button === 2 || ev.clientX || ev.clientY ? { x: ev.clientX, y: ev.clientY } : row);
  };
  /* the expert the keyboard is on is the one Tab comes back to */
  const focused = (ev: FocusEvent) => {
    const b = ev.target as HTMLElement; if (!b.matches || !b.matches(".s-row.sx") || !b.dataset.of) return;
    const lead = b.dataset.of.slice(2);
    if (xpTab.get(lead) !== b.dataset.k) { xpTab.set(lead, b.dataset.k!); redraw.poke(); }
  };
  return <nav class="side-list" id="proj-list" ref={list} onKeyDown={keys} onContextMenu={context} onScroll={() => closeMenu(false)} onfocusin={focused}>{rows.map((r) => draw(r, m.on ? m.path : ""))}</nav>;
}
