/* A Lead's team and its experts, in the words the sidebar and Overview use. (The old page's chat and team board word
   the same things with their own copies of these, until they move.) */
import type { Member, Project, Team } from "../store";

/* the member a chat is, in any Lead's run that is going */
export function memberOf(teams: Record<string, Team>, id: string): Member | null {
  if (!id) return null;
  for (const lead in teams) { const s = teams[lead]; if (s.run) { const m = (s.members || []).find((x) => x && x.id === id); if (m) return m; } }
  return null;
}
/* a member's state, in a word or two, with its dot */
export const SW: Record<string, { word: string; dot: string }> = {
  working: { word: "working", dot: " live" },
  parked: { word: "parked", dot: " park" },
  verifying: { word: "being checked", dot: " verify" },
  merged: { word: "merged", dot: " ok" },
  stopped: { word: "stopped", dot: "" },
  failed: { word: "failed", dot: " bad" },
};
/* and in one sentence, for its tooltip */
export function swTip(status: string | undefined, thing: string, role?: string) {
  return status === "working" && role === "librarian" ? "Looking it up in the knowledge base now. The answer comes on the board in a few seconds."
    : status === "working" ? "Working on its part now, in its own copy of the " + thing + "."
    : status === "parked" ? "Done for now, or waiting on someone. Parked costs nothing; a mention on the board wakes it."
    : status === "verifying" ? "Its part is being checked before it goes into the " + thing + "."
    : status === "merged" ? "Checked and added to the run's copy of the " + thing + ". It reaches your " + thing + " only with your yes."
    : status === "stopped" ? "Stopped. Its copy of the " + thing + " was let go."
    : status === "failed" ? "Its check did not pass. The board says what failed, and it is asked to fix it."
    : "";
}
/* "UI builder: Shop menu" -> ["UI builder", "Shop menu"]; an expert's name comes first */
export function splitExpert(name?: string, expert?: string): [string, string] {
  const n = String(name || ""), i = n.indexOf(":");
  if (expert) return [expert, i >= 0 && n.slice(0, i).trim() === expert ? n.slice(i + 1).trim() : n === expert ? "" : n];
  return i >= 0 ? [n.slice(0, i).trim(), n.slice(i + 1).trim()] : ["Helper", n];
}
/* game wording for a Roblox game, plain words for anything else */
export function thingOf(projects: Project[], path: string | null) {
  const pr = path ? projects.find((x) => x.path === path) : null, t = pr && (pr.type || (pr as any).kind);
  return t === "game" || t === "roblox" ? "game" : "project";
}
/* when a chat was made, from its id ("c<milliseconds>-<n>"): the order experts are listed in */
export const bornAt = (id: string) => { const m = /^c(\d+)-(\d+)/.exec(id || ""); return m ? +m[1] + +m[2] / 1e6 : 0; };
