/* On the Mac the page runs up under a see-through title bar (window.__fridaeFrame is "mac") and draws the close,
   minimise and full screen buttons itself, a little larger than the system's, at the left of the sidebar's head. Their
   marks show while the pointer is over them, and they go grey while another app is in front, as the system's do.
   The empty parts of the top bar move the window and a double press zooms it. Full screen hides the buttons
   (window.fridaeFull).
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
  if (!framed()) return null;
  if (mac()) return <MacButtons />;
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

function MacButtons() {
  const [on, setOn] = useState(document.hasFocus());
  useEffect(() => {
    const f = () => setOn(true), b = () => setOn(false);
    addEventListener("focus", f); addEventListener("blur", b);
    return () => { removeEventListener("focus", f); removeEventListener("blur", b); };
  }, []);
  return (
    <div class={"mac-ctl" + (on ? "" : " off")} role="group" aria-label="Window">
      <button class="mac-b close" aria-label="Close" onClick={() => send("close")}>
        <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="M1.5 1.5l5 5M6.5 1.5l-5 5" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" /></svg>
      </button>
      <button class="mac-b min" aria-label="Minimise" onClick={() => send("min")}>
        <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="M1 4h6" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" /></svg>
      </button>
      <button class="mac-b full" aria-label="Full screen" onClick={() => send("full")}>
        <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true"><path d="M1.2 3.6V1.2h2.4zM6.8 4.4v2.4H4.4z" fill="currentColor" /></svg>
      </button>
    </div>
  );
}
