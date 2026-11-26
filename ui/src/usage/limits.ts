/* Plan limits, worked out from the app's status: one entry per signed-in account.
   status.limits.accounts is one entry per account: {ai, account, label, checked, error, windows: [{id, label, used (0
   to 1), resets (when it starts again, in seconds; 0 when not known), length (the window's length in seconds)}]}. An
   older app sends only Claude's windows, as {claude: {<window>: {used, resets, seen}}} (or the windows alone): they
   are shown as one account, so the page works with either. How much is left, the even-pace line and the pace mark
   follow T3 Code's usage limits (MIT; see THIRD_PARTY.md). */
import { ago } from "../util";
import { caps } from "../ai";
import type { Ai } from "../store";
export type Win = { id: string; label: string; used: number; resets: number; length: number };
export type Account = { ai: string; account: string; label: string; checked: number; error: string; windows: Win[] };
export function limitName(key: any) {
  if (key === "five_hour") return "Session";
  if (key === "seven_day") return "Weekly";
  const m = String(key).match(/^seven_day_(.+)$/);
  const words = (m ? m[1] : String(key)).split("_").map((w) => w.charAt(0).toUpperCase() + w.slice(1)).join(" ");
  return m ? "Weekly · " + words : words;
}
export function limitAccounts(status: any): Account[] {
  const L = (status && status.limits) || {};
  const one = (w, id) => ({ id: String(w.id || id || ""), label: String(w.label || limitName(w.id || id || "")), used: Math.max(0, Math.min(1, Number(w.used) || 0)), resets: Number(w.resets) || 0, length: Number(w.length) || 0 });
  if (Array.isArray(L.accounts)) return L.accounts.filter((a) => a && typeof a === "object").map((a) => ({ ai: String(a.ai || ""), account: String(a.account || ""), label: String(a.label || a.ai || "Account"), checked: Number(a.checked) || 0, error: a.error ? String(a.error) : "", windows: (Array.isArray(a.windows) ? a.windows : []).filter((w) => w && typeof w === "object").map((w) => one(w)) }));
  const old = L.claude && typeof L.claude === "object" ? L.claude : L;
  const keys = Object.keys(old).filter((k) => old[k] && typeof old[k] === "object" && typeof old[k].used === "number");
  if (!keys.length) return [];
  const order = (k) => (k === "five_hour" ? 0 : k === "seven_day" ? 1 : 2);
  keys.sort((a, b) => order(a) - order(b) || a.localeCompare(b));
  const length = (k) => (k === "five_hour" ? 18000 : /^seven_day/.test(k) ? 604800 : 0);
  return [{ ai: "claude", account: "", label: "Claude", checked: Math.max(0, ...keys.map((k) => Number(old[k].seen) || 0)), error: "", windows: keys.map((k) => one({ id: k, used: old[k].used, resets: old[k].resets, length: length(k) })) }];
}
export const remainingPercent = (w: Win) => Math.round(100 - Math.max(0, Math.min(100, w.used * 100)));
/* how much of the window has gone by (0 to 1); not known without its end or its length */
export function elapsedShare(w: Win, now: number) {
  if (!w.resets || !(w.length > 0)) return null;
  return Math.max(0, Math.min(1, (w.length - (w.resets - now)) / w.length));
}
/* the bar shows what is left, so even pace is the share of the window still to come */
export function paceRemainingPercent(w: Win, now: number) { const e = elapsedShare(w, now); return e === null ? null : Math.round((1 - e) * 100); }
export function paceOf(w: Win, now: number) {
  const e = elapsedShare(w, now); if (e === null) return null;
  const gap = w.used * 100 - e * 100;
  return gap > 5 ? "ahead" : gap < -5 ? "under" : "on";
}
export function formatDuration(secs: any) {
  const left = Math.max(0, secs), days = Math.floor(left / 86400), hours = Math.floor((left % 86400) / 3600), minutes = Math.floor((left % 3600) / 60);
  return days > 0 ? days + "d " + hours + "h" : hours > 0 ? hours + "h " + minutes + "m" : minutes + "m";
}
export function formatResetsIn(w: Win, now: number) {
  if (!w.resets) return "";
  if (w.resets <= now) return "resets now";
  if (w.resets - now < 60) return "resets in a moment";
  return "resets in " + formatDuration(w.resets - now);
}
export function formatResetsAt(w: Win, now: number) {
  if (!w.resets) return "";
  const d = new Date(w.resets * 1000), t = new Date(now * 1000), time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const days = Math.round((new Date(d.getFullYear(), d.getMonth(), d.getDate()) - new Date(t.getFullYear(), t.getMonth(), t.getDate())) / 86400000);
  return days <= 0 ? time : days === 1 ? "tomorrow at " + time : d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" }) + " " + time;
}
/* the pace marks: [what it says, the icon's path] */
export const PACE: Record<string, [string, string]> = {
  ahead: ["Ahead of pace: using it up faster than the time goes by", "M1.5 10.5l4-4 2.5 2.5 5-5|M9.5 4H13v3.5"],
  on: ["On pace with the time left", "M2.5 11a5.5 5.5 0 1 1 11 0|M8 11l2.5-3.5"],
  under: ["Under pace: room left for the rest of the time", "M1.5 4.5l4 4 2.5-2.5 5 5|M9.5 11H13V7.5"],
};
/* the AIs whose tools do not report their limits: one line for all of them (those this computer offers) */
export function quietLimits(accounts: Account[], ais: Ai[]) {
  const shown = new Set(accounts.map((a) => (a.ai === "codex" ? "chatgpt" : a.ai)));
  const quiet = ais.filter(([id]) => caps(id).limits === false && !shown.has(id)).map((a) => a[1]);
  return quiet.length ? (quiet.length > 1 ? quiet.slice(0, -1).join(", ") + " and " : "") + quiet[quiet.length - 1] + (quiet.length > 1 ? " do not report their limits." : " does not report its limits.") : "";
}
export const checkedLine = (a: Account) => (a.checked ? "Checked " + ago(a.checked) : "");
