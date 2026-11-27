/* Moving a started chat to another of the user's computers, from the machine chip's menu: the chat carries on there,
   with all its messages, and is put away where it was, so every window sees one chat. This page drives it with the app
   on each side (POST /api/chat-move, agents.rs):
   - while the chat works, it is held ("hold") and moves when the reply is over; what the user sends meanwhile goes
     with it;
   - "out" on the computer it is on gives the chat as it goes, "in" on the other opens it there (a new id) with a
     project of that computer's, and "away" closes it where it was (kept there, out of the lists);
   - the project: the other computer's project of the same name; with none, the chip asks which (or none).
   Nothing is put away until the other computer has it: a failure leaves the chat where it was. */
import { api } from "../api";
import { H } from "../chat/host";
import { poke } from "../chat/chatStore";
import { C, note } from "../chat/compose";
import { store } from "../store";
import { at, bare, machineOf, nameOf, withM } from "./route";
import { thisName } from "./remote";

type Pending = { to: string; project: string | null };
/* chat id (as the page has it) -> where it goes when its reply is over */
const pending: Record<string, Pending> = {};
/* the chat whose project on the other computer the chip is asking for, and the choices */
export const asking: { id: string; to: string; list: { path: string; name: string }[] } = { id: "", to: "", list: [] };

export const movingTo = (id: string): string | null => (id in pending ? pending[id].to : null);

/* the other computer's name, as the line in the moved chat says it ("Moved here from Laptop.") */
function computerName(m: string) {
  if (!m) return thisName() || "your other computer";
  const x = store.get().machines.find((y) => y.id === m);
  return x ? x.name : "your other computer";
}
const call = (m: string, body: any) => api(at(m, "/api/chat-move"), body).catch(() => null);

/* the projects of computer `m`, as the page lists them */
function projectsOn(m: string): { path: string; name: string }[] {
  const all: any[] = (H.projects && H.projects.projects) || [];
  return all.filter((x) => x && typeof x.path === "string" && machineOf(x.path) === m && !x.path.endsWith("~"));
}
const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

/* The chip's choice in a started chat: move it to `to` ("" is this computer). Picking the computer it is on takes back
   a move that waits for the reply. `project` is the answer to the chip's question (a path there, or null for none). */
export async function moveChat(id: string, to: string, project?: string | null) {
  const from = machineOf(id), p = H.agents && H.agents.get(id);
  asking.id = "";
  if (!p) return;
  if (to === from) { if (id in pending) await release(id); poke(); return; }
  if (project === undefined) {
    const had = p.project && typeof p.project.name === "string" ? p.project.name : "";
    if (!had) project = null;
    else {
      const there = projectsOn(to), match = there.find((x) => same(x.name, had));
      if (match) project = match.path;
      else { asking.id = id; asking.to = to; asking.list = there; C.pop = "machine"; poke(); return; }
    }
  }
  pending[id] = { to, project };
  poke();
  if (p.busy) {
    const r = await call(from, { action: "hold", agent: bare(id), on: true });
    if (!r || !r.ok) { delete pending[id]; note((r && r.error) || "Couldn't reach " + nameOf(from) + ". The chat stays where it is."); poke(); }
    return;
  }
  await go(id);
}

/* a move that waited is taken back: what the user sent meanwhile goes on where the chat is */
async function release(id: string) {
  delete pending[id];
  await call(machineOf(id), { action: "hold", agent: bare(id), on: false });
}

/* the chat's reply is over (its "done"): a move that waited for it goes now */
export function replyOver(id: string) {
  if (id in pending) setTimeout(() => go(id), 400);
}

async function go(id: string, tries = 0) {
  const want = pending[id];
  if (!want) return;
  const from = machineOf(id), to = want.to;
  /* (asked now: once it is put away here, the page has already moved on from it) */
  const front = H.current === id;
  const out = await call(from, { action: "out", agent: bare(id) });
  if (!pending[id] || pending[id] !== want) return;   /* taken back or changed meanwhile */
  if (!out || !out.ok) {
    /* the turn is ending as this is asked: once more in a moment */
    if (out && /still working/.test(out.error || "") && tries < 30) { setTimeout(() => go(id, tries + 1), 1000); return; }
    delete pending[id]; poke();
    note((out && out.error) || "Couldn't reach " + nameOf(from) + ". The chat stays where it is.");
    return;
  }
  const inn = await call(to, { action: "in", chat: out.chat, project: want.project ? bare(want.project) : null, from: computerName(from) });
  if (!inn || !inn.ok) {
    await release(id); poke();
    note((inn && inn.error) || "Couldn't reach " + nameOf(to) + ". The chat stays where it is.");
    return;
  }
  /* the other computer has it: it is put away here (a failure leaves two copies, never none) */
  const away = await call(from, { action: "away", agent: bare(id) });
  delete pending[id];
  if (!away || !away.ok) note("The chat is on " + nameOf(to) + " now, but " + nameOf(from) + " couldn't put its copy away. You can delete that one.");
  /* (api() already gives ids of another computer's answer in the page's names) */
  const next = machineOf(inn.agent) === to ? inn.agent : withM(to, inn.agent);
  /* its events come in with the other computer's next poll: shown in front as soon as it is there */
  for (let i = 0; i < 100 && !(H.agents && H.agents.has(next)); i++) await new Promise((r) => setTimeout(r, 150));
  if (front && H.agents && H.agents.has(next)) H.select(next);
  poke();
}
