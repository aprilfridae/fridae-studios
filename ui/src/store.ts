/* What the page knows, in one place, while the old page (ui/index.html) and the new screens live side by side.
   The old page writes here where it already receives data; a screen reads with useStore and is painted again
   when what it reads has changed. */
import { useEffect, useReducer, useRef } from "preact/hooks";

/* `machine`: the paired computer it is on (ui/src/remote); its `path` is then "<machine>~<its path there>" */
export type Project = { path: string; name: string; type?: string; kind?: string; bible?: boolean; kind_set?: boolean; machine?: string };
/* a computer this one is paired with (GET /api/remote/machines); `online` as the last call to it went */
export type Machine = { id: string; name: string; addr: string; port: number; fp: string; online: boolean; last_seen?: number };
/* GET /api/projects as a whole: where new projects go and whether git is there, beside the list */
export type ProjReply = { projects: Project[]; default_parent?: string; default_parent_plain?: string; git?: boolean };
export type ReviewItem = { id: string; title?: string; summary?: string; plain?: string; status: string; blocking?: boolean; created?: number; media?: string[] };
export type NeedCard = { id: string; title?: string; note?: string };
/* a project's GET /api/review reply, as far as Review uses it */
export type ReviewReply = { project: string; key: string; items?: ReviewItem[]; needs?: NeedCard[]; via?: string };
/* something answered on the page: it stays as one line (`said`) until `gone` */
export type Answered = { at: number; gone?: boolean; said?: string };
export type Light = { list: string[]; at: number; from: HTMLElement | null };

export type State = {
  projects: Project[];
  projReply: ProjReply;                      /* the same list with the rest of its reply (New project reads where and git) */
  review: Record<string, ReviewReply>;       /* project path -> its review reply */
  reviewDone: Record<string, Answered>;      /* item or card id -> the answer made here */
  reviewOrder: string[];                     /* the tabs' order, set when the page opens */
  reviewTab: string;                         /* the project whose things are shown */
  reviewJump: { path: string; id?: string; n: number } | null;   /* "go to this project (and this thing)" */
  light: Light | null;                       /* the picture shown large */
  /* what the old page already fetches or holds, for Settings and Usage */
  status: any;                               /* GET /api/status, as the page's poll last had it */
  polls: number;                             /* how many times the status has been asked for */
  ais: Ai[];                                 /* the AIs this computer offers */
  curProj: string;                           /* the project in front ("" for none) */
  settings: { open: boolean; page: string; n: number };   /* the Settings window: shown, its page, and a count of its openings */
  usageOpen: boolean;                        /* the Usage window is shown */
  usage: UsageToday | null;                  /* today's tokens (usage/usage.ts) */
  gh: any;                                   /* GET /api/github/status; null until it has been asked */
  ghNote: string; ghBusy: boolean;           /* GitHub's row: a press that failed, a press on its way */
  engine: EngineRow | null;                  /* the promo engine's row (promos/engine.ts) */
  saved: { chats: any[]; deleted: any[] };   /* GET /api/history */
  setupOut: string;                          /* what Run a full check printed */
  scale: number; sideW: number;              /* text size (%) and the sidebar's width (px) */
  theme: { id: string; mode: string };       /* the colour theme in use */
  /* what the sidebar and Overview show: written by the old page (its `sideToStore`) until the chat has moved too */
  nav: Nav;                                  /* the page, the project and the chat in front */
  chats: Chat[];                             /* every open chat, expert and helper; an unchanged one is the same object */
  steps: Record<string, Steps>;              /* chat id -> the steps it has taken (Overview's graph) */
  teams: Record<string, Team>;               /* a Lead's chat id -> its team's run and members (GET /api/swarm) */
  notConnected: string[] | null;             /* what the chat in front needs that is not connected (the dot at the bottom left); null until known */
  machines: Machine[];                       /* the computers this one is paired with (ui/src/remote) */
};
/* `view`: "chat", "promos", or a page of no one project ("all" is Overview, "review", "newproj"). `proj`: the project
   in front ("" for the chats of no project, null on a page). `chat`: the chat in front. */
export type Nav = { view: string; proj: string | null; chat: string | null };
/* an expert a chat's conversation says it started */
export type XpLink = { id: string; name: string; expert: string; runsOn: string; at: number };
export type Chat = {
  id: string; name: string;
  project: string;                           /* where it belongs: its project's folder ("" for none); an expert belongs where its Lead does */
  own: boolean; want: boolean;               /* it has a project of its own; it is being tied to one */
  parent: string; helper: boolean; research: boolean;
  started: boolean; busy: boolean; now: string; last: string;
  ai: string;                                /* the AI it was put on ("" for the default) */
  runsOn: string; expert: string; plain: string;
  replay: boolean;                           /* put back from a saved chat */
  when: number;                              /* an expert's last start or end of a turn seen here (ms; 0 for none) */
  xps: XpLink[];
};
export type Steps = { n: number; recent: { label: string; t: number }[] };
export type Member = { id: string; name?: string; role?: string; status?: string; ai?: string; model?: string; studio_turn?: boolean };
export type Team = { run: string | null; members: Member[] };
/* an AI a chat can run on: [id, name, the program behind it, how to get it] */
export type Ai = [string, string, string, string];
export type Sum = { total: number; input: number; output: number; cached: number; cost: number; turns: number; quiet: number };
export type UsageToday = { all: Sum; first: boolean; rows: [string, Sum, string, string][] };
/* `text` is Settings' longer words, `short` the Promos tab's */
export type EngineRow = { text: string; short: string; button: string; error: string; bar: number; ready: boolean; running: boolean; title: string };

let state: State = { projects: [], projReply: { projects: [] }, review: {}, reviewDone: {}, reviewOrder: [], reviewTab: "", reviewJump: null, light: null,
  status: {}, polls: 0, ais: [], curProj: "", settings: { open: false, page: "general", n: 0 }, usageOpen: false, usage: null, gh: null, ghNote: "", ghBusy: false,
  engine: null, saved: { chats: [], deleted: [] }, setupOut: "", scale: 100, sideW: 232, theme: { id: "fridae", mode: "dark" },
  nav: { view: "chat", proj: null, chat: null }, chats: [], steps: {}, teams: {}, notConnected: null, machines: [] };
const subs = new Set<() => void>();

export const store = {
  get: () => state,
  set(patch: Partial<State>) {
    let changed = false;
    for (const k in patch) if (!Object.is((patch as any)[k], (state as any)[k])) changed = true;
    if (!changed) return;
    state = { ...state, ...patch };
    subs.forEach((f) => f());
  },
  subscribe(f: () => void) { subs.add(f); return () => { subs.delete(f); }; },
};

/* the part of the store a component reads: it is painted again when that part is a different value */
export function useStore<T>(select: (s: State) => T): T {
  const [, again] = useReducer((n: number) => n + 1, 0);
  const value = select(state), last = useRef(value), sel = useRef(select);
  last.current = value; sel.current = select;
  useEffect(() => {
    const check = () => { if (!Object.is(sel.current(state), last.current)) again(0); };
    check();
    return store.subscribe(check);
  }, []);
  return value;
}

/* a value of one screen's own (what a page of Settings last fetched), read with .use() and painted again on .set() */
export function cell<T>(first: T) {
  let v = first; const on = new Set<() => void>();
  return {
    get: () => v,
    set(next: T) { v = next; on.forEach((f) => f()); },
    poke() { on.forEach((f) => f()); },
    use(): T { const [, again] = useReducer((n: number) => n + 1, 0); useEffect(() => { on.add(again as any); return () => { on.delete(again as any); }; }, []); return v; },
  };
}
/* paint this component again */
export function useAgain() { return useReducer((n: number) => n + 1, 0)[1] as unknown as () => void; }
