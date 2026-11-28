/* The chat's own store: every chat's conversation as a list of rows (what the old page kept as DOM in a column per
   chat), and the agent events that build it. A chat (the old page's `pane`, one per agent, still made and listed by
   ui/index.html because the sidebar and the Overview read it) keeps its conversation in `p.tl`.

   A row is one thing in the column: a message of yours, a line of the app's, a turn of the agent's (its steps, its
   words, its cards: the turn's `parts`). The list on screen draws only the rows in view (Timeline.tsx), and a row is
   drawn again only when `touch(row)` says it changed, so an agent at work repaints its own turn and nothing else.
   What the bar over the chat and the message box read is not kept here twice: they read the page's own state through
   `H` and are drawn again on `poke()`. */
import { useEffect, useReducer } from "preact/hooks";
import { H } from "./host";
import { cap } from "./md";
import { C, modelsFor } from "./compose";
/* a Go model's name says who makes it ("GLM · GLM 5.3"): the model's own name says it already */
const ocName = (name: string) => { const [g, r] = name.split(" · "); return r && (r.startsWith(g) || g === "Other") ? r : name; };
import { replyOver } from "../remote/move";
import { accountWord } from "./AccountChip";
import { foldAppTools } from "./appTools";

export type Part = { id: number; k: string; v: number; born: number; [more: string]: any };
export type Row = { id: number; k: string; v: number; born: number; on?: () => void; h?: number; [more: string]: any };
export type Timeline = {
  id: number; rows: Row[]; subs: Set<() => void>;
  follow: boolean;                              /* the reader is at the end, and stays there as it grows */
  anchor: { row: Row; dy: number } | null;      /* else: the row at the top of what is seen, and how far down it sits */
};

let ids = 0;
const mk = (k: string, more?: Record<string, any>) => Object.assign({ id: ++ids, k, v: 0, born: performance.now() }, more) as Row;
export function newTl(): Timeline { return { id: ++ids, rows: [], subs: new Set(), follow: true, anchor: null }; }
export const tlOf = (p: any): Timeline => p.tl || (p.tl = newTl());
const changed = (tl: Timeline) => tl.subs.forEach((f) => f());
/* this row is drawn again (and only it) */
export function touch(row: Row | null | undefined) { if (!row) return; row.v++; if (row.on) row.on(); }
function put(tl: Timeline, row: Row) { tl.rows.push(row); changed(tl); return row; }
export function drop(tl: Timeline, row: Row | null | undefined) { const i = row ? tl.rows.indexOf(row) : -1; if (i >= 0) { tl.rows.splice(i, 1); changed(tl); } }
function toEnd(tl: Timeline, row: Row) { const i = tl.rows.indexOf(row); if (i === tl.rows.length - 1 && i >= 0) return; if (i >= 0) tl.rows.splice(i, 1); tl.rows.push(row); changed(tl); }
/* the reader is taken to the end of this conversation */
export function toLatest(tl: Timeline) { tl.follow = true; tl.anchor = null; changed(tl); }

/* Things fade in once, as they arrive. A chat put back from its log, or shown again, is simply there. */
let calmAt = 0;
export function calm() { calmAt = performance.now(); }
export const fresh = (born: number) => born > calmAt && performance.now() - born < 320;

/* the bar over the chat, the message box and the bar over the tabs are drawn again */
const ticks = new Set<() => void>();
export function poke() { ticks.forEach((f) => f()); }
export function useTick() {
  const again = useReducer((n: number) => n + 1, 0)[1] as unknown as () => void;
  useEffect(() => { ticks.add(again); return () => { ticks.delete(again); }; }, []);
}

/* ---------- a chat is made: its welcome and its start card ---------- */
export function startState(p: any, row: Row) {
  p.start = { step: p.project ? "ready" : "folder", name: "", parent: "", git: true, kind: "plain", msg: "", focus: false, paint: () => touch(row) };
}
export function opened(p: any) {
  const row = mk("hello", { p, lede: "" });
  startState(p, row); tlOf(p).rows.push(row);
}
/* A project with no chat yet (or New chat): an empty page and the message box. No chat is made until the first
   message is sent or a start button is pressed; the card's buttons make the chat first (`draft`). */
export const draft: { tl: Timeline | null; p: any } = { tl: null, p: null };
export function showDraft(key: string, pr: any) {
  const proxy: any = { draft: true, key, id: null, project: pr || null, seed: null };
  const tl = newTl(), row = mk("hello", { p: proxy, lede: pr ? "A new chat for " + pr.name + "." : "" });
  startState(proxy, row);
  if (pr) proxy.start.step = H.startStep(pr);
  tl.rows.push(row); draft.tl = tl; draft.p = proxy; calm(); poke();
}
/* the conversation in front: a chat's, the empty page's, or none */
export function frontTl(): Timeline | null { const p = H.agents.get(H.current); return p ? tlOf(p) : H.draftOn ? draft.tl : null; }

/* ---------- a turn and what goes into it ---------- */
function newTurn(p: any) {
  /* which account the reply runs on, named only for a chat whose AI has more than one (the app says "account" then) */
  p.turn = put(tlOf(p), mk("turn", { p, label: p.model || H.aiName(H.chatAi(p)), acct: p.account ? accountWord(p.account) : "", parts: [] as Part[], steps: null, undo: null }));
  return p.turn as Row;
}
const turnOf = (p: any): Row => p.turn || newTurn(p);
const part = (k: string, more?: Record<string, any>) => mk(k, more) as Part;
/* into the chat's turn; whatever comes after it starts a new list of steps */
function add(p: any, pt: Part) { const t = turnOf(p); t.parts.push(pt); t.steps = null; touch(t); return pt; }
export function partChanged(row: Row | null | undefined, pt: Part) { pt.v++; touch(row); }
function steps(p: any): Part {
  const t = turnOf(p);
  if (!t.steps || t.parts[t.parts.length - 1] !== t.steps) { t.steps = part("steps", { list: [] as { t: string; born: number }[], live: false, done: false, open: false }); t.parts.push(t.steps); }
  return t.steps;
}
function calmSteps(t: Row | null) { if (!t) return; for (const s of t.parts) if (s.k === "steps" && s.live) { s.live = false; s.v++; } }
function settle(p: any) {
  const t = p.turn as Row | null; if (!t) return;
  for (const s of t.parts) if (s.k === "steps") { s.live = false; s.done = true; s.v++; }
  touch(t);
}

/* ---------- experts: the line for expert `hid` in chat `p`'s conversation, put where the chat's log says the expert
   was started. The experts a chat started are kept with it (p.xps), in order: the sidebar lists them. ---------- */
function xpLink(p: any, hid: string, name: string, expert: string, runsOn: string) {
  let x = p.xps[hid];
  if (!x) x = p.xps[hid] = { id: hid, name: "", expert: "", runsOn: "", at: Object.keys(p.xps).length, row: null, part: null };
  if (name) x.name = name;
  if (expert) x.expert = expert;
  if (runsOn) x.runsOn = runsOn;   /* "Cursor · Auto": only when it runs on another AI or model than the chat that started it */
  return x;
}
/* an expert's state changed: the dot on its line in the chat that started it */
export function paintExpert(h: any) {
  const of = H.agents.get(h.parent), x = of && of.xps[h.id];
  if (x && x.row) { if (x.part) x.part.v++; touch(x.row); }
}

/* ---------- cards in an expert's chat that ask to allow a program, or to run a command once, are live only while
   the expert is stopped on them: in a log put back from a saved chat, or once the expert has gone on, has stopped or
   has handed its result back, the card stays as what was asked, without its buttons. ---------- */
function approval(p: any, pt: Part) {
  if (!H.isHelper(p)) return pt;
  pt.approval = true; (p.approvals = p.approvals || []).push(pt);
  if (p.replay || p.handedBack) pt.shut = true;
  return pt;
}
function pastApprovals(p: any) {
  if (!H.isHelper(p) || !p.approvals) return;
  for (const pt of p.approvals) if (!pt.shut && !pt.said) { pt.shut = true; pt.v++; touch(pt.row); }
}

/* ---------- the night: an overnight run's checkpoint, taken back from the chat itself ---------- */
/* the night began with something saved to go back to: the project's files (a commit), Studio's steps, or both */
export function nightSaved(p: any) { return !!(p && p.night && (p.night.commit || p.night.studio)); }
/* This chat's overnight run can be undone now: a checkpoint was saved, it is not undone, the chat is not at work, and
   no undo of it is on its way (a studio run has its own undo, in the run's menu). */
export function nightOpen(p: any) { return nightSaved(p) && !p.nightUndone && !p.nightUndoing && !p.busy && !p.studio && !p.night.run; }
/* The chat's own line where the night began. With a checkpoint it says where the undo is; with none it says so. */
export function paintNightTag(p: any) {
  const r = p.nightRow as Row | null; if (!r) return;
  const run = !!(p.studio || (p.night && p.night.run)), game = !!p.project, rbx = H.isGameProj(p.project);
  const text = !nightSaved(p) ? (game ? (rbx ? "No checkpoint: no git in the project and Studio not connected" : "No checkpoint: the project has no git") : "No checkpoint: Studio not connected")
    : "Checkpoint saved" + (run ? " · Undo this run is in the run's menu, at the top of this chat" : "");
  const btn = !(run || !nightOpen(p));
  if (text !== r.text || btn !== r.btn) { r.text = text; r.btn = btn; touch(r); }
}
/* a night begins: its line goes into the chat, and anything left open about the night before it goes */
function nightTag(p: any) {
  const tl = tlOf(p);
  if (p.nightRow) { p.nightRow.old = true; touch(p.nightRow); }
  for (const r of tl.rows.filter((x) => x.k === "nightask")) drop(tl, r);
  p.nightRow = put(tl, mk("night", { p, text: "", btn: false, old: false }));
  paintNightTag(p);
}
/* the line's own link: the card that asks before undoing */
export function askNight(p: any) {
  if (!nightOpen(p)) { paintNightTag(p); return; }
  const tl = tlOf(p);
  for (const r of tl.rows.filter((x) => x.k === "nightask")) drop(tl, r);
  put(tl, mk("nightask", { p, busy: "", said: "", files: false }));
  if (p.id === H.current) toLatest(tl);
}

/* "Waiting for you in Review": one line at the end of a Lead's chat while its run waits at a gate */
function studioWaitLine(p: any, on: boolean) {
  p.studioWaiting = on;
  if (!on) { if (p.waitRow) drop(tlOf(p), p.waitRow); return; }
  if (!p.waitRow) p.waitRow = mk("wait", { p });
  toEnd(tlOf(p), p.waitRow); p.last = "Waiting for you";
}
/* Messages sent while the chat was working: shown under the conversation until they go. */
function paintQueue(p: any, items: any[], paused: boolean, kept: boolean) {
  if (!p.q) p.q = mk("queue", { p, items: [], paused: false, kept: false });
  p.q.items = items; p.q.paused = paused; p.q.kept = kept; p.q.born = performance.now();
  toEnd(tlOf(p), p.q); touch(p.q);
}
/* one line in the chat's turn that leads somewhere else: a promo that is ready, a thing filed for review */
export function leadLine(p: any, text: string, go: string, press: () => void) {
  add(p, part("line", { text, go, press }));
}

/* ---------- an agent's event: the chat's own state and its conversation. What the rest of the page does about it
   (the sidebar, the Overview, the list of saved chats) stays in ui/index.html's onEvent, which calls this first.
   Returns "auto" for a message the app sent itself (the page then repaints the sidebar and nothing else). ---------- */
export function event(p: any, e: any): string | void {
  const tl = tlOf(p), live = e.id > H.bootMark;
  if (C.startingRun && e.agent === C.startingRun && ["user", "busy", "error", "done"].includes(e.kind)) C.startingRun = "";
  if (e.kind === "agent_new") {
    if (e.restored) p.replay = true;   /* put back from a saved chat: what follows is its log, until it works again */
    p.name = e.name; if (e.ai) p.ai = e.ai; if (e.role) p.role = e.role; if (e.parent) p.parent = e.parent; if (e.expert) p.expert = e.expert; if (e.team) p.team = e.team; if (e.runs_on) p.runsOn = e.runs_on;
    /* an expert's chat (and the researcher's, kept out of sight) has no welcome */
    if (H.isHelper(p) || p.role === "research") { drop(tl, tl.rows.find((r) => r.k === "hello")); p.start = null; }
  } else if (e.kind === "helper_started") {
    /* this chat started an expert here: one line at this point of the conversation leads to the expert's own chat */
    const x = xpLink(p, e.helper, e.name, e.expert, e.runs_on);
    if (x.row) { if (x.part) { x.row.parts = x.row.parts.filter((q: Part) => q !== x.part); touch(x.row); } else drop(tl, x.row); }
    if (p.turn) { x.part = add(p, part("xp", { x })); x.row = p.turn; }
    else { x.part = null; x.row = put(tl, mk("xp", { p, x })); }
  } else if (e.kind === "ai") {
    p.ai = e.ai;
    if (e.from) { put(tl, mk("tag", { cls: "tag ai-tag", text: "Switched from " + H.aiName(e.from) + " to " + H.aiName(e.ai) + ". It gets the chat so far with your next message." })); p.turn = null; }
  } else if (e.kind === "project") {
    /* the event's own `id` is its number on the event list: the project's id comes as projectId */
    p.project = e; p.projectId = e.projectId || ""; H.tied(p);
    if (p.start) { p.start.step = H.startStep(e) === "ready" ? "ready" : (p.start.step === "folder" || p.start.step === "new" ? "kind" : p.start.step); p.start.paint(); }
  } else if (e.kind === "model") {
    p.model = e.name;
    if (p.turn) { p.turn.label = e.name; touch(p.turn); }
  } else if (e.kind === "texrequest") {
    const pt = add(p, part("tex", { req: e.req, name: e.name || "texture", prompt: e.prompt, said: "", copied: false }));
    pt.row = p.turn; p.asks = p.asks || {}; p.asks[e.req] = pt; p.now = "Waiting for your image";
  } else if (e.kind === "texdone") {
    const pt = p.asks && p.asks[e.req];
    if (pt) { pt.said = e.ok ? "Image received" : "Texture skipped"; partChanged(pt.row, pt); delete p.asks[e.req]; }
  } else if (e.kind === "night") {
    p.night = e; p.nightUndone = false; nightTag(p);
  } else if (e.kind === "night_undone") {
    p.nightUndone = true; put(tl, mk("card", { title: "The night was undone", text: e.text || "" }));
  } else if (e.kind === "run_undone") {
    put(tl, mk("card", { title: "The run was undone", text: e.text || "" }));
  } else if (e.kind === "studio_waiting") {
    studioWaitLine(p, !!e.value);
  } else if (e.kind === "renamed") {
    p.name = e.name;
  } else if (e.kind === "account") {
    /* which of its AI's accounts this chat runs on (the account chip, and each reply's label, while the AI has more
       than one). A pick by hand is in use from here (or came to nothing): the chip no longer shows it as waiting */
    p.account = e.account; p.accountPick = 0;
    if (e.picked) { put(tl, mk("tag", { cls: "tag ai-tag", text: "On " + (e.account >= 2 ? H.aiName(H.chatAi(p)) + " account " + e.account : "your own " + H.aiName(H.chatAi(p)) + " account") + " from here, as you picked." })); p.turn = null; }
  } else if (e.kind === "account_pick") {
    /* an account picked in the chip, for the chat's next message */
    p.accountPick = e.account;
  } else if (e.kind === "moved") {
    /* switched to another of the user's computers (ui/src/remote/move.ts) and its AI isn't set up there: the line
       that says which AI carries it on */
    put(tl, mk("tag", { cls: "tag ai-tag", text: e.text || "" })); p.turn = null;
  } else if (e.kind === "rollover") {
    /* one of the AI's two accounts ran out of usage and the chat carries on with the other: the app's one line about
       it. The same message goes again there, so what follows is a turn of its own */
    put(tl, mk("tag", { cls: "tag ai-tag", text: e.text || "" })); p.turn = null;
  } else if (e.kind === "queued") {
    paintQueue(p, e.items || [], !!e.paused, !!e.kept);
  } else if (e.kind === "user" && e.steered) {
    /* written to a Lead while it works: given to it at its next step, so the turn it is on carries on */
    put(tl, mk("you", { text: e.text })); put(tl, mk("tag", { text: "Given to it at its next step" }));
    p.turn = null;
  } else if (e.kind === "user") {
    drop(tl, tl.rows.find((r) => r.k === "hello"));
    p.turn = null; p.started = true; p.autopilot = !!e.autopilot; p.overnight = e.overnight && e.until ? e.until : 0;
    p.studio = !!e.studio; studioWaitLine(p, !!e.studio_waiting);
    p.carried = e.auto ? (p.carried || 0) + 1 : 0;
    if (e.auto) {
      const left = p.overnight ? Math.max(0, p.overnight - Date.now() / 1000) : 0;
      put(tl, mk("tag", { text: p.overnight ? (p.studio ? "Studio" : "Overnight") + " · turn " + p.carried + " · " + Math.floor(left / 3600) + "h " + Math.floor((left % 3600) / 60) + "m left" : "Full auto · carrying on by itself (" + p.carried + " of 12)" }));
      if (e.agent === H.current) poke();
      return "auto";
    }
    put(tl, mk("you", { text: e.text, images: e.images && e.images.length ? e.images : null, files: e.files && e.files.length ? e.files : null }));
    /* an OpenCode model by its name, as the picker shows it ("GLM 5.3 Flash", "OpenCode Zen · big-pickle") */
    const ran = e.model && H.chatAi(p) === "opencode" ? ocName((modelsFor("opencode").find((m) => m[0] === e.model) || [])[1] || e.model) : e.model;
    const tags = [ran, e.selftest ? "self-test on" : "", e.gemini ? "gemini textures" : "", e.full && !e.studio ? "full auto" : "", e.studio ? "Studio" : e.overnight ? "overnight, 24 hours" : e.autopilot && !e.full ? "autopilot" : ""].filter(Boolean).join("  ·  ");
    if (tags) put(tl, mk("tag", { text: cap(tags) }));
    if (e.fallback) put(tl, mk("tag", { text: e.fallback }));
    /* which model ran and why, when that is not what was set (from the app: one line, two short sentences at most) */
    if (e.model_note) put(tl, mk("tag", { text: e.model_note }));
  } else if (e.kind === "appcard") {
    /* the app toolbox asks before an AI changes something of the app's (app-rs/src/app_mcp/card.rs): a card that
       holds the AI's call until it is answered here. In a chat put back from its log nobody is waiting on it any more */
    const pt = add(p, part("appcard", { card: e.card, title: e.title || "", text: e.text || "", allow: e.allow || "Allow", said: "", sent: false, shut: !!p.replay, last: p.last, now: p.now }));
    pt.row = p.turn; p.asks = p.asks || {}; p.asks[e.card] = pt;
    if (!pt.shut) { p.now = "Waiting for you"; p.last = "Needs you"; }
  } else if (e.kind === "appcard_done") {
    const pt = p.asks && p.asks[e.card];
    if (pt) {
      pt.said = e.outcome || "cancelled"; partChanged(pt.row, pt); delete p.asks[e.card];
      if (p.last === "Needs you") p.last = pt.last;
      if (p.now === "Waiting for you") p.now = pt.now;
    }
  } else if (e.kind === "apptool") {
    /* a call to the app toolbox: one line, put where it began and changed in place as it goes on */
    p.appCalls = p.appCalls || {};
    const was = p.appCalls[e.call], more = { status: e.status, label: e.label || "", summary: e.summary || null, links: Array.isArray(e.links) ? e.links : [] };
    if (was && was.row && was.row.parts.includes(was)) { Object.assign(was, more); partChanged(was.row, was); }
    else { const pt = add(p, part("apptool", { call: e.call, ...more })); pt.row = p.turn; p.appCalls[e.call] = pt; }
    if (e.status === "running") p.now = e.label || p.now;
  } else if (e.kind === "tool") {
    const s = steps(p), t = p.turn as Row;
    s.list.push({ t: e.label, born: performance.now() }); s.live = true; s.v++; touch(t);
    p.now = e.label;
  } else if (e.kind === "text") {
    /* The agent's last line "STATUS: <WORD>" is for the app, not the reader: it is never shown as written (the saved
       chat keeps it). CONTINUE, DONE and NEEDS YOU are said in plain words; WAITING alone shows as the "Waiting for
       you" line; any other word, or a reason after one, is shown as it is, so nothing the agent said is lost. The
       line must be a line of its own: a sentence that ends "...the build status: done" is left as it was written. */
    const said = /(?:^|\n)[ \t]*STATUS:[ \t]*(CONTINUE|DONE|NEEDS YOU[^\n]*)\s*$/i.exec(e.text) || /(?:^|\n)[ \t]*STATUS:[ \t]*([A-Z][A-Z _-]*(?::[^\n]*)?)\.?\s*$/.exec(e.text);
    const text = said ? e.text.slice(0, said.index).trimEnd() : e.text;
    let plain = "";
    if (said) {
      const st = said[1].trim(), up = st.toUpperCase(), colon = st.indexOf(":");
      const word = (colon < 0 ? st : st.slice(0, colon)).trim().toUpperCase(), why = colon < 0 ? "" : st.slice(colon + 1).trim();
      const needs = /^NEEDS YOU/.test(up);
      plain = needs ? st.replace(/^needs you:?\s*/i, "Needs you: ")
        : (word === "CONTINUE" ? "Autopilot · more to do" : word === "DONE" ? "Finished and checked" : word === "WAITING" ? (why ? "Waiting" : "") : cap(word.toLowerCase())) + (why ? ": " + why : "");
      p.last = needs ? "Needs you" : word === "DONE" ? "Finished" : word === "CONTINUE" ? "Carrying on" : word === "WAITING" ? "Waiting for you" : p.last;
    }
    /* a message that was only the STATUS line, with nothing to say in its place, starts no turn */
    if (text || plain) {
      if (text) add(p, part("prose", { text }));
      if (plain) { const t = turnOf(p); t.parts.push(part("tag", { text: plain })); touch(t); }
    }
    if (p.turn) { calmSteps(p.turn); touch(p.turn); }
    p.now = "Writing";
  } else if (e.kind === "plain") {
    /* the app's plain-English line of what this expert did, for the user: shown in the expert's chat and on its row */
    p.plain = e.text || "";
    if (e.text) add(p, part("plain", { text: e.text }));
  } else if (e.kind === "images") {
    add(p, part("shots", { urls: e.urls || [] }));
  } else if (e.kind === "blocked" || e.kind === "refused") {
    const commands = e.commands || [], names = commands.map((c: any) => c.name);
    const pt = add(p, part(e.kind, { commands, said: "", shut: false }));
    pt.row = p.turn;
    if (e.kind === "blocked" && names.every((n: string) => H.allowed().includes(n))) pt.said = "Allowed: " + names.join(", ");
    approval(p, pt);
    p.last = "Needs you";
  } else if (e.kind === "error") {
    /* the turn failed because nobody is signed in to this chat's AI: the same Sign in as in the picker, beside the
       line (not on lines from before the app was opened: they are history) */
    add(p, part("err", { text: e.text, signIn: e.sign_in && live ? e.sign_in : "" }));
    p.last = e.text === "Stopped." ? "Stopped" : "Needs attention";
    pastApprovals(p);
  } else if (e.kind === "handed_back") {
    /* a helper's result went back to the chat that started it: it is finished, whatever it last asked for */
    p.last = p.last === "Stopped" ? "Stopped" : "Done";
    p.handedBack = true; pastApprovals(p);
  } else if (e.kind === "busy") {
    p.busy = !!e.value; p.now = "Starting"; if (p.busy) { p.last = "Done"; p.since = Date.now(); }
    if (p.busy) { p.replay = false; p.handedBack = false; pastApprovals(p); }   /* it has gone on: what it asked before is history */
  } else if (e.kind === "done") {
    if (live) replyOver(p.id);   /* a move to another computer that waited for this reply goes now */
    const t = p.turn as Row | null, st = H.status;
    if (t && !H.isHelper(p) && live && st.plugin && st.plugin.connected && H.isGameProj(p.project) && t.parts.some((x: Part) => x.k === "steps" && x.list.length)) {
      /* one turn can be undone in Studio: the newest; an offer nobody took goes when the next one comes */
      if (p.undoRow && p.undoRow.undo && !p.undoRow.undo.said) { p.undoRow.undo = null; touch(p.undoRow); }
      t.undo = { text: "Undo this turn in Studio", said: false }; p.undoRow = t;
    }
    if (t) { const folded = foldAppTools(t.parts, part); if (folded.length !== t.parts.length) { t.parts = folded; t.steps = null; } }
    settle(p); p.busy = false; p.now = ""; p.turn = null; p.appCalls = {};
  }
  if (p.q && p.q.items.length && tl.rows[tl.rows.length - 1] !== p.q) toEnd(tl, p.q);   /* waiting messages stay last */
  /* an expert: the dot on its line in the chat that started it */
  if (H.isHelper(p)) paintExpert(p);
  if (p.nightRow) paintNightTag(p);   /* where its undo is depends on its project, and on whether it is at work */
  if (e.agent === H.current) { poke(); if (e.kind === "user" || e.kind === "queued") toLatest(tl); }
}
/* an expert's line stays in the chat that started it when the expert's chat is closed: its dot goes plain */
export function closed(p: any) { if (H.isHelper(p)) paintExpert(p); }
