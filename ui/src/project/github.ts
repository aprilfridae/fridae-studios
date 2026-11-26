/* GitHub: Settings, Connections (connect with a one-time code entered on GitHub) and the project panel (Put on GitHub,
   Push now). Fridae keeps the sign-in to itself: agents never get it, and nothing is sent to GitHub unless you press
   one of these buttons. Put on GitHub and Push now run in the background: the page follows the project's last_push
   ("running", "ok", "error") and keeps the buttons greyed while one runs. */
import { api } from "../api";
import { cell, store } from "../store";

/* the project the panel shows and what the app says of it on GitHub */
export const ghProj = cell<{ path: string; info: any }>({ path: "", info: null });
let poll = 0;
const running = () => { const i = ghProj.get().info; return !!(i && i.last_push && i.last_push.state === "running"); };
/* the project whose panel is open (set by Project.tsx), "" when it is closed */
export let panelPath = "";
export function setPanelPath(p: string) { panelPath = p; }

export async function loadGithub(project?: string) {
  project = project || panelPath;
  let r: any = null;
  try { r = await api("/api/github/status" + (project ? "?project=" + encodeURIComponent(project) : "")); } catch (e) { return; }
  if (!r) return;
  if (project) ghProj.set({ path: project, info: r.project || null });
  store.set({ gh: r });
  clearTimeout(poll);
  /* while a code waits for you on GitHub, or a project is being sent there, the page follows it */
  if (r.pending || (project && running())) poll = setTimeout(() => loadGithub(project), r.pending ? 2000 : 1500) as unknown as number;
}
/* One press at a time. A press while another is on its way is simply dropped ({busy}), never shown as a failure. */
export async function ghAct(action: string, body?: Record<string, unknown>): Promise<any> {
  if (store.get().ghBusy) return { busy: true };
  store.set({ ghBusy: true });
  let r: any = null; try { r = await api("/api/github/" + action, body || {}); } catch (e) {}
  store.set({ ghBusy: false, ghNote: r ? "" : "Could not reach the app. Try again." });
  await loadGithub();
  return r;
}
