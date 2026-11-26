/* What the sidebar shows, worked out from the store: each project with one state mark, No project when such chats
   exist, and New project. The project in front shows its chats under it, and under a chat the experts it started,
   each a chat of its own. Every row is plain values, so a row whose values did not change is not drawn again. */
import type { Chat, Member, State } from "../store";
import { needsCount, needsStrong } from "../review/model";
import { ago } from "../util";
import { SW, bornAt, memberOf, splitExpert, swTip, thingOf } from "./team";
import { XP_IDLE, noteWhen, pageStart, shrink, xpOld, xpOpen, xpTab, xpWhen } from "./fold";

/* the part of the store the sidebar reads */
export type Slice = Pick<State, "nav" | "chats" | "projects" | "review" | "reviewDone" | "saved" | "teams" | "ais"> & { defaultAi: string };

/* the page in front is a project's tab, or one of the pages of no one project: Overview, Review and New project */
export const isPage = (v: string) => v === "newproj" || v === "all" || v === "review";
/* the user's own chats (not experts or helpers, not the researcher) */
export const isChat = (p: Chat) => !p.helper && !p.research;

/* one mark per project: how much waits for the user, and whether anything works on it */
export function projState(s: Pick<State, "review" | "reviewDone" | "chats">, path: string) {
  const r = s.review[path];
  return { n: needsCount(r, s.reviewDone), strong: needsStrong(r, s.reviewDone), working: s.chats.some((p) => p.busy && p.project === path) };
}
/* everything that waits for the user, over every project: Review's count */
export function totals(s: Pick<State, "projects" | "review" | "reviewDone" | "chats">) {
  let n = 0, k = 0, strong = false;
  for (const x of s.projects) { const st = projState(s, x.path); if (st.n) { n += st.n; k++; } strong = strong || st.strong; }
  return { n, k, strong };
}
/* the chats listed for a project: started ones, plus the one in front */
export function projectChats(s: Pick<State, "chats" | "nav">, key: string) {
  return s.chats.filter((p) => isChat(p) && p.project === key && (p.started || p.busy || p.id === s.nav.chat || (!!key && !p.started)));
}

/* an expert, from wherever it is known: its Lead's team, the Lead's conversation, the saved chats, the open ones */
type Expert = { id: string; name?: string; expert?: string; runsOn?: string; at?: number; ai?: string; plain?: string; member?: Member; pane?: Chat; when: number };
/* The experts a chat started, newest first. */
export function expertsOf(s: Slice, p: Chat): Expert[] {
  const all = new Map<string, any>();
  const put = (id: string, more: any) => { const x = all.get(id) || { id }; for (const k in more) if (more[k] !== undefined && more[k] !== null && more[k] !== "") x[k] = more[k]; all.set(id, x); };
  /* the members of its team's run, if it leads one: first, so what the chat itself knows of each wins */
  const team = s.teams[p.id];
  if (team && team.run) for (const m of team.members || []) if (m && m.id) put(m.id, { name: m.name, expert: m.role, member: m });
  for (const x of p.xps) put(x.id, { name: x.name, expert: x.expert, runsOn: x.runsOn, at: x.at });
  for (const c of s.saved.chats || []) if (c.parent === p.id) put(c.id, { name: c.name, expert: c.expert, ai: c.ai, plain: c.plain });
  for (const h of s.chats) if (h.parent === p.id) put(h.id, { name: h.name, expert: h.expert, runsOn: h.runsOn, pane: h });
  /* an open expert nothing is known of was open when the page loaded: it counts from then; one put back from a
     saved chat that the list of saved chats never named counts as old */
  for (const x of all.values()) x.when = xpWhen.get(x.id) || (x.pane && !x.pane.replay ? pageStart : 0);
  return [...all.values()].sort((a, b) => (bornAt(b.id) - bornAt(a.id)) || ((b.at || 0) - (a.at || 0)));
}
/* at work now, at work within the last half hour, the chat in front, or a member of a team's run that is going */
const xpRecent = (x: Expert, now: number, front: string | null) => !!(x.pane && x.pane.busy) || !!x.member || x.id === front || now - x.when < XP_IDLE;
/* an expert's state in a few words, and the class of its dot */
export function xpState(h: Chat): [string, string] {
  const words = h.busy ? "working · " + (h.now || "starting") : h.last === "Stopped" ? "stopped" : h.last === "Needs attention" ? "stopped with an error" : h.last === "Needs you" ? "waiting for an answer" : h.started ? "finished" : "starting";
  return [words, h.busy ? " live" : h.started && h.last !== "Stopped" && h.last !== "Needs attention" ? " ok" : ""];
}

export type ExpertRow = { t: "expert"; key: string; id: string; of: string; cls: string; dot: string; who: string; job: string; small: string; turnTip: string; tip: string; on: boolean; tab: number };
export type MoreRow = { t: "more"; key: string; id: string; text: string; open: boolean; tip: string };
export type BoxRow = { t: "box"; key: string; id: string; domId: string; cls: string; label: string; inert: boolean; rows: Row[] };
export type Row =
  | { t: "all"; key: string; on: boolean; busy: boolean }
  | { t: "review"; key: string; on: boolean; n: number; k: number; strong: boolean }
  | { t: "proj"; key: string; path: string; name: string; cls: string; n: number; strong: boolean; working: boolean; ais: string }
  | { t: "chat"; key: string; id: string; name: string; on: boolean; live: boolean; count: number; open: boolean; tip: string; ai: string }
  | { t: "none"; key: string; cls: string; busy: boolean; ais: string }
  /* a project with its chats (the one in front), in a box of its own */
  | { t: "pbox"; key: string; path: string; on: boolean; rows: Row[] }
  | { t: "add"; key: string; on: boolean }
  | ExpertRow | MoreRow | BoxRow;

export function rowsOf(s: Slice, now: number): Row[] {
  const { nav, chats, projects } = s, view = nav.view, top: Row[] = [];
  let rows = top;
  noteWhen(chats, s.saved.chats || []);
  const byId = new Map(chats.map((p) => [p.id, p]));
  const aiName = (id?: string) => { const a = s.ais.find((x) => x[0] === id); return a ? a[1] : id === "chatgpt" ? "ChatGPT" : "Claude"; };
  const chatAi = (p: Chat) => p.ai || s.defaultAi;
  const inFront = (key: string) => nav.proj === key && !isPage(view);
  /* with one of its chats in front the project's row only brightens: the chat's row carries the mark */
  const marked = (key: string) => (inFront(key) ? (view === "chat" && !!nav.chat && byId.has(nav.chat) ? " on in" : " on") : "");
  const front = view === "chat" ? nav.chat : null;

  /* One expert's row: who and what for, then its state and its AI. */
  const expert = (x: Expert, lead: Chat | null, tab: string): ExpertRow => {
    const key = "x:" + x.id, h = x.pane, [who, job] = splitExpert(x.name, x.expert), on = x.id === nav.chat && view === "chat";
    /* a member of a team's run says its state in the run (parked, being checked, merged...); while it works, what it
       is doing, as any expert does */
    const m = x.member || memberOf(s.teams, x.id), sw = m && SW[m.status || ""], doing = !!(h && h.busy && (!m || m.status === "working"));
    const state: [string, string] = sw && !doing ? [sw.word, sw.dot] : h ? xpState(h) : [x.when ? ago(x.when / 1000) : "closed", ""];
    const ai = x.runsOn || (m && m.ai ? aiName(m.ai) + (m.model ? " · " + m.model : "") : aiName(h ? chatAi(h) : x.ai || (lead ? chatAi(lead) : undefined)));
    const plain = (h && h.plain) || x.plain || "", still = !!(m && (m.status === "working" || m.status === "parked" || m.status === "verifying"));
    const turn = !!(m && m.studio_turn), thing = thingOf(projects, lead ? lead.project : nav.proj);
    return {
      t: "expert", key, id: x.id, of: lead ? lead.id : "", on, dot: state[1],
      cls: (on ? " on" : "") + (doing ? " busy" : "") + (turn ? " st-turn" : ""), who, job,
      /* once the expert has finished, its row shows the plain-English line of what it did; while it works, its state */
      small: plain && !(h && h.busy) && !still ? "In plain words: " + plain : state[0] + " · " + ai,
      turnTip: turn ? "It has the Studio turn: the only one changing the live place right now. The others carry on with their own copies." : "",
      tip: [[who, job, state[0], ai].filter(Boolean).join(" · ") + ".", sw ? swTip(m!.status, thing, m!.role) : "", turn ? "It has the Studio turn." : "", plain ? "In plain words: " + plain : "", lead ? "" : "The chat that started it is closed."].filter(Boolean).join(" "),
      tab: !lead || tab === key ? 0 : -1,
    };
  };
  /* the chats of the project in front, each with its experts under it */
  const chatsOf = (key: string) => {
    for (const p of projectChats(s, key)) {
      const xs = expertsOf(s, p), open = xs.length > 0 && xpOpen(p.id), at = xs.filter((x) => x.pane && x.pane.busy).length, on = p.id === front;
      const state = p.busy ? p.now || "Working" : p.last, kids = "k:" + p.id, many = xs.length === 1 ? "expert" : xs.length + " experts";
      rows.push({ t: "chat", key: "c:" + p.id, id: p.id, name: p.name, on, live: p.busy || at > 0, count: xs.length, open, ai: chatAi(p),
        tip: [p.name, aiName(chatAi(p)), state, xs.length ? (xs.length === 1 ? "1 expert" : many) + (at ? ", " + at + " working" : "") : ""].filter(Boolean).join(" · ") });
      if (!open && !shrink.has(kids)) continue;
      /* those at work or lately at work, then one line for the rest, which opens to them */
      const recent = xs.filter((x) => xpRecent(x, now, nav.chat)), old = xs.filter((x) => !xpRecent(x, now, nav.chat)), olds = "o:" + p.id, showOld = old.length > 0 && xpOld.has(p.id);
      const shown = recent.concat(showOld ? old : []).map((x) => "x:" + x.id), kept = xpTab.get(p.id) || "";
      const tab = shown.includes(kept) ? kept : shown.includes("x:" + nav.chat) ? "x:" + nav.chat : shown[0];
      const inside: Row[] = recent.map((x) => expert(x, p, tab));
      if (old.length) {
        inside.push({ t: "more", key: "m:" + p.id, id: p.id, open: showOld, text: old.length + (recent.length ? " earlier" : old.length === 1 ? " earlier expert" : " earlier experts"),
          tip: (old.length === 1 ? "One expert that has" : old.length + " experts that have") + " not worked for half an hour or more. Press to " + (showOld ? "hide" : "show") + (old.length === 1 ? " it." : " them.") });
        if (showOld || shrink.has(olds)) inside.push({ t: "box", key: olds, id: p.id, domId: "old-" + p.id, cls: "s-kids", label: "Earlier experts of " + p.name, inert: !showOld, rows: old.map((x) => expert(x, p, tab)) });
      }
      rows.push({ t: "box", key: kids, id: p.id, domId: "kids-" + p.id, cls: "s-kids tree", label: "Experts of " + p.name, inert: !open, rows: inside });
    }
    /* an expert that is open while the chat that started it is closed */
    for (const h of chats) if (h.helper && !byId.has(h.parent) && h.project === key) rows.push(expert({ id: h.id, name: h.name, expert: h.expert, runsOn: h.runsOn, pane: h, when: 0 }, null, ""));
  };

  top.push({ t: "all", key: "all", on: view === "all", busy: chats.some((p) => p.busy) });
  top.push({ t: "review", key: "review", on: view === "review", ...totals(s) });
  /* the AIs a project's open chats run on, in the order first used ("claude gemini") */
  const aisOf = (key: string) => [...new Set(projectChats(s, key).filter((p) => p.started || p.busy).map(chatAi))].join(" ");
  const boxed = (key: string, head: Row) => {
    const inner: Row[] = [head]; rows = inner;
    if (inFront(key)) chatsOf(key);
    rows = top;
    top.push({ t: "pbox", key: "pb:" + key, path: key, on: inFront(key), rows: inner });
  };
  for (const x of projects) boxed(x.path, { t: "proj", key: "p:" + x.path, path: x.path, name: x.name, cls: marked(x.path), ...projState(s, x.path), ais: aisOf(x.path) });
  const loose = chats.filter((p) => isChat(p) && !p.own && !p.want && (p.started || p.busy));
  if (loose.length || nav.proj === "") boxed("", { t: "none", key: "none", cls: marked(""), busy: loose.some((p) => p.busy), ais: aisOf("") });
  top.push({ t: "add", key: "add", on: view === "newproj" });
  return top;
}
