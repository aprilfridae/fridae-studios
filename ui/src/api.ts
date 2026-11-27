/* The app's own server: the same call the old page makes (its `api()`, which comes here once app.js has loaded), with
   the window's token. A call about a chat or project on another computer goes to that computer through this one's
   app (remote/route.ts). */
import { back, markOnline, route } from "./remote/route";

export const TOKEN = new URLSearchParams(location.search).get("t") || "";

export async function api(path: string, body?: unknown): Promise<any> {
  const to = route(path, body);
  const r = await fetch(to.url, { method: to.body === undefined ? "GET" : "POST", headers: { "X-Token": TOKEN, "Content-Type": "application/json" }, body: to.body === undefined ? undefined : JSON.stringify(to.body) });
  if (to.m) markOnline(to.m, r.status !== 502 && r.status !== 401);
  const j = await r.json();
  return to.m ? back(to.m, to.rest, j) : j;
}
