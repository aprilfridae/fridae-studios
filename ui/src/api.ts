/* The app's own server: the same call the old page makes (its `api()`, which comes here once app.js has loaded), with
   the window's token. A call about a chat or project on another computer goes to that computer through this one's
   app (remote/route.ts), and what this computer's app answered (offline, not paired, identity changed) is noted on
   that machine. */
import { back, markReply, route } from "./remote/route";

export const TOKEN = new URLSearchParams(location.search).get("t") || "";

export async function api(path: string, body?: unknown): Promise<any> {
  const to = route(path, body);
  const r = await fetch(to.url, { method: to.body === undefined ? "GET" : "POST", headers: { "X-Token": TOKEN, "Content-Type": "application/json" }, body: to.body === undefined ? undefined : JSON.stringify(to.body) });
  if (!to.m) return r.json();
  let j: any = null;
  try { j = await r.json(); } finally { markReply(to.m, r.status, j); }
  return back(to.m, to.rest, j);
}
