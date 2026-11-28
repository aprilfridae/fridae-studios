/* A tab's outline, shared by the chat bar's pull tab (Head.tsx, hanging down from the line under the top bar), Jump to
   latest (Timeline.tsx, standing up on the message box's top edge) and the Overview's pull-down bar: each side one smooth
   curve out of the line it joins, round into the far edge. `fill` covers the line where the tab joins it (so the line runs round
   the tab); `line` is the outline alone, open along the joined edge. w and h in pixels; up: the tab stands on its line. */
export function tabPaths(w: number, h: number, up = false, at: { dx?: number; dy?: number; k?: number } = {}) {
  /* at: moved by dx, dy; k, how far the tab has grown out of its line (0 to 1: 0 is flat on the line) */
  const dx = at.dx || 0, dy = at.dy || 0, k = at.k === undefined ? 1 : at.k;
  const y = (v: number) => +(dy + (up ? h - v * k : v * k)).toFixed(2), r = (v: number) => +(dx + w - v).toFixed(2), x = (v: number) => +(dx + v).toFixed(2);
  /* each side is one smooth curve, level where it leaves the line and level where it meets the far edge, s across */
  const S = Math.min(14, w / 4), c = S / 2, b = h - 0.5;
  const fill = `M${x(0)} ${y(0)}H${r(0)}V${y(1)}C${r(c)} ${y(1)} ${r(c)} ${y(h)} ${r(S)} ${y(h)}H${x(S)}C${x(c)} ${y(h)} ${x(c)} ${y(1)} ${x(0)} ${y(1)}Z`;
  const line = `M${x(0)} ${y(0.5)}C${x(c)} ${y(0.5)} ${x(c)} ${y(b)} ${x(S)} ${y(b)}H${r(S)}C${r(c)} ${y(b)} ${r(c)} ${y(0.5)} ${r(0)} ${y(0.5)}`;
  /* the flat piece of line the tab stands on, where it leaves the line it joins */
  const base = `M${x(0)} ${y(0.5)}H${r(0)}`;
  return { fill, line, base };
}
/* the outline drawn: the fill in the tab's colour (.f), the line (.s), and standing up, the flat line under it (.b) */
export function TabShape({ w, h, up }: { w: number; h: number; up?: boolean }) {
  const t = tabPaths(w, h, up);
  return <svg class="tab-shape" width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true"><path class="f" d={t.fill} /><path class="s" d={t.line} />{up && <path class="b" d={t.base} />}</svg>;
}
