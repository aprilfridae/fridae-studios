/* A list in the sidebar opens and shuts with a short ease, and nothing is laid out again while it runs: the page
   takes its final shape at once, and the ease is only moves and a fade, which the browser does without the page's
   help. The list is a window (`.s-kids`) over its rows (`.s-in`): the window slides one way and the rows the other,
   so the rows stay where they are and the window's lower edge uncovers them; what follows the list rides on that
   edge. A list longer than what can be seen is held to that length for the ease (the rest is under the sidebar's
   foot already), and New project, which is pinned, goes from where it was to where it is. With less motion the list
   is simply there, or gone. */
import { store } from "../store";
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

export function easeList(key: string, open: boolean, addWas?: number | null) {
  endEase();
  const box = boxOf(key), list = document.getElementById("proj-list");
  const gone = () => { if (!open) { drop(key); redraw.poke(); } };
  if (!box || !list) { gone(); return; }
  const inner = box.firstElementChild as HTMLElement | null;
  if (sideCalm() || !box.animate || !inner) { gone(); return; }
  /* px of the page per px of the window (the text-size setting scales the page; engines differ on what a
     rectangle is measured in) */
  const scale = store.get().scale, lr = list.getBoundingClientRect(), s = (scale > 0 ? scale : 100) / 100, ratio = list.offsetWidth ? lr.width / list.offsetWidth : 1, z = Math.abs(ratio - s) <= Math.abs(ratio - 1) ? s : 1;
  const top = (n: Element) => n.getBoundingClientRect().top / z, br = box.getBoundingClientRect(), seen = (lr.bottom - br.top) / z;
  if (seen <= 0) { gone(); return; }   /* all of it is under the foot: nothing to see */
  /* what follows the list, down to the end of the sidebar: it rides on the list's lower edge; New project is pinned */
  const after: HTMLElement[] = [];
  for (let n: Element | null = box; n && n !== list; n = n.parentElement) for (let f = n.nextElementSibling; f; f = f.nextElementSibling) after.push(f as HTMLElement);
  const add = after.find((f) => f.matches(".proj.add")), rest = after.filter((f) => f !== add);
  let tall = br.height / z, held = false, moved: number, addMoved = 0;
  if (tall > seen + 1) { tall = Math.ceil(seen); box.style.height = tall + "px"; held = true; }
  if (open) {
    moved = tall + (parseFloat(getComputedStyle(box.parentElement!).rowGap) || 0) + (parseFloat(getComputedStyle(box).marginBottom) || 0);
    if (add && typeof addWas === "number") addMoved = top(add) - addWas / z;
  } else {
    /* where things will be once the list is gone: looked at with the list taken out for a moment */
    const keep = list.scrollTop, now = rest.length ? top(rest[0]) : 0, addNow = add ? top(add) : 0;
    box.style.display = "none";
    moved = rest.length ? now - top(rest[0]) : tall; addMoved = add ? addNow - top(add) : 0;
    box.style.display = ""; list.scrollTop = keep;
  }
  const y = (px: number) => "translateY(" + px.toFixed(2) + "px)", path = (a: number, b: number) => (open ? [{ transform: y(a) }, { transform: y(b) }] : [{ transform: y(b) }, { transform: y(a) }]);
  const timing: KeyframeAnimationOptions = { duration: Math.round(Math.max(200, Math.min(360, 150 + tall * 0.35))), easing: "cubic-bezier(0.33, 1, 0.68, 1)", fill: open ? "none" : "forwards" };
  const hidden = { transform: y(tall), opacity: 0 }, shown = { transform: y(0), opacity: 1 };
  box.classList.add("easing");
  const riders = rest.concat(add ? [add] : []); riders.forEach((f) => { f.style.willChange = "transform"; });
  const anims = [box.animate(path(-tall, 0), timing), inner.animate(open ? [hidden, { opacity: 1, offset: 0.7 }, shown] : [shown, { opacity: 0, offset: 0.7 }, hidden], timing)]
    .concat(rest.map((f) => f.animate(path(-moved, 0), timing)), add && addMoved ? [add.animate(path(-addMoved, 0), timing)] : []);
  const tidy = () => { box.classList.remove("easing"); if (held) box.style.height = ""; riders.forEach((f) => { f.style.willChange = ""; }); if (!open) drop(key); };
  sideEase = { anims, tidy };
  anims[0].onfinish = () => {
    if (sideEase && sideEase.anims === anims) sideEase = null;
    if (open) { tidy(); return; }
    /* shut: the list goes and what rode on it lets go in the same moment, so nothing is seen to jump */
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
