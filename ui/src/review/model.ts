/* What Review shows, worked out from what the store holds. */
import { useReducer } from "preact/hooks";
import { TOKEN } from "../api";
import type { Answered, NeedCard, ReviewItem, ReviewReply } from "../store";

/* the Lead's own board cards point at Review, so Review leaves them out */
const LEAD_CARD = "Waiting for your answer in Review";
const LEAD_PAUSED_CARD = "The studio run is paused. Resume it in Review";

type Done = Record<string, Answered>;
/* one thing on the page: finished work to approve (`item`) or a to-do only the user can do (`need`) */
export type Thing = { key: string; id: string; path: string; item?: ReviewItem; need?: NeedCard; media: string[]; pics: string[]; said: string | null };

const needCards = (r?: ReviewReply) => ((r && r.needs) || []).filter((c) => (c.note || "") !== LEAD_CARD && (c.note || "") !== LEAD_PAUSED_CARD);
const shown = (done: Done, id: string) => !done[id] || !done[id].gone;
export const isClip = (file: string) => /\.(mp4|mov)$/i.test(file);
/* a project on another of the user's computers has its pictures there too (`via`: ui/src/remote/route.ts) */
export const mediaUrl = (r: ReviewReply, id: string, file: string) =>
  (r.via || "") + "/media/review/" + encodeURIComponent(r.key) + "/" + encodeURIComponent(id) + "/" + encodeURIComponent(file) + "?t=" + TOKEN;

/* how many things need the user on a project: what its run waits for, and its to-dos. The number on its tab. */
export function needsCount(r: ReviewReply | undefined, done: Done) {
  const waiting = ((r && r.items) || []).filter((i) => i.status === "waiting" && !done[i.id] && i.blocking);
  return waiting.length + needCards(r).filter((c) => !done[c.id]).length;
}

/* a run is stopped until the user answers, or a to-do waits: the sidebar says so on the count */
export function needsStrong(r: ReviewReply | undefined, done: Done) {
  return needCards(r).some((c) => !done[c.id]) || ((r && r.items) || []).some((i) => i.status === "waiting" && !done[i.id] && !!i.blocking);
}
/* a run that is going: started and not through its last stage */
export function runActive(run: any) {
  const done = !!(run && run.stages && run.stages.length && run.stages.every((s: any) => s.status === "done"));
  return !!(run && run.on && !done);
}
/* what a project has on the page: what its run waits for, the to-dos only the user can do, then finished work to
   look at. Something just answered stays as one line until the next refresh after it. */
export function things(path: string, r: ReviewReply | undefined, done: Done): Thing[] {
  if (!r) return [];
  const said = (id: string) => (done[id] ? done[id].said || "" : null);
  const item = (i: ReviewItem): Thing => {
    const media = i.media || [];
    return { key: path + "\n" + [i.id, i.created, media.join("/"), i.title].join("|"), id: i.id, path, item: i, media, pics: media.filter((f) => !isClip(f)).map((f) => mediaUrl(r, i.id, f)), said: said(i.id) };
  };
  const need = (c: NeedCard): Thing => ({ key: path + "\n" + [c.id, c.title, c.note].join("|"), id: c.id, path, need: c, media: [], pics: [], said: said(c.id) });
  const items = (r.items || []).filter((i) => (i.status === "waiting" || done[i.id]) && shown(done, i.id));
  return items.filter((i) => i.blocking).map(item).concat(needCards(r).filter((c) => shown(done, c.id)).map(need), items.filter((i) => !i.blocking).map(item));
}

/* What a thing's box remembers while it is on the page, whichever tab is in front: whether it has been shown (its
   entrance plays once), and a change note being written. */
export type Kept = { entered: boolean; open: boolean; note: string; err: string; busy: boolean };
const kept = new Map<string, Kept>();
export function useKept(key: string): [Kept, (patch: Partial<Kept>) => void] {
  const [, again] = useReducer((n: number) => n + 1, 0);
  if (!kept.has(key)) kept.set(key, { entered: false, open: false, note: "", err: "", busy: false });
  const set = (patch: Partial<Kept>) => { const now = kept.get(key); if (now) { kept.set(key, { ...now, ...patch }); again(0); } };
  return [kept.get(key)!, set];
}
export function forget(keep: Set<string>) { for (const k of [...kept.keys()]) if (!keep.has(k)) kept.delete(k); }
