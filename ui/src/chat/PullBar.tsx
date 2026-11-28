/* A bar tucked away behind a tab at the top of a page, which slides out when the tab is pressed: a chat's (its name,
   the run's controls), the Overview's (what is going in every project) and Review's (which project to look at). One look
   for every page: the bar, the tab and the top bar above them are one sheet of frosted glass the page runs under, one
   lighter line runs under the bar and round the tab, and the tab is the same size everywhere (TAB_W by TAB_H). Whether
   it is out is remembered under `keep`. While it moves (and while tucked) it is clipped; once out, its menus may hang
   below it again. */
import { useLayoutEffect, useRef, useState } from "preact/hooks";
import { TabShape, tabPaths } from "./tab";

export const TAB_W = 148, TAB_H = 16;
/* the top bar's height (its glass is the bar's, reaching up behind it) and how far in the tab sits */
const TOP = 48, TAB_X = 24;
const DOWN = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2 3.5l2.5 2.5L7 3.5" /></svg>;

export function PullBar({ keep, what, bar, cls, onTuck, children }: { keep: string; what: string; bar: string; cls?: string; onTuck?(): void; children: any }) {
  const [out, setOut] = useState(() => { try { return localStorage.getItem(keep) === "1"; } catch (e) { return false; } });
  const [moving, setMoving] = useState(false);
  const me = useRef<HTMLDivElement>(null), glass = useRef<HTMLDivElement>(null);
  /* the glass is cut to the top bar, the bar and the tab as one shape, drawn again as the bar slides; the page under it
     is told how tall the bar is (--fold-h), so what it shows first starts below it */
  useLayoutEffect(() => {
    const el = me.current!, g = glass.current!;
    const draw = () => {
      const box = el.getBoundingClientRect(), w = box.width, h = box.height;
      if (el.parentElement) el.parentElement.style.setProperty("--fold-h", h.toFixed(2) + "px");
      g.style.clipPath = `path('M0 0H${w}V${(TOP + h).toFixed(2)}H0Z${tabPaths(TAB_W, TAB_H, false, { dx: TAB_X, dy: TOP + h - 1 }).fill}')`;
    };
    draw();
    const ro = new ResizeObserver(draw); ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const fold = () => {
    const v = !out; setOut(v); setMoving(true); setTimeout(() => setMoving(false), 320);
    if (!v && onTuck) onTuck();
    try { localStorage.setItem(keep, v ? "1" : "0"); } catch (e) {}
  };
  return (
    <div ref={me} class={"headfold" + (cls ? " " + cls : "") + (out ? " out" : "") + (moving || !out ? " clip" : "")}>
      <div class="hf-glass" ref={glass} />
      <div class="hf-in" inert={!out}><div class={bar}>{children}</div></div>
      <button class="hf-tab" aria-expanded={out ? "true" : "false"} aria-label={(out ? "Tuck away " : "Show ") + what} title={(out ? "Tuck away " : "Show ") + what} onClick={fold}><TabShape w={TAB_W} h={TAB_H} />{DOWN}</button>
    </div>
  );
}
