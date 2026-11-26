/* What the sidebar keeps of its own: whose experts are shown, whose earlier experts, the lists easing open or shut,
   and when each expert was last at work. */
import { cell } from "../store";
import type { Chat } from "../store";

/* the list is drawn again when this is poked */
export const redraw = cell(0);
/* whether a chat's experts are shown under it: shown until the arrow hides them, and the choice is kept per chat */
let xpFold: Record<string, boolean> = {};
try { const f = JSON.parse(localStorage.getItem("experts-shown") || "{}"); if (f && typeof f === "object") xpFold = f; } catch (e) {}
export const xpOpen = (id: string) => xpFold[id] !== false;
export function setXpOpen(id: string, on: boolean) {
  if (xpOpen(id) === !!on) return;
  if (on) { delete xpFold[id]; grow.add("k:" + id); } else xpFold[id] = false;
  try { localStorage.setItem("experts-shown", JSON.stringify(xpFold)); } catch (e) {}
}
/* Lists that were just shown (they open with a short ease once they are in the page) and lists on their way out
   (kept in the page, inert, until their ease ends). Both by the list's key: "k:<chat>" a chat's experts, "o:<chat>"
   its earlier ones. */
export const grow = new Set<string>(), shrink = new Set<string>();
export const xpOld = new Set<string>();   /* the chats whose earlier experts are shown */
/* Of a chat's experts one row takes the Tab key (the one the keyboard was last on, else the one in front, else the
   first); the arrow keys go through the rest. chat id -> row key */
export const xpTab = new Map<string, string>();
/* the element of a list, by its key */
export const boxOf = (key: string) => document.getElementById((key[0] === "k" ? "kids-" : "old-") + key.slice(2));

/* When each expert was last at work (ms). A saved one: when its chat was last saved, as first heard (opening an old
   expert to read it, and closing it again, saves it anew, and that is not work). An open one: its last start or end
   of a turn seen by the page. An expert idle for longer than XP_IDLE is listed under "earlier", out of the way. */
export const xpWhen = new Map<string, number>(), XP_IDLE = 30 * 60 * 1000, pageStart = Date.now();
export function noteWhen(chats: Chat[], saved: any[]) {
  for (const c of chats) if (c.when) xpWhen.set(c.id, c.when);
  for (const c of saved) if (c.parent && !xpWhen.has(c.id) && c.updated) xpWhen.set(c.id, c.updated * 1000);
}
