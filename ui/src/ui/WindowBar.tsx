/* On the Mac the system's close, minimise and zoom buttons stay, but the page runs up under the see-through title bar
   (window.__fridaeFrame is "mac"): the sidebar's head leaves them room, the empty parts of the top bar move the window,
   and a double press zooms it. Full screen hides the buttons, so the room goes too (window.fridaeFull).
   The window's own controls, on Windows, where the system's title bar was: minimise, maximise (restore while the
   window fills the screen) and close at the top right of the page's top bar. The empty parts of the top bar and the
   sidebar's head move the window (a double press maximises), and a thin strip along the top edge resizes it. The app
   says it draws no title bar (window.__fridaeFrame); in a browser none of this is shown. */
import { useEffect, useState } from "preact/hooks";

const w = window as any;
export const framed = () => !!(w.__fridaeFrame && w.ipc && w.ipc.postMessage);
const mac = () => w.__fridaeFrame === "mac";
const send = (what: string) => { if (framed()) w.ipc.postMessage("win:" + what); };
/* what may not start a drag: anything that is pressed, typed in or chosen */
const ACTIVE = "button, input, textarea, select, a, label, summary, [role=button], [role=tab], [contenteditable], .chip, .menu, .grip, .side-grip";

let maxed = false;
const subs = new Set<(m: boolean) => void>();
w.fridaeFull = (f: boolean) => { document.body.classList.toggle("win-full", f); };
w.fridaeMax = (m: boolean) => { maxed = m; document.body.classList.toggle("win-max", m); subs.forEach((f) => f(m)); };

export function setUpFrame() {
  if (!framed()) return;
  document.body.classList.add(mac() ? "mac-frame" : "framed");
  document.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    const t = e.target as Element;
    if (!t.closest || !t.closest("[data-drag]") || t.closest(ACTIVE)) return;
    e.preventDefault();
    if (e.detail === 2) send("max"); else send("drag");
  });
}

export function WindowBar() {
  const [max, setMax] = useState(maxed);
  useEffect(() => { subs.add(setMax); return () => { subs.delete(setMax); }; }, []);
  if (!framed() || mac()) return null;
  return (
    <>
      <div class="win-edge" aria-hidden="true" onMouseDown={(e) => { if (e.button === 0 && !max) { e.preventDefault(); send("resize-top"); } }}></div>
      <div class="win-ctl" role="group" aria-label="Window">
        <button class="win-b" aria-label="Minimise" title="Minimise" onClick={() => send("min")}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M0 5.5h10" stroke="currentColor" stroke-width="1" /></svg>
        </button>
        <button class="win-b" aria-label={max ? "Restore" : "Maximise"} title={max ? "Restore" : "Maximise"} onClick={() => send("max")}>
          {max
            ? <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><path d="M2.5 2.5V.5h7v7h-2" /><rect x=".5" y="2.5" width="7" height="7" /></svg>
            : <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true"><rect x=".5" y=".5" width="9" height="9" /></svg>}
        </button>
        <button class="win-b close" aria-label="Close" title="Close" onClick={() => send("close")}>
          <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M.5.5l9 9M9.5.5l-9 9" stroke="currentColor" stroke-width="1" /></svg>
        </button>
      </div>
    </>
  );
}
