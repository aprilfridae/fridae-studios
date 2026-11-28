/* The conversation: the rows of the chat in front, and of them only the ones in view.

   A long chat has thousands of rows; the column holds the ones around what is seen and two empty blocks standing in
   for the rest (above and below), as tall as those rows are: measured when they were last drawn, guessed until then.
   What is seen never moves under the reader: at the end it stays at the end as the chat grows (`follow`), anywhere
   else the row at the top of the view stays where it is (`anchor`) whatever is measured or drawn above it.
   A short chat (up to ALL rows) is drawn whole, exactly as the old page drew it. */
import { useEffect, useLayoutEffect, useRef } from "preact/hooks";
import { useAgain } from "../store";
import { frontTl, useTick, type Row, type Timeline as Tl } from "./chatStore";
import { RowBody } from "./Rows";
import { keep } from "./keep";
import { TabShape, tabPaths } from "./tab";

const ALL = 40, OVER = 700;

/* one row: drawn again when touch(row) says so, and not when the list around it moves */
const RowView = keep(function RowView({ row }: { row: Row }) {
  const again = useAgain(), v = row.v;
  useEffect(() => { row.on = again; if (row.v !== v) again(); return () => { if (row.on === again) row.on = undefined; }; }, [row]);
  return <RowBody row={row} />;
}, (a, b) => a.row === b.row);

/* how tall a row is: as last measured (to the top of the next row), else a guess from what is in it */
function tall(row: Row): number {
  if (row.h !== undefined) return row.h;
  if (row.k === "you") return 82 + 24 * Math.floor((row.text || "").length / 70) + (row.images ? 100 : 0);
  if (row.k === "turn") { let n = 52; for (const pt of row.parts) n += pt.k === "prose" ? 34 + 0.34 * pt.text.length : pt.k === "steps" ? 26 : 64; return n; }
  return row.k === "tag" || row.k === "night" ? 22 : row.k === "hello" ? 320 : row.k === "queue" ? 60 * row.items.length : 44;
}

/* whether the reader is away from the end of the chat in front: the way back shows then */
const away = { on: false, subs: new Set<() => void>(), go: () => {} };
function setAway(on: boolean) { if (away.on !== on) { away.on = on; away.subs.forEach((f) => f()); } }
/* what is seen is put right again: after the message box changed height, or the page came back into view */
export let relayout = () => {};

function List({ tl }: { tl: Tl }) {
  const again = useAgain(), col = useRef<HTMLDivElement>(null);
  const rows = tl.rows, n = rows.length;
  const w = useRef<{ a: number; b: number; top: number; bot: number } | null>(null);
  /* the rows to draw, from where the reader is; before anything is laid out, the end (or around the kept row) */
  const range = () => {
    const sc = col.current && (col.current.parentElement as HTMLElement);
    if (n <= ALL) return { a: 0, b: n - 1, top: 0, bot: 0 };
    if (!sc || !sc.clientHeight) {
      if (w.current && w.current.b < n) return w.current;
      const at = tl.follow || !tl.anchor ? n - 1 : Math.max(0, rows.indexOf(tl.anchor.row));
      const a = Math.max(0, tl.follow || !tl.anchor ? n - ALL : at - 4), b = Math.min(n - 1, a + ALL - 1);
      let top = 0, bot = 0; for (let i = 0; i < a; i++) top += tall(rows[i]); for (let i = b + 1; i < n; i++) bot += tall(rows[i]);
      return { a, b, top, bot };
    }
    let total = 0; for (let i = 0; i < n; i++) total += tall(rows[i]);
    const view = sc.clientHeight, from = (tl.follow ? total - view - 300 : sc.scrollTop - col.current!.offsetTop) - OVER, to = tl.follow ? Infinity : from + view + 2 * OVER;
    let a = -1, b = n - 1, y = 0, top = 0, bot = 0;
    for (let i = 0; i < n; i++) {
      const h = tall(rows[i]);
      if (a < 0 && (y + h > from || i === n - 1)) { a = i; top = y; }
      if (y >= to && a >= 0) { b = i - 1; for (let j = i; j < n; j++) bot += tall(rows[j]); break; }
      y += h;
    }
    if (b < a) b = a;
    return { a, b, top, bot };
  };
  /* The rows drawn now stay drawn while they still cover what is seen with room to spare: a new set is worked out
     only once the reader nears an edge of them. Working it out on every move made the guessed heights of rows not yet
     drawn decide it, and two sets could take turns frame after frame (the view shook between them). */
  const sum = (i: number, j: number) => { let s = 0; for (let k = i; k < j; k++) s += tall(rows[k]); return s; };
  const covered = (at: { a: number; b: number }) => {
    const c = col.current, sc = c && (c.parentElement as HTMLElement);
    if (!c || !sc || !sc.clientHeight || at.b >= n || (tl.follow && at.b !== n - 1)) return false;
    const off = at.a > 0 ? 1 : 0, f = c.children[off] as HTMLElement | undefined, l = c.children[off + at.b - at.a] as HTMLElement | undefined;
    if (!f || !l) return false;
    return (at.a === 0 || f.offsetTop <= sc.scrollTop - OVER / 3) && (at.b === n - 1 || l.offsetTop + l.offsetHeight >= sc.scrollTop + sc.clientHeight + OVER / 3);
  };
  const pick = () => {
    const was = w.current;
    if (n > ALL && was && covered(was)) return { a: was.a, b: was.b, top: sum(0, was.a), bot: sum(was.b + 1, n) };
    return range();
  };
  const cur = pick(); w.current = cur;
  const first = useRef(true), tries = useRef(0);
  /* where this list last put the scroll itself (its own moves are not the reader's), and where the reader last was */
  const ours = useRef(-1), last = useRef(0);
  /* the row at the top of what is seen, and how far down it sits: what is kept still while the chat changes around it */
  const grab = (c: HTMLElement, sc: HTMLElement, at: { a: number; b: number }) => {
    const kids = c.children, off = at.a > 0 ? 1 : 0;
    for (let i = at.a; i <= at.b; i++) {
      const el = kids[off + i - at.a] as HTMLElement | undefined;
      if (el && el.offsetTop + el.offsetHeight > sc.scrollTop) { tl.anchor = { row: tl.rows[i], dy: el.offsetTop - sc.scrollTop }; return; }
    }
  };
  const fix = () => {
    const c = col.current, sc = c && (c.parentElement as HTMLElement);
    if (!c || !sc || !sc.clientHeight) return;
    const kids = c.children, off = cur.a > 0 ? 1 : 0;
    /* a chat shown again: back to where the reader was in it */
    if (first.current) {
      first.current = false;
      const at = tl.follow || !tl.anchor ? -1 : rows.indexOf(tl.anchor.row), el = at >= cur.a && at <= cur.b ? (kids[off + at - cur.a] as HTMLElement) : null;
      if (el) sc.scrollTop = el.offsetTop - tl.anchor!.dy; else { tl.follow = true; tl.anchor = null; }
    }
    /* measure: a row's height is the way to the top of the next one (the space between them is part of it) */
    for (let i = cur.a; i <= cur.b; i++) {
      const el = kids[off + i - cur.a] as HTMLElement | undefined, next = kids[off + i - cur.a + 1] as HTMLElement | undefined;
      if (!el) break;
      (rows[i] as any).el = el;
      if (i < cur.b && next) rows[i].h = next.offsetTop - el.offsetTop;
      else if (i === n - 1) rows[i].h = el.offsetHeight;
    }
    const now = pick();
    if ((now.a !== cur.a || now.b !== cur.b || Math.abs(now.top - cur.top) > 0.5 || Math.abs(now.bot - cur.bot) > 0.5) && tries.current++ < 8) { again(); return; }
    tries.current = 0;
    if (tl.follow) { if (sc.scrollHeight - sc.scrollTop - sc.clientHeight > 0.5) { sc.scrollTop = sc.scrollHeight; ours.current = sc.scrollTop; } }
    else if (tl.anchor) {
      const el = (tl.anchor.row as any).el as HTMLElement | undefined;
      if (el && el.parentElement === c) { const d = el.offsetTop - sc.scrollTop - tl.anchor.dy; if (Math.abs(d) >= 1) { sc.scrollTop += d; ours.current = sc.scrollTop; } }
      else grab(c, sc, cur);   /* the reader jumped far (the scrollbar): the kept row is one of those now drawn */
    } else grab(c, sc, cur);
    setAway(!tl.follow);
  };
  const fixRef = useRef(fix); fixRef.current = fix;
  useLayoutEffect(() => { fix(); });
  useLayoutEffect(() => {
    const c = col.current!, sc = c.parentElement as HTMLElement;
    const onChange = () => again();
    tl.subs.add(onChange);
    /* the reader scrolled: any move up lets go of the end at once (a wheel scrolls smoothly, a little at a time, and
       the chat must not pull it back while it grows); moving down to near the end follows it again */
    const leave = () => { if (tl.follow) { tl.follow = false; grab(c, sc, w.current!); setAway(true); } };
    const onScroll = () => {
      if (!sc.clientHeight) return;
      const top = sc.scrollTop, mine = Math.abs(top - ours.current) < 1, gap = sc.scrollHeight - top - sc.clientHeight;
      if (!mine) {
        if (top < last.current - 0.5) tl.follow = gap < 2;
        else if (gap < 80) tl.follow = true;
      }
      last.current = top;
      if (tl.follow) tl.anchor = null;
      else grab(c, sc, w.current!);
      setAway(!tl.follow);
      const was = w.current!, now = pick();
      if (now.a !== was.a || now.b !== was.b) again();
    };
    const onWheel = (e: WheelEvent) => { if (e.deltaY < 0) leave(); };
    const onKey = (e: KeyboardEvent) => { if (["PageUp", "ArrowUp", "Home"].includes(e.key) && !(e.target as HTMLElement).closest("textarea,input")) leave(); };
    /* a press on the scrollbar (the column's own edge, not a row) */
    const onDown = (e: PointerEvent) => { if (e.target === sc && e.offsetX > sc.clientWidth) leave(); };
    sc.addEventListener("scroll", onScroll, { passive: true });
    sc.addEventListener("wheel", onWheel, { passive: true });
    sc.addEventListener("keydown", onKey);
    sc.addEventListener("pointerdown", onDown);
    const ro = new ResizeObserver(() => fixRef.current());
    ro.observe(c); ro.observe(sc);
    relayout = () => fixRef.current();
    away.go = () => { tl.follow = true; tl.anchor = null; again(); };
    return () => { tl.subs.delete(onChange); sc.removeEventListener("scroll", onScroll); sc.removeEventListener("wheel", onWheel); sc.removeEventListener("keydown", onKey); sc.removeEventListener("pointerdown", onDown); ro.disconnect(); for (let i = 0; i < tl.rows.length; i++) (tl.rows[i] as any).el = undefined; relayout = () => {}; setAway(false); };
  }, [tl]);
  const out = [];
  for (let i = cur.a; i <= cur.b; i++) out.push(<RowView key={rows[i].id} row={rows[i]} />);
  return (
    <div class="col" ref={col}>
      {cur.a > 0 && <div key="top" aria-hidden="true" style={{ height: cur.top + "px" }} />}
      {out}
      {cur.b < n - 1 && <div key="bot" aria-hidden="true" style={{ height: cur.bot + "px" }} />}
    </div>
  );
}

export function Timeline() {
  useTick();
  const tl = frontTl();
  return <div class="scroll" id="scroll">{tl && <List key={tl.id} tl={tl} />}</div>;
}

/* the way back to the end of a chat the reader has scrolled up in */
/* `tab`: a tab standing on the message box's top edge (tab.tsx), the box's line running round it; without the box (an
   expert's chat, read only) it floats over the dock as a pill. The tab has no glass of its own: the box's glass
   (.composer::before) reaches up over it, cut to the box and the tab as one shape (boxAndTab), so the blur behind them
   is one and the same. The shape is kept on the box as --cm-clip, follows the box's size, and morphs as the tab grows */
/* the tab's size, and how far in from the box's right edge it stands (as the chat bar's tab is from the left) */
const TL_W = 148, TL_H = 16, TL_IN = 24;
const n = (v: number) => +v.toFixed(2);
/* the glass's shape: the box (w by h, corners r) and the tab on it, k grown (0 to 1), as one outline. The glass starts
   TL_H above the box's outer edge, so the box starts at TL_H and the tab (standing on the box's inner edge, 1px lower)
   at 1, its foot overlapping the box's top row. Each part runs the same way round, so where they overlap they add up */
function boxAndTab(w: number, h: number, r: number, k: number) {
  const B = TL_H, E = B + h;
  const box = `M0 ${n(B + r)}V${n(E - r)}A${r} ${r} 0 0 0 ${n(r)} ${E}H${n(w - r)}A${r} ${r} 0 0 0 ${w} ${n(E - r)}V${n(B + r)}A${r} ${r} 0 0 0 ${n(w - r)} ${B}H${n(r)}A${r} ${r} 0 0 0 0 ${n(B + r)}Z`;
  return "path('" + box + tabPaths(TL_W, TL_H, true, { dx: n(w - 1 - TL_IN - TL_W), dy: 1, k }).fill + "')";
}
export function ToLatest({ tab }: { tab?: boolean }) {
  const again = useAgain(), me = useRef<HTMLButtonElement>(null), was = useRef(away.on), sinking = useRef(false), timer = useRef(0), edge = useRef<SVGPathElement>(null);
  useEffect(() => { away.subs.add(again); return () => { away.subs.delete(again); }; }, []);
  /* the box's glass shape: kept to the box's size, and morphing (for as long as the tab grows or sinks) when it shows */
  useLayoutEffect(() => {
    const box = tab && me.current && (me.current.parentElement as HTMLElement | null);
    if (!box) return;
    const put = () => {
      const r = parseFloat(getComputedStyle(box).borderTopLeftRadius) || 0;
      const w = box.offsetWidth, h = box.offsetHeight, x1 = n(w - 1 - TL_IN), x0 = n(x1 - TL_W);
      box.style.setProperty("--cm-clip", boxAndTab(w, h, r, away.on ? 1 : 0.001));
      /* the box's outline is one drawn line (not a border, which the screen rounds to whole pixels differently from the
         tab's drawn outline, so the two never quite met): round the box, open where the tab stands at once as it comes,
         and closed again only once it has sunk flat as it goes */
      const q = 0.5, R = n(Math.max(0, r - q)), open = away.on || sinking.current;
      if (edge.current) edge.current.setAttribute("d", `M${open ? x0 : x1} ${q}H${n(q + R)}A${R} ${R} 0 0 0 ${q} ${n(q + R)}V${n(h - q - R)}A${R} ${R} 0 0 0 ${n(q + R)} ${n(h - q)}`
        + `H${n(w - q - R)}A${R} ${R} 0 0 0 ${n(w - q)} ${n(h - q - R)}V${n(q + R)}A${R} ${R} 0 0 0 ${n(w - q - R)} ${q}H${x1}${open ? "" : "Z"}`);
      /* and the tab goes from view in the same moment the line closes, never before or after: the two lines are never
         drawn over each other (which shows as a brighter piece of line), nor is there a gap with neither */
      if (me.current) me.current.classList.toggle("gone", !open);
    };
    if (was.current !== away.on) {
      /* one timer: scrolling to and fro quickly, an older one must not end this grow or sink early */
      was.current = away.on; sinking.current = !away.on; box.classList.add("tl-morph"); clearTimeout(timer.current);
      timer.current = window.setTimeout(() => { box.classList.remove("tl-morph"); if (sinking.current) { sinking.current = false; put(); } }, 260);
    }
    put();
    const ro = new ResizeObserver(put); ro.observe(box);
    return () => ro.disconnect();
  });
  return (
    <>
    {tab && <svg class="cm-edge" aria-hidden="true"><path ref={edge} /></svg>}
    <button ref={me} class={"to-latest" + (tab ? " tl-tab" : "") + (away.on ? " on" : "")} tabIndex={away.on ? 0 : -1} aria-hidden={away.on ? undefined : "true"} title="Go to the newest message" onClick={() => away.go()}>
      {tab && <TabShape w={TL_W} h={TL_H} up />}
      <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M5 1.5v7M1.5 5.5 5 9l3.5-3.5" /></svg><span class="tl-t">Jump to latest</span>
    </button>
    </>
  );
}
