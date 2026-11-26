/* Which page of Settings is shown, and what each page does when it comes into view. The window is opened
   and closed by ../windows.ts. */
import { store } from "../store";
import { PAGES } from "./nav";

const onShow: Record<string, (() => void)[]> = {}, onOpen: ((page: string, asked: boolean) => void)[] = [], onAny: (() => void)[] = [];
/* a page asks to be told when it is picked, and when the window opens (asked: a page was named) */
export function whenShown(pages: string[], f: () => void) { pages.forEach((p) => (onShow[p] = onShow[p] || []).push(f)); }
export function whenOpened(f: (page: string, asked: boolean) => void) { onOpen.push(f); }
export function whenPageChanges(f: () => void) { onAny.push(f); }

export const pageNow = () => store.get().settings.page;
/* Settings is open on one of these pages */
export function shown(...pages: string[]) { const s = store.get().settings; return s.open && pages.includes(s.page); }

export function showPage(name: string) {
  if (!PAGES.some((p) => p.id === name)) name = "general";
  store.set({ settings: { ...store.get().settings, page: name } });
  onAny.forEach((f) => f());
  (onShow[name] || []).forEach((f) => f());
}
export function openSettings(page?: string) {
  const asked = typeof page === "string";
  if (asked) showPage(page!);
  const s = store.get().settings;
  store.set({ settings: { ...s, open: true, n: s.n + 1 } });
  onOpen.forEach((f) => f(s.page, asked));
}
export function closeSettings() { const s = store.get().settings; if (s.open) store.set({ settings: { ...s, open: false } }); }
