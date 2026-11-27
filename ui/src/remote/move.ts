/* Switching the computer a started chat runs on, from the machine chip's menu. Nothing else about the chat changes:
   the same chat (its id, its messages, its place in the list) carries on on the other computer, its project's files
   copied there first, so either computer has them all; every window sees the one chat. The page drives it with the app
   on each side (POST /api/chat-move, agents.rs; POST /api/project-copy, project_copy.rs):
   - while the chat works, it is held ("hold") and switches when the reply is over; what the user sends meanwhile
     goes with it;
   - the project's files are copied to that computer's project of the same name (made there if it has none):
     "copy", run by this window's own app, which asks both computers;
   - "out" on the computer it is on gives the chat as it goes, "in" on the other opens it there with its copy of the
     project, and "away" closes it where it was (kept there, out of the lists).
   Nothing is put away until the other computer has it: a failure leaves the chat where it was. */
import { api } from "../api";
import { H } from "../chat/host";
import { poke } from "../chat/chatStore";
import { note } from "../chat/compose";
import { at, bare, machineOf, nameOf, withM } from "./route";

/* "wait": for the reply to be over; "copy": the project's files are going; "go": the chat is going */
export type Stage = "wait" | "copy" | "go";
type Pending = { to: string; stage: Stage };
/* chat id (as the page has it) -> where it goes */
const pending: Record<string, Pending> = {};

export const movingTo = (id: string): string | null => (id in pending ? pending[id].to : null);
export const stageOf = (id: string): Stage | null => (id in pending ? pending[id].stage : null);

const call = (m: string, body: any) => api(at(m, "/api/chat-move"), body).catch(() => null);
const cut = (id: string) => { delete pending[id]; poke(); };

/* The chip's choice in a started chat: run it on `to` ("" is this computer). Picking the computer it is on takes back
   a switch that waits for the reply. */
export async function moveChat(id: string, to: string) {
  const from = machineOf(id), p = H.agents && H.agents.get(id);
  if (!p) return;
  if (to === from) { if (id in pending && pending[id].stage === "wait") await release(id); poke(); return; }
  if (id in pending && pending[id].stage !== "wait") return;   /* already on its way */
  pending[id] = { to, stage: "wait" };
  poke();
  if (p.busy) {
    const r = await call(from, { action: "hold", agent: bare(id), on: true });
    if (!r || !r.ok) { cut(id); note((r && r.error) || "Couldn't reach " + nameOf(from) + ". The chat stays where it is."); }
    return;
  }
  await go(id);
}

/* a switch that waited is taken back: what the user sent meanwhile goes on where the chat is */
async function release(id: string) {
  delete pending[id];
  await call(machineOf(id), { action: "hold", agent: bare(id), on: false });
}

/* the chat's reply is over (its "done"): a switch that waited for it goes now */
export function replyOver(id: string) {
  if (id in pending && pending[id].stage === "wait") setTimeout(() => go(id), 400);
}

async function go(id: string, tries = 0) {
  const want = pending[id];
  if (!want) return;
  const from = machineOf(id), to = want.to, p = H.agents && H.agents.get(id);
  /* (asked now: once it is put away here, the page has already moved on from it) */
  const front = H.current === id;
  const proj = p && p.project && typeof p.project.path === "string" && p.project.path ? p.project.path : "";
  /* the project's files first, so the chat finds them there */
  let there: string | null = null;
  if (proj) {
    want.stage = "copy"; poke();
    const c = await api("/api/project-copy", { action: "copy", from, path: bare(proj), to }).catch(() => null);
    if (pending[id] !== want) return;   /* taken back meanwhile */
    if (!c || !c.ok) {
      await release(id); poke();
      note(((c && c.error) || "Couldn't copy the project's files to " + nameOf(to) + ".") + " The chat stays on " + nameOf(from) + ".");
      return;
    }
    there = c.path;
  }
  want.stage = "go"; poke();
  const out = await call(from, { action: "out", agent: bare(id) });
  if (pending[id] !== want) return;
  if (!out || !out.ok) {
    /* the turn is ending as this is asked: once more in a moment */
    if (out && /still working/.test(out.error || "") && tries < 30) { want.stage = "wait"; setTimeout(() => go(id, tries + 1), 1000); return; }
    cut(id);
    note((out && out.error) || "Couldn't reach " + nameOf(from) + ". The chat stays where it is.");
    return;
  }
  const inn = await call(to, { action: "in", chat: out.chat, project: there });
  if (!inn || !inn.ok) {
    await release(id); poke();
    note((inn && inn.error) || "Couldn't reach " + nameOf(to) + ". The chat stays where it is.");
    return;
  }
  /* the other computer has it: it is put away here (a failure leaves two copies, never none) */
  const away = await call(from, { action: "away", agent: bare(id) });
  delete pending[id];
  if (!away || !away.ok) note("The chat runs on " + nameOf(to) + " now, but " + nameOf(from) + " couldn't put its old copy away. You can delete that one.");
  /* a project made there for the copy is in the list at once, so the chat stays under its project */
  if (there && H.loadProjects) await Promise.resolve(H.loadProjects()).catch(() => {});
  /* (api() already gives ids of another computer's answer in the page's names) */
  const next = machineOf(inn.agent) === to ? inn.agent : withM(to, inn.agent);
  /* its events come in with the other computer's next poll: shown in front as soon as it is there */
  for (let i = 0; i < 100 && !(H.agents && H.agents.has(next)); i++) await new Promise((r) => setTimeout(r, 150));
  if (front && H.agents && H.agents.has(next)) H.select(next);
  poke();
}
