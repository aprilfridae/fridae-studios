/* The computers this one is paired with: their list, one long-poll of events each, their projects and saved chats
   beside this computer's, and their status for the message box. This computer's own polling, projects and history
   stay as they were in ui/index.html; what comes from another computer is added to them there (withProjects,
   withHistory, host.remoteEvents). */
import { api } from "../api";
import { host } from "../host";
import { H } from "../chat/host";
import { poke } from "../chat/chatStore";
import { store, type Machine } from "../store";
import { at, eventFrom, machineOf, markOnline, withM } from "./route";

const cursor: Record<string, number> = {};          /* machine -> the id of the last event read from it */
const following = new Set<string>();                 /* the machines a long-poll runs for */
const projs: Record<string, any[]> = {};             /* machine -> its projects as last read: still shown, dimmed, while it is offline */
const hist: Record<string, { chats: any[]; deleted: any[] }> = {};
const stat: Record<string, { at: number; s: any }> = {};   /* machine -> its GET /api/status, for the AI chip */
const sleep = (ms: number) => new Promise((ok) => setTimeout(ok, ms));
const known = (m: string) => store.get().machines.some((x) => x.id === m);

/* GET /api/remote/machines (every 20 s): a machine is online as its own long-poll last found it, once one runs */
export async function loadMachines() {
  let r: any = null;
  try { r = await api("/api/remote/machines"); } catch (e) { return; }
  if (!r || !Array.isArray(r.machines)) return;
  const was = store.get().machines;
  const list: Machine[] = r.machines.map((x: any) => { const old = was.find((w) => w.id === x.id); return { ...x, online: old && following.has(x.id) ? old.online : !!x.online }; });
  if (JSON.stringify(list) !== JSON.stringify(was)) store.set({ machines: list });
  for (const x of list) if (!following.has(x.id)) follow(x.id);
}

/* One long-poll per machine. Its first batch is the chats it has open (history to the page); what follows is new.
   While it cannot be reached it is marked offline and asked again after 2 s, then 4, 8... up to 30 s; when it answers
   again its projects and saved chats are read again. */
async function follow(m: string) {
  following.add(m);
  let wait = 2000, down = true;
  while (known(m)) {
    let r: any = null;
    try { r = await api(at(m, "/api/events?after=" + (cursor[m] || 0))); } catch (e) {}
    if (!known(m)) break;
    if (r && Array.isArray(r.events)) {
      const first = !(m in cursor), list: any[] = [];
      let after = cursor[m] || 0;
      for (const e of r.events) { if (typeof e.id === "number" && e.id > after) after = e.id; const x = eventFrom(m, e, !first); if (x) list.push(x); }
      cursor[m] = after; wait = 2000;
      markOnline(m, true);
      if (list.length) host.remoteEvents(list);
      if (down) { down = false; host.loadProjects(); host.reloadSaved(); }
    } else {
      markOnline(m, false); down = true;
      await sleep(wait); wait = Math.min(wait * 2, 30000);
    }
  }
  following.delete(m);
}

/* this computer's GET /api/projects with each paired machine's added after it (an offline one's as last read) */
export async function withProjects(reply: any) {
  const ms = store.get().machines;
  await Promise.all(ms.map(async (x) => {
    if (!x.online) return;
    try { const r = await api(at(x.id, "/api/projects")); if (r && Array.isArray(r.projects)) projs[x.id] = r.projects; } catch (e) {}
  }));
  const more = ms.flatMap((x) => projs[x.id] || []);
  return more.length ? { ...reply, projects: (reply.projects || []).concat(more) } : reply;
}
/* this computer's GET /api/history with each paired machine's saved chats added */
export async function withHistory(saved: any) {
  const ms = store.get().machines;
  await Promise.all(ms.map(async (x) => {
    if (!x.online) return;
    try { const r = await api(at(x.id, "/api/history")); if (r && Array.isArray(r.chats)) hist[x.id] = { chats: r.chats, deleted: r.deleted || [] }; } catch (e) {}
  }));
  const hs = ms.map((x) => hist[x.id]).filter(Boolean);
  if (!hs.length) return saved;
  return { ...saved, chats: (saved.chats || []).concat(...hs.map((h) => h.chats)), deleted: (saved.deleted || []).concat(...hs.map((h) => h.deleted)) };
}

/* the machine the message box sends to: the chat in front's, or on an empty page its project's ("" is this computer) */
export function boxMachine(): string {
  const p = H.agents && H.agents.get(H.current);
  return p ? machineOf(p.id) : H.draftOn ? machineOf(H.curProj) : "";
}
/* that machine's status, so the AI chip offers its AIs and models (null for this computer, or until it has come) */
export function boxStatus(): any {
  const m = boxMachine(); if (!m) return null;
  const s = stat[m];
  if (!s || Date.now() - s.at > 20000) readStatus(m);
  return s ? s.s : null;
}
const asking = new Set<string>();
async function readStatus(m: string) {
  if (asking.has(m)) return;
  asking.add(m);
  let r: any = null;
  try { r = await api(at(m, "/api/status")); } catch (e) {}
  /* a failed read is kept as a time too, so it is not asked again on every paint */
  stat[m] = { at: Date.now(), s: r && r.ai ? r : stat[m] ? stat[m].s : null };
  asking.delete(m);
  if (r && r.ai) poke();
}

/* The chip's choice, before the first message: an empty page on that machine's own No project. Coming back to this
   computer goes back to the project the page was on before another machine was picked. */
let cameFrom: string | null = null;
export function choose(m: string) {
  const now = boxMachine();
  if (m === now) return;
  if (!now && H.curProj !== null) cameFrom = H.curProj;
  host.newChat(m ? withM(m, "") : cameFrom !== null && !machineOf(cameFrom) ? cameFrom : "");
}

export function start() { loadMachines(); setInterval(loadMachines, 20000); }
