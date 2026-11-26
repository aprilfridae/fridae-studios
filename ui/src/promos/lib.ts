/* What the Promos tab keeps: the finished promos (the library), the chats it started (In progress), and the form. */
import { api } from "../api";
import { cell, store } from "../store";

export type Card = { id: string; agent?: string; folder?: string; style?: string; duration?: number; created?: string; mtime?: number; caption_text?: string; check?: any };
/* card id -> the newest copy of it */
export const lib = cell(new Map<string, Card>());
/* newest data wins: an older copy of a card (a replayed event) never replaces a newer one */
export function upsert(card: Card | null | undefined) {
  if (!card || !card.id) return false;
  const had = lib.get().get(card.id);
  if (had && (had.mtime || 0) >= (card.mtime || 0)) return false;
  lib.get().set(card.id, card); lib.poke();
  return true;
}
export async function loadPromos() {
  try { const r = await api("/api/promos"); (r.cards || []).forEach(upsert); } catch (e) {}
}

/* chats this tab started: [{ agent, title, at }], kept in this browser */
export type PromoChat = { agent: string; title: string; at: number };
let first: PromoChat[] = [];
try { first = JSON.parse(localStorage.getItem("promo-chats") || "[]").filter((x: any) => x && typeof x.agent === "string"); } catch (e) { first = []; }
export const prChats = cell(first);
export function setChats(list: PromoChat[]) {
  try { localStorage.setItem("promo-chats", JSON.stringify(list.slice(-30))); } catch (e) {}
  prChats.set(list);
}

/* the game a promo (or a promo chat) belongs to: its chat's project, "" for none, null when its chat is not known */
export function promoProj(agent?: string): string | null {
  if (!agent) return null;
  const s = store.get(), p = s.chats.find((x) => x.id === agent);
  if (p) return p.project;
  const c = (s.saved.chats || []).find((x: any) => x.id === agent);
  return c ? c.project || "" : null;
}
/* shown for the game in front: its own, and those of no game (marked so) */
export function promoHere(agent?: string) {
  const k = promoProj(agent), cur = store.get().nav.proj || "";
  return { show: k === null || k === "" || k === cur, nogame: (k === null || k === "") && !!cur };
}
export const chatKnown = (id?: string) => { const s = store.get(); return !!id && (s.chats.some((x) => x.id === id) || (s.saved.chats || []).some((c: any) => c.id === id)); };

/* sentence case for words that arrive in lower case, like a style name: "update-log" is "Update log" */
export function styleName(s?: string) { const t = String(s || "").replace(/-/g, " "); return t ? t.charAt(0).toUpperCase() + t.slice(1) : t; }
export const promoLen = (secs?: number) => { const s = Math.round(Number(secs) || 0); return s ? Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0") : ""; };
export const promoWhen = (c: Card) => Date.parse(c.created || "") || (c.mtime || 0) * 1000;

/* the form: what is picked, and a passing message beside Make promo (the reason it is disabled comes back after it) */
export const VIDEO = ["mp4", "mov", "mkv", "webm"], AUDIO = ["mp3", "wav", "m4a", "ogg"];
const ext = (p: string) => (/\.([A-Za-z0-9]+)$/.exec(p) || ["", ""])[1].toLowerCase();
export const form = cell({ clips: [] as string[], music: "", reference: "", style: "", length: 30, project: null as string | null, forProj: null as string | null,
  brief: "", voice: "", mute: false, msg: "", msgUntil: 0 });
export function setForm(patch: Partial<ReturnType<typeof form.get>>) { form.set({ ...form.get(), ...patch }); }
export function prNote(text: string) { setForm({ msg: text, msgUntil: Date.now() + 5000 }); setTimeout(() => form.poke(), 5100); }
/* videos become clips; a music file becomes the music when none is chosen yet */
export function addPaths(paths: string[]) {
  const f = form.get(), clips = f.clips.slice(); let music = f.music, skipped = 0;
  document.getElementById("pr-form")?.classList.remove("drop");
  for (const p of paths) {
    const x = ext(p);
    if (VIDEO.includes(x)) { if (!clips.includes(p)) clips.push(p); }
    else if (AUDIO.includes(x) && !music) music = p;
    else skipped++;
  }
  setForm({ clips, music });
  if (skipped) prNote(skipped === 1 ? "One file was left out: only videos can be clips, and one music file." : skipped + " files were left out: only videos can be clips, and one music file.");
}
