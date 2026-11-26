/* What the page knows, in one place, while the old page (ui/index.html) and the new screens live side by side.
   The old page writes here where it already receives data; a screen reads with useStore and is painted again
   when what it reads has changed. */
import { useEffect, useReducer, useRef } from "preact/hooks";

export type Project = { path: string; name: string; type?: string };
export type ReviewItem = { id: string; title?: string; summary?: string; plain?: string; status: string; blocking?: boolean; created?: number; media?: string[] };
export type NeedCard = { id: string; title?: string; note?: string };
/* a project's GET /api/review reply, as far as Review uses it */
export type ReviewReply = { project: string; key: string; items?: ReviewItem[]; needs?: NeedCard[] };
/* something answered on the page: it stays as one line (`said`) until `gone` */
export type Answered = { at: number; gone?: boolean; said?: string };
export type Light = { list: string[]; at: number; from: HTMLElement | null };

export type State = {
  projects: Project[];
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
  engine: EngineRow | null;                  /* the promo engine's row, as the old page words it */
  saved: { chats: any[]; deleted: any[] };   /* GET /api/history */
  setupOut: string;                          /* what Run a full check printed */
  scale: number; sideW: number;              /* text size (%) and the sidebar's width (px) */
  theme: { id: string; mode: string };       /* the colour theme in use */
};
/* an AI a chat can run on: [id, name, the program behind it, how to get it] */
export type Ai = [string, string, string, string];
export type Sum = { total: number; input: number; output: number; cached: number; cost: number; turns: number; quiet: number };
export type UsageToday = { all: Sum; first: boolean; rows: [string, Sum, string, string][] };
export type EngineRow = { text: string; button: string; error: string; bar: number; ready: boolean; running: boolean; title: string };

let state: State = { projects: [], review: {}, reviewDone: {}, reviewOrder: [], reviewTab: "", reviewJump: null, light: null,
  status: {}, polls: 0, ais: [], curProj: "", settings: { open: false, page: "general", n: 0 }, usageOpen: false, usage: null, gh: null, ghNote: "", ghBusy: false,
  engine: null, saved: { chats: [], deleted: [] }, setupOut: "", scale: 100, sideW: 232, theme: { id: "fridae", mode: "dark" } };
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
