/* A list in the sidebar opens and shuts with a short ease: the list's own height grows from nothing or shrinks to
   nothing while its rows fade, so the project's box and everything under it follow its lower edge as one, and
   nothing jumps when it ends. A list longer than what can be seen still eases over no more than what can be seen
   (the rest is under the sidebar's foot already). With less motion the list is simply there, or gone. */
import { cssZoom } from "../util";
import { boxOf, grow, redraw, shrink } from "./fold";

export const sideCalm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
let sideEase: { anims: Animation[]; tidy(): void } | null = null;   /* the ease going on */
/* what waits until the list has been drawn again (List.tsx runs these after each draw) */
export const afterDraw: (() => void)[] = [];
/* lists whose way out starts once the list is drawn with them inert */
const leaving: string[] = [];

export function endEase() {
  const e = sideEase; if (!e) return;
  sideEase = null; e.anims.forEach((a) => { a.onfinish = null; a.cancel(); }); e.tidy();
}
const drop = (key: string) => { shrink.delete(key); };

export function easeList(key: string, open: boolean, _addWas?: number | null) {
  endEase();
  const box = boxOf(key), list = document.getElementById("proj-list");
  const gone = () => { if (!open) { drop(key); redraw.poke(); } };
  if (!box || !list) { gone(); return; }
  const inner = box.firstElementChild as HTMLElement | null;
  if (sideCalm() || !box.animate || !inner) { gone(); return; }
  /* px of the page per px of the window (the text-size setting scales the page; engines differ on what a
     rectangle is measured in) */
  const lr = list.getBoundingClientRect(), s = cssZoom(), ratio = list.offsetWidth ? lr.width / list.offsetWidth : 1, z = Math.abs(ratio - s) <= Math.abs(ratio - 1) ? s : 1;
  const br = box.getBoundingClientRect(), seen = (lr.bottom - br.top) / z;
  if (seen <= 0) { gone(); return; }   /* all of it is under the foot: nothing to see */
  const full = box.offsetHeight, tall = Math.min(full, Math.max(0, seen));
  /* the list's gap to what follows goes with it, so the last 2px do not jump either */
  const gap = parseFloat(getComputedStyle(box.parentElement!).rowGap) || 0, mb = parseFloat(getComputedStyle(box).marginBottom) || 0;
  const shut = { height: "0px", marginBottom: (mb - gap).toFixed(2) + "px" }, wide = { height: full + "px", marginBottom: mb + "px" };
  const timing: KeyframeAnimationOptions = { duration: Math.round(Math.max(200, Math.min(320, 150 + tall * 0.35))), easing: "cubic-bezier(0.33, 1, 0.68, 1)", fill: open ? "none" : "forwards" };
  box.classList.add("easing");
  const anims = [box.animate(open ? [shut, wide] : [wide, shut], timing),
    inner.animate(open ? [{ opacity: 0 }, { opacity: 1 }] : [{ opacity: 1 }, { opacity: 0, offset: 0.6 }, { opacity: 0 }], timing)];
  const tidy = () => { box.classList.remove("easing"); if (!open) drop(key); };
  sideEase = { anims, tidy };
  anims[0].onfinish = () => {
    if (sideEase && sideEase.anims === anims) sideEase = null;
    if (open) { anims.forEach((a) => a.cancel()); tidy(); return; }
    /* shut: the list goes in the same moment its ease lets go of it */
    drop(key); afterDraw.push(() => { anims.forEach((a) => a.cancel()); tidy(); }); redraw.poke();
  };
}
/* show or hide a list in the sidebar: `flip` changes what is kept, then the list eases in, or eases out and goes */
export function foldSide(key: string, shown: boolean, flip: () => void) {
  endEase();
  const node = boxOf(key);
  flip();
  if (shown && node && node.isConnected && !sideCalm()) { shrink.add(key); leaving.push(key); }
  else if (!shown) grow.add(key);
  redraw.poke();
}
/* the list was drawn: lists on their way out start their ease, lists just shown ease in */
export function drawn(addWas: number | null) {
  for (const f of afterDraw.splice(0)) f();
  for (const key of leaving.splice(0)) easeList(key, false);
  for (const key of [...grow]) if (boxOf(key)) easeList(key, true, addWas);
  grow.clear();
}
