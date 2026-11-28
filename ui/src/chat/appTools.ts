/* The app toolbox in a chat: what an AI's call to one of Fridae's own tools (app-rs/src/app_mcp) looks like on the
   page. Ported from T3 Code's tool rows (packages/shared/src/t3McpToolPresentation.ts, its counted summaries in
   packages/client-runtime/src/t3ToolSummary.ts and work-log/presentation.ts; MIT, see THIRD_PARTY.md), in Fridae's
   own look: the app sends each call's words already made (`apptool` events: running, then done, failed or declined),
   so the page only draws them, keeps one line per call, and folds a finished turn's run of the same calls into one
   counted line ("Renamed 2 projects").

   Chats and projects a call or the agent's words name are links `[title](fridae://chat/<id>)` and
   `[name](fridae://project/<id>)`, drawn as chips that open them. A chip for a chat or project the page does not
   know stays plain words. */
import { H } from "./host";
import type { Part } from "./chatStore";
import { machineOf, SEP } from "../remote/route";

/* An AI's own step line for a call to the toolbox never reaches the page: the app counts it and sends no step
   (agents.rs `toolbox_step`), so the call's own row is its only line. */

/* what became of a card, in the words of its one line */
export const OUTCOME: Record<string, string> = { allowed: "allowed", declined: "declined", cancelled: "closed", timed_out: "timed out" };

/* ---------- links ---------- */
export type Link = { kind: "chat" | "project"; id: string; title: string };
const ID = /^[\w.~-]{1,120}$/;
/* the page's name for an id a chat's AI gave: a chat on another computer names that computer's chats and projects
   by their own ids, which the page knows with the computer in front */
function inPage(kind: string, id: string, from: string): string {
  const m = machineOf(from);
  if (!m || kind === "project") return id;
  return id.startsWith(m + SEP) ? id : m + SEP + id;
}
function projectOf(id: string, from: string) {
  const m = machineOf(from), list: any[] = (H.projects && H.projects.projects) || [];
  const listed = list.find((x) => x && x.id === id && (x.machine || "") === m);
  if (listed) return listed;
  /* the chat's own project, by the id its "project" event carried (one on this computer) */
  const own = !m && H.agents.get(from);
  return own && own.projectId === id && own.project && own.project.path ? { id, path: own.project.path } : null;
}
/* the link leads somewhere the page knows: an open chat, a saved one, a project in the list */
export function known(kind: string, id: string, from: string): boolean {
  if (!ID.test(id || "")) return false;
  if (kind === "project") return !!projectOf(id, from);
  if (kind !== "chat") return false;
  const pid = inPage(kind, id, from);
  return H.agents.has(pid) || ((H.saved && H.saved.chats) || []).some((c: any) => c && c.id === pid);
}
/* a press on a chip: the chat, or the project; anything the page does not know does nothing */
export function openLink(kind: string, id: string, from: string) {
  if (!known(kind, id, from)) return;
  if (kind === "chat") H.openChatById(inPage(kind, id, from));
  else H.showProject(projectOf(id, from).path, "chat");
}
/* chips made by md.ts (in an agent's words) are plain markup: a press on one inside `root` goes here; and those
   leading nowhere the page knows become plain words */
export function wireChips(root: HTMLElement, from: string) {
  root.querySelectorAll<HTMLAnchorElement>("a.app-link").forEach((a) => {
    const kind = a.dataset.chat !== undefined ? "chat" : "project", id = a.dataset.chat ?? a.dataset.project ?? "";
    if (!known(kind, id, from)) {
      const s = document.createElement("span"); s.className = "app-link plain"; s.textContent = a.textContent; a.replaceWith(s); return;
    }
    a.onclick = (ev) => { ev.preventDefault(); ev.stopPropagation(); openLink(kind, id, from); };
  });
}

/* ---------- a finished turn: runs of two or more of the same finished call fold into one counted line ---------- */
export function foldAppTools(parts: Part[], make: (k: string, more: Record<string, any>) => Part): Part[] {
  const out: Part[] = [];
  let run: Part[] = [];
  const same = (a: Part, b: Part) => a.status === "done" && b.status === "done" && a.summary && b.summary && a.summary.verb === b.summary.verb && a.summary.many === b.summary.many;
  const flush = () => {
    if (run.length >= 2) out.push(make("appgroup", { verb: run[0].summary.verb, many: run[0].summary.many, items: run, open: false }));
    else out.push(...run);
    run = [];
  };
  for (const pt of parts) {
    if (pt.k === "apptool" && pt.status === "done" && pt.summary && pt.summary.verb) {
      if (run.length && !same(run[0], pt)) flush();
      run.push(pt);
    } else { flush(); out.push(pt); }
  }
  flush();
  return out;
}
/* "Renamed 2 projects" */
export const counted = (g: Part) => g.verb + " " + g.items.length + " " + g.many;

/* A chip's href is its own fridae:// link, never the page's address (which carries the window's token): copying it
   gives the title, or that link. The window never follows it: a press of any button is the page's own. */
if (typeof document !== "undefined") {
  const stop = (ev: Event) => { const t = ev.target as Element | null; if (t && t.closest && t.closest("a.app-link")) ev.preventDefault(); };
  document.addEventListener("auxclick", stop, true);
  document.addEventListener("click", stop, true);
}
