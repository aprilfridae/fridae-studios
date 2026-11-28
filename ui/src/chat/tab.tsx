/* A tab's outline, shared by the chat bar's pull tab (Head.tsx, hanging down from the line under the top bar) and
   Jump to latest (Timeline.tsx, standing up on the message box's top edge): sides that flare out into the line it
   joins, rounded where they meet the far edge. `fill` covers the line where the tab joins it (so the line runs round
   the tab); `line` is the outline alone, open along the joined edge. w and h in pixels; up: the tab stands on its line. */
export function tabPaths(w: number, h: number, up = false, at: { dx?: number; dy?: number; k?: number } = {}) {
  /* at: moved by dx, dy; k, how far the tab has grown out of its line (0 to 1: 0 is flat on the line) */
  const dx = at.dx || 0, dy = at.dy || 0, k = at.k === undefined ? 1 : at.k;
  const y = (v: number) => +(dy + (up ? h - v * k : v * k)).toFixed(2), r = (v: number) => +(dx + w - v).toFixed(2), x = (v: number) => +(dx + v).toFixed(2);
  const fill = `M${x(0)} ${y(0)}H${r(0)}V${y(1)}C${r(6)} ${y(1)} ${r(7.8)} ${y(2.4)} ${r(8.8)} ${y(6)}L${r(11.7)} ${y(h - 5.6)}C${r(12.6)} ${y(h - 2.4)} ${r(14.4)} ${y(h - 1)} ${r(17.8)} ${y(h - 1)}`
    + `H${x(17.8)}C${x(14.4)} ${y(h - 1)} ${x(12.6)} ${y(h - 2.4)} ${x(11.7)} ${y(h - 5.6)}L${x(8.8)} ${y(6)}C${x(7.8)} ${y(2.4)} ${x(6)} ${y(1)} ${x(0)} ${y(1)}Z`;
  const line = `M${x(0)} ${y(0.5)}C${x(6)} ${y(0.5)} ${x(7.8)} ${y(2)} ${x(8.8)} ${y(5.8)}L${x(11.7)} ${y(h - 5.7)}C${x(12.6)} ${y(h - 2.4)} ${x(14.4)} ${y(h - 0.5)} ${x(17.8)} ${y(h - 0.5)}`
    + `H${r(17.8)}C${r(14.4)} ${y(h - 0.5)} ${r(12.6)} ${y(h - 2.4)} ${r(11.7)} ${y(h - 5.7)}L${r(8.8)} ${y(5.8)}C${r(7.8)} ${y(2)} ${r(6)} ${y(0.5)} ${r(0)} ${y(0.5)}`;
  return { fill, line };
}
/* the outline drawn: the fill in the tab's colour (.f), and the line (.s) */
export function TabShape({ w, h, up }: { w: number; h: number; up?: boolean }) {
  const t = tabPaths(w, h, up);
  return <svg class="tab-shape" width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true"><path class="f" d={t.fill} /><path class="s" d={t.line} /></svg>;
}
