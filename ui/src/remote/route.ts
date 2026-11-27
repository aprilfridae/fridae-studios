/* Other computers' chats and projects, as the page names them. The app forwards any call made to /m/<machine>/... to
   that computer (the client's side of remote control in app-rs), so the page only has to say which computer a chat or
   a project is on:
   - a chat on another computer has the page id "<machine>~<its id there>",
   - a project from it the path "<machine>~<its path there>" (and `machine` set),
   - its chats with no project belong to "<machine>~" (that computer's own No project).
   This computer's ids and paths are unchanged. A call that names such a chat or project goes to /m/<machine>/ with the
   bare id or path (route), and what comes back gets the prefix again (back), so the rest of the page never has to
   know. */
import { store, type Machine } from "../store";

export const SEP = "~";

/* only a computer this one is paired with counts: a path such as C:/PROGRA~1 is never taken for one */
const machine = (id: string): Machine | undefined => store.get().machines.find((x) => x.id === id);
export function machineOf(key: string | null | undefined): string {
  if (!key || typeof key !== "string") return "";
  const i = key.indexOf(SEP);
  return i > 0 && machine(key.slice(0, i)) ? key.slice(0, i) : "";
}
export const bare = (key: string) => { const m = machineOf(key); return m ? key.slice(m.length + 1) : key; };
export const withM = (m: string, v: string) => (m ? m + SEP + v : v);
/* where a chat with no project belongs: "" here, "<machine>~" on another computer */
export const homeOf = (id: string) => withM(machineOf(id), "");
export const isHome = (key: string | null | undefined) => !!key && key.endsWith(SEP) && !!machineOf(key) && key.length === machineOf(key).length + 1;
export const nameOf = (m: string) => (m ? (machine(m) || { name: "Another computer" }).name : "This PC");
export const online = (m: string) => !m || !!(machine(m) || { online: false }).online;
/* What is wrong with a machine, as the page says it: "" when it can be used; "offline"; "unpaired" (it no longer knows
   this computer: 401); "changed" (it answered with another certificate than at pairing, so it may not be the same
   computer). The two last need pairing again; nothing is sent to it until then. */
export const troubleOf = (m: string): string => { const x = m ? machine(m) : null; return !x ? "" : x.trouble || (x.online ? "" : "offline"); };
/* that, in a few words after the machine's name (the sidebar) and on its own line (the chip's menu) */
export const TROUBLE_SHORT: Record<string, string> = { offline: "offline", unpaired: "needs pairing again", changed: "identity changed" };
export const TROUBLE_LINE: Record<string, string> = { offline: "Offline", unpaired: "Needs pairing again", changed: "Identity changed" };
/* and when a message cannot go: the words stay in the box */
export function troubleSaid(m: string) {
  const t = troubleOf(m), n = nameOf(m);
  return t === "unpaired" ? n + " needs pairing again. Your message is still here."
    : t === "changed" ? n + "'s identity changed. Pair it again to be sure it is yours. Your message is still here."
    : "Couldn't reach " + n + ". Make sure Fridae is open there with Let my other computers connect on, and that both are on the same network. Your message is still here.";
}
/* the path of a call to the app's server on that computer */
export const at = (m: string, path: string) => (m ? "/m/" + encodeURIComponent(m) + path : path);

/* A machine's state as the last call to it found it. `trouble` undefined keeps what was known (a machine that needs
   pairing again still does while it is also offline). Only a change is written to the store. */
export function markOnline(m: string, on: boolean, trouble?: "" | "unpaired" | "changed") {
  const ms = store.get().machines, i = ms.findIndex((x) => x.id === m);
  if (i < 0) return;
  const t = trouble === undefined ? ms[i].trouble || "" : trouble;
  if (ms[i].online === on && (ms[i].trouble || "") === t) return;
  const next = ms.slice(); next[i] = { ...ms[i], online: on, trouble: t };
  store.set({ machines: next });
}
/* what this computer's app answered for a call to /m/<machine>/ (the contract): 401 "not paired", 502 "identity
   changed", 502 "offline", 404 "unknown computer"; anything else came from the machine itself */
export function markReply(m: string, status: number, body: any) {
  const err = body && typeof body === "object" ? body.error : "";
  if (status === 401) markOnline(m, false, "unpaired");
  else if (status === 502 && err === "identity changed") markOnline(m, false, "changed");
  else if (status === 502 || (status === 404 && err === "unknown computer")) markOnline(m, false);
  else markOnline(m, true, "");
}

/* the fields of a call's body, and of its query, that may name a chat or a project */
const NAMES = ["agent", "id", "target", "lead", "parent", "path", "project", "chat"];
export function route(path: string, body?: any): { url: string; body: any; m: string; rest: string } {
  const to = /^\/m\/([^/]+)(\/.*)$/.exec(path);
  if (to) return { url: path, body, m: decodeURIComponent(to[1]), rest: to[2] };
  const qi = path.indexOf("?"), base = qi < 0 ? path : path.slice(0, qi), query = qi < 0 ? "" : path.slice(qi + 1);
  const params = query ? query.split("&").map((kv) => { const j = kv.indexOf("="); return j < 0 ? [kv, null] : [kv.slice(0, j), decodeURIComponent(kv.slice(j + 1).replace(/\+/g, " "))]; }) as [string, string | null][] : [];
  let m = "";
  if (body && typeof body === "object" && !Array.isArray(body)) for (const k of NAMES) if (!m && typeof body[k] === "string") m = machineOf(body[k]);
  for (const [, v] of params) if (!m && v) m = machineOf(v);
  if (!m) return { url: path, body, m: "", rest: path };
  const strip = (v: any) => (typeof v === "string" && v.startsWith(m + SEP) ? v.slice(m.length + 1) : v);
  let b = body;
  if (body && typeof body === "object" && !Array.isArray(body)) { b = { ...body }; for (const k of NAMES) if (k in b) b[k] = strip(b[k]); }
  const q = params.map(([k, v]) => (v === null ? k : k + "=" + encodeURIComponent(strip(v)))).join("&");
  const rest = base + (q ? "?" + q : "");
  return { url: at(m, rest), body: b, m, rest };
}

/* What another computer answered, in the page's names: the chat it made or opened, the project, its projects, its
   saved chats, a project's Review (its pictures come through /m/ too) and a Lead's team. */
export function back(m: string, rest: string, r: any): any {
  if (!r || typeof r !== "object") return r;
  const p = rest.split("?")[0], pre = (v: any) => (typeof v === "string" && v ? m + SEP + v : v);
  if (typeof r.agent === "string") r.agent = pre(r.agent);
  if (r.project && typeof r.project === "object" && typeof r.project.path === "string") r.project = { ...r.project, path: pre(r.project.path), machine: m };
  if (p === "/api/projects" && Array.isArray(r.projects)) r.projects = r.projects.map((x: any) => ({ ...x, path: pre(x.path), machine: m }));
  if (p === "/api/history") {
    const fix = (c: any) => ({ ...c, id: pre(c.id), parent: pre(c.parent), project: m + SEP + (c.project || "") });
    if (Array.isArray(r.chats)) r.chats = r.chats.map(fix);
    if (Array.isArray(r.deleted)) r.deleted = r.deleted.map(fix);
  }
  if (p === "/api/review") {
    if (typeof r.project === "string") r.project = pre(r.project);
    if (r.run && r.run.lead) r.run = { ...r.run, lead: pre(r.run.lead) };
    r.via = at(m, "");
  }
  if (p === "/api/swarm" && Array.isArray(r.members)) r.members = r.members.map((x: any) => ({ ...x, id: pre(x.id) }));
  return r;
}

/* An event from another computer, in the page's names. What only concerns this computer's own window (its setup
   output, the promo engine, dropped files, its usage count) is left out. `live`: it happened after that computer's
   first batch, so it is new to the page; the first batch is history (the page compares an event's id with the id of
   this computer's first batch, `bootMark`). */
const HERE_ONLY = new Set(["setup", "promo_engine", "rag_setup", "mode", "clips", "attached", "promo", "usage"]);
export function eventFrom(m: string, e: any, live: boolean): any {
  if (!e || HERE_ONLY.has(e.kind)) return null;
  const x = { ...e, id: live ? Number.MAX_SAFE_INTEGER : 0 }, pre = (v: string) => m + SEP + v;
  for (const k of ["agent", "parent", "helper", "lead", "team", "req"]) if (typeof x[k] === "string" && x[k]) x[k] = pre(x[k]);
  if (x.kind === "project" && typeof x.path === "string") { x.path = pre(x.path); x.machine = m; }
  if (typeof x.project === "string" && x.project) x.project = pre(x.project);
  if (x.run && x.run.lead) x.run = { ...x.run, lead: pre(x.run.lead) };
  /* A picture is shown with the page's token after it, so only a path on that computer's own server is kept: one "/"
     not followed by "/" (a browser reads "//" as another host), and only plain path characters after it (no
     backslash, no space). Anything else (https://..., //host, /\host, javascript:...) is dropped, never shown. */
  const own = (u: any) => typeof u === "string" && /^\/(?!\/)[\w\-./%~+=&?#@!$'()*,;:]*$/.test(u);
  const urls = (list: any[]) => list.filter(own).map((u: string) => at(m, u));
  if (Array.isArray(x.urls)) x.urls = urls(x.urls);
  if (Array.isArray(x.images)) x.images = urls(x.images);
  if (x.item && typeof x.item.by_chat === "string" && x.item.by_chat) x.item = { ...x.item, by_chat: pre(x.item.by_chat) };
  return x;
}
