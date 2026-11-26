/* The sidebar's width: between 200px and 480px, 232px unless it was changed, remembered on this computer. The page
   itself reads it before anything is drawn (ui/index.html, `--side-w`); from then on it is set here, by the handle
   on the sidebar's edge (Grip.tsx) and by Settings, Appearance. */
import { store } from "../store";

export const MIN = 200, MAX = 480, BASE = 232;
let w = BASE;
export const sideW = () => w;
/* `keep`: also remember it (a drag keeps only where it ends) */
export function setSideW(v: number, keep?: boolean) {
  w = Math.max(MIN, Math.min(MAX, Math.round(v)));
  document.documentElement.style.setProperty("--side-w", w + "px");
  store.set({ sideW: w });
  if (keep) { try { if (w === BASE) localStorage.removeItem("side-w"); else localStorage.setItem("side-w", String(w)); } catch (e) {} }
}
setSideW(BASE);
try { const v = parseInt(localStorage.getItem("side-w") || "", 10); if (v) setSideW(v); } catch (e) {}
