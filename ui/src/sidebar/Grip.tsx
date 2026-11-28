/* The sidebar's right edge is a handle: drag it (nothing moves until the pointer has gone 3px), or the arrow keys,
   16px a press; Home and End go to the narrowest and the widest, Enter and a double press back to 232px. A narrow
   window sets its own width and hides the handle (sidebar.css). */
import { useRef } from "preact/hooks";
import { useStore } from "../store";
import { cssZoom } from "../util";
import { BASE, MAX, MIN, setSideW, sideW } from "./width";

export function Grip() {
  const w = useStore((s) => s.sideW), ref = useRef<HTMLDivElement>(null);
  const down = (ev: PointerEvent) => {
    const grip = ref.current!;
    if (ev.button !== 0) return;
    ev.preventDefault(); grip.focus({ preventScroll: true });
    const x0 = ev.clientX, w0 = sideW(), z = cssZoom();
    let moved = false;
    grip.classList.add("drag"); document.body.classList.add("side-drag");
    try { grip.setPointerCapture(ev.pointerId); } catch (e) {}
    const move = (e: PointerEvent) => { if (!moved && Math.abs(e.clientX - x0) < 3) return; moved = true; setSideW(w0 + (e.clientX - x0) / z); };
    const end = () => {
      grip.removeEventListener("pointermove", move); grip.removeEventListener("pointerup", end); grip.removeEventListener("pointercancel", end);
      grip.classList.remove("drag"); document.body.classList.remove("side-drag"); setSideW(sideW(), true);
    };
    grip.addEventListener("pointermove", move); grip.addEventListener("pointerup", end); grip.addEventListener("pointercancel", end);
  };
  const key = (ev: KeyboardEvent) => {
    const now = sideW(), to = ev.key === "ArrowLeft" ? now - 16 : ev.key === "ArrowRight" ? now + 16 : ev.key === "Home" ? MIN : ev.key === "End" ? MAX : ev.key === "Enter" ? BASE : null;
    if (to === null) return;
    ev.preventDefault(); ev.stopPropagation(); setSideW(to, true);
  };
  return <div class="side-grip" id="side-grip" ref={ref} role="separator" aria-orientation="vertical" aria-label="Sidebar width" aria-valuemin={MIN} aria-valuemax={MAX} aria-valuenow={w} tabIndex={0}
    title="Drag to resize. Double-click to reset." onPointerDown={down} onDblClick={() => setSideW(BASE, true)} onKeyDown={key}></div>;
}
