/* The AIs, and signing in to them: what the app's status says about each (status.ai), worded as one plain line and its
   buttons, and the presses. Settings, Connections draws these; so does the old page's model picker, through
   window.fridae.ai, so both say the same thing from the same state. */
import { store } from "./store";
import { host } from "./host";
import { quiet } from "./util";

const st = () => store.get().status || {};
const ai = () => st().ai || {};
export function aiName(id: string) { const a = store.get().ais.find((x) => x[0] === id); return a ? a[1] : id === "chatgpt" ? "ChatGPT" : "Claude"; }
export function defaultAi() { const d = ai().default; return store.get().ais.some((a) => a[0] === d) ? d : "claude"; }
/* what an AI can do, from the app's table (status.ai.caps): the page asks it instead of naming AIs */
export function caps(id: string) { return (ai().caps || {})[id] || {}; }

export type Line = { text: string; buttons: [string, string][] };
/* Your own sign-in to Claude Code, Cursor and Gemini, as the app has it from each tool (status.ai.sign_in: state "in",
   "out" or "unknown", who, and the sign-in started here). What it needs from you now, as one plain line and its
   buttons; nothing to say (signed in, not known yet, or no sign-in offered on this computer): no text. There is no
   signing out here: these are your own tools' sign-ins, which Fridae only ever asks about and starts. */
const sgError: Record<string, string> = {}, sgConfirm: Record<string, { text: string; at: number } | null> = {};
export function signInOf(id: string) { return (ai().sign_in || {})[id] || {}; }
/* the app's line that says what a new sign-in does to one that is there: it stands for a minute, for the second press */
export function signInConfirm(id: string) { const c = sgConfirm[id]; return c && Date.now() - c.at < 60000 ? c.text : ""; }
export function signInLine(id: string): Line {
  const s = signInOf(id), login = s.login || {};
  if (!s.offered) return { text: "", buttons: [] };
  /* one that waits is said first, whatever the tool says about who is signed in (it may have been started beside a
     failed message while the tool still said "signed in") */
  if (login.state === "waiting") return { text: "Sign in on the page that opened in your browser. If none opened, cancel and try again.", buttons: [["Cancel", "cancel"]] };
  if (signInConfirm(id)) return { text: signInConfirm(id), buttons: [["Sign in anyway", "sure"]] };
  /* Google holds the account back until it is verified: nothing to press here, it is done in the browser */
  if (s.verify) return { text: "Google wants this account verified before Gemini will work on it. Sign in to it at accounts.google.com and finish Google's check. Fridae looks again every few minutes.", buttons: [] };
  if (s.state === "in") return { text: "", buttons: [] };
  const why = login.message ? ": " + login.message + (/[.!?]$/.test(login.message) ? "" : ".") : ".";
  const text = sgError[id] ? sgError[id]
    : s.state !== "out" ? ""
    : login.state === "failed" ? "The sign-in did not finish" + why + " Try again."
    /* a key on this computer that Fridae keeps from this AI: the switch may be all that is missing */
    : s.kept_key ? "Not signed in. If you use an API key, turn on \"Use API keys from my environment\" in Settings, Connections."
    : "Not signed in.";
  return { text, buttons: text ? [["Sign in", "login"]] : [] };
}
/* Sign in starts the tool's own sign-in, which opens its page in your browser; the app then asks the tool who is
   signed in. The app first asks the tool again: when it does not say "nobody is signed in", the answer is a line that
   says what a new sign-in does, and only a second press ("sure") starts it. A start that fails says why, in the same
   line. `fromChat`: pressed beside a failed message, where no notice is in view, so what comes next is handed back
   to be said over the message box, and the second press is on the same button. */
export async function pressSignIn(id: string, action: string, fromChat?: boolean): Promise<string> {
  const sure = action === "sure" || !!(fromChat && signInConfirm(id));
  const body = action === "cancel" ? { ai: id, action } : { ai: id, action: "login", sure };
  const r = await quiet("/api/signin", body);
  const asked = !!(r && r.confirm);
  sgConfirm[id] = asked ? { text: r.error, at: Date.now() } : null;
  sgError[id] = action === "cancel" || asked ? "" : !r ? "Could not reach the app. Try again." : r.ok === false ? (r.error || "The sign-in could not start.") + " Try again." : "";
  const said = asked ? r.error : sgError[id] || "Sign in on the page that opened in your browser, then send your message again.";
  if (fromChat) { host.refresh().then(watchSignIns); return said; }
  await host.refresh(); watchSignIns();
  return said;
}
/* Fridae's own OpenCode sign-in: Fridae keeps OpenCode apart from yours (its own chats and settings), so it signs in
   on its own, once, with OpenCode's device sign-in: a code here, approved on opencode.ai. Free models work without.
   What it needs from you now, as one plain line and its buttons. Nothing to say (signed in, or not known yet): no text. */
let ocError = "";
export function openCodeSignIn() {
  const oc = ai().opencode || {}, login = oc.login || {}, signed: string[] = oc.signed_in || [], waiting = login.state === "waiting";
  /* signed in while work was running: Fridae's OpenCode starts again when that work has finished, with the plan's models */
  const pending = !!oc.pending && !waiting;
  const text: string = waiting ? (login.code ? "On the page that opens, sign in and enter the code " + login.code + "." : "Starting the sign-in...")
    : pending ? "Signed in. Paid models start once running work has finished."
    : signed.length ? ""
    : ocError ? ocError
    : login.state === "failed" ? "The sign-in did not finish" + (login.message ? ": " + login.message + (/[.!?]$/.test(login.message) ? "" : ".") : ".") + " Try again."
    : oc.free_only ? "Free models only. Sign in to use a paid plan." : "";
  const buttons: [string, string][] = waiting ? ((login.url ? [["Open the sign-in page", "open"]] : []) as [string, string][]).concat([["Cancel", "cancel"]]) : pending || signed.length ? [] : [["Sign in", "login"]];
  return { text, buttons, signed, pending, known: !!oc.known };
}
/* Sign in starts OpenCode's device sign-in and opens its page; the code to enter there shows in the notice. A start
   that fails says why, in the same line. */
export async function pressOpenCode(action: string) {
  const r = await quiet("/api/opencode", { action });
  /* only a sign-in that could not start: a page that did not open leaves the sign-in's own line in place */
  ocError = action !== "login" ? "" : !r ? "Could not reach the app. Try again." : r.ok === false ? (r.error || "The sign-in could not start.") + " Try again." : "";
  if (action === "login" && r && r.state === "waiting" && r.url) await quiet("/api/opencode", { action: "open" });
  await host.refresh(); watchSignIns();
}
/* what an AI needs from you now: OpenCode's own sign-in, or the tool's; null when there is nothing to say */
export function noticeOf(id: string): Line | null {
  if (!(ai().installed || {})[id]) return null;
  const s = id === "opencode" ? openCodeSignIn() : signInLine(id);
  return s.text ? { text: s.text, buttons: s.buttons } : null;
}
/* More accounts for an AI (status.ai.accounts, for Claude and for Gemini): each is a sign-in of Fridae's own for the
   same tool, in a folder of Fridae's own. A chat moves to the next one that has usage left when its account runs out,
   in the order of this list (your own account first). */
export function accountOf(id: string) { return (ai().accounts || {})[id] || {}; }
export function accountsOf(id: string): any[] { return accountOf(id).accounts || []; }
export const accError: Record<string, string> = {};
/* Add account makes the app's folder for one more account and starts the tool's sign-in for it (its page opens in
   your browser); Remove deletes that account's folder. */
export async function pressAccount(id: string, action: string, name: string) {
  const r = await quiet("/api/accounts", { ai: id, action, name });
  if (r && r.id) sgConfirm[r.id] = null;
  accError[id] = !r ? "Could not reach the app. Try again." : r.ok === false && action === "add" && !r.id ? (r.error || "The account could not be added.") + " Try again." : "";
  try { await host.refresh(); } catch (e) {}
  watchSignIns();
}
/* while a sign-in waits (OpenCode's, or one of your own tools'), the page follows it */
let poll = 0;
export function watchSignIns() {
  clearTimeout(poll);
  const waiting = ((ai().opencode || {}).login || {}).state === "waiting" || Object.values(ai().sign_in || {}).some((x: any) => x && x.login && x.login.state === "waiting");
  if (waiting) poll = setTimeout(async () => { await host.refresh(); watchSignIns(); }, 2000) as unknown as number;
}
