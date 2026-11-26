/* The pages of Settings, in the order of the list on the left, each with its icon. */
import type { JSX } from "preact";
export type Page = { id: string; name: string; icon: JSX.Element };
const svg = (d: JSX.Element) => <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">{d}</svg>;
export const NAV: { title: string; pages: Page[] }[] = [
  { title: "App", pages: [
    { id: "general", name: "General", icon: svg(<><path d="M2 4.5h6.5M11.5 4.5H14M2 11.5h2.5M7.5 11.5H14" /><circle cx="10" cy="4.5" r="1.5" /><circle cx="6" cy="11.5" r="1.5" /></>) },
    { id: "appearance", name: "Appearance", icon: svg(<><circle cx="8" cy="8" r="5.5" /><path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor" stroke="none" /></>) },
  ] },
  { title: "AIs", pages: [
    { id: "connections", name: "Connections", icon: svg(<><circle cx="4" cy="8" r="2" /><circle cx="12" cy="8" r="2" /><path d="M6 8h4" /></>) },
    { id: "experts", name: "Experts", icon: svg(<><circle cx="6" cy="5.5" r="2" /><path d="M2.5 13c0-2 1.5-3.5 3.5-3.5S9.5 11 9.5 13" /><path d="M10.5 3.800a2 2 0 0 1 0 3.400M11.5 9.800c1.3.500 2 1.7 2 3.2" /></>) },
    { id: "permissions", name: "Permissions", icon: svg(<><path d="M8 1.800l5 1.800v4c0 3-2 5.2-5 6.6-3-1.4-5-3.6-5-6.600v-4z" /><path d="M6 8l1.5 1.5 2.7-2.9" /></>) },
    { id: "images", name: "Images", icon: svg(<><rect x="2" y="3" width="12" height="10" rx="2" /><circle cx="5.8" cy="6.5" r="1" /><path d="M2.5 11.500l3.5-3 2.5 2 2-1.5 3 2.5" /></>) },
  ] },
  { title: "Knowledge", pages: [
    { id: "skills", name: "Skills", icon: svg(<><path d="M8 2l6 3-6 3-6-3z" /><path d="M2 8l6 3 6-3M2 11l6 3 6-3" /></>) },
    { id: "knowledge", name: "Knowledge", icon: svg(<><path d="M3 2.500h7.500a2 2 0 0 1 2 2v9H5a2 2 0 0 1-2-2z" /><path d="M3 11.500a2 2 0 0 1 2-2h7.5" /></>) },
  ] },
  { title: "More", pages: [
    { id: "chats", name: "Chats", icon: svg(<><path d="M2.5 4A1.5 1.5 0 0 1 4 2.500h8A1.5 1.5 0 0 1 13.5 4v5.500A1.5 1.5 0 0 1 12 11H7l-3 2.500V11a1.5 1.5 0 0 1-1.5-1.500z" /></>) },
    { id: "guide", name: "Guide", icon: svg(<><circle cx="8" cy="8" r="5.5" /><path d="M6.3 6.300a1.8 1.8 0 1 1 2.5 1.700c-.500.3-.800.6-.800 1.200M8 11.300v.100" /></>) },
    { id: "about", name: "About", icon: svg(<><circle cx="8" cy="8" r="5.5" /><path d="M8 7.500V11M8 5v.100" /></>) },
  ] },
];
export const PAGES: Page[] = NAV.flatMap((g) => g.pages);
