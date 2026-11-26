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
};

let state: State = { projects: [], review: {}, reviewDone: {}, reviewOrder: [], reviewTab: "", reviewJump: null, light: null };
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
