/* Today's tokens: POST /api/usage since midnight, summed for the day and for each AI that ran, kept in the store for
   the pop-up at the bottom left and the Usage window. */
import { api } from "../api";
import { store, type Sum } from "../store";
import { host } from "../host";
import { aiName } from "../ai";
import { fmtTokens } from "../util";

const turnsText = (n: number) => n + (n === 1 ? " turn" : " turns");
/* `quiet`: turns the AI ran without reporting what they used (Cursor on its kept connection). They are said as such, never counted as zero. */
const sum = (list: any[]): Sum => list.reduce((a, x) => ({ total: a.total + (x.total || 0), input: a.input + (x.input || 0), output: a.output + (x.output || 0), cached: a.cached + (x.cache_read || 0), cost: a.cost + (x.cost || 0), turns: a.turns + 1, quiet: a.quiet + (x.unreported ? 1 : 0) }), { total: 0, input: 0, output: 0, cached: 0, cost: 0, turns: 0, quiet: 0 });
const shown = (s: Sum) => (s.quiet === s.turns ? "–" : fmtTokens(s.total - s.cached));
const said = (id: string, s: Sum) => [
  s.turns > s.quiet ? turnsText(s.turns - s.quiet) + " · " + fmtTokens(s.input) + " in, " + fmtTokens(s.output) + " out" + (s.cached ? ", " + fmtTokens(s.cached) + " read from cache" : "") + (s.cost ? " · about $" + s.cost.toFixed(2) + " at API prices" : "") : "",
  s.quiet ? turnsText(s.quiet) + " not reported by " + aiName(id) + " over this connection" : "",
].filter(Boolean).join(" · ");

export async function loadUsage() {
  const day = new Date(); day.setHours(0, 0, 0, 0);
  const today = day.getTime() / 1000;
  let r; try { r = await api("/api/usage", { since: today }); } catch (e) { return; }
  const now = ((r && r.records) || []).filter((x: any) => x.t >= today), all = sum(now);
  /* nothing counted yet while the day's first turn runs: "…", not a "0" that looks like nothing is happening */
  const first = !now.length && !!host.anyBusy();
  /* every AI that ran today, the ones offered here first (an expert may run on one that is not) */
  const ran = [...new Set(store.get().ais.map((a) => a[0]).concat(now.map((x: any) => x.ai).filter(Boolean)))];
  const rows = ran.map((id) => [id, sum(now.filter((x: any) => x.ai === id))] as [string, Sum]).filter(([, s]) => s.turns);
  store.set({ usage: { all, first, rows: rows.map(([id, s]) => [id, s, shown(s), said(id, s)]) } });
}
/* after a turn: soon, once, however many asked */
let timer = 0;
export function loadUsageSoon() { clearTimeout(timer); timer = setTimeout(loadUsage, 400) as unknown as number; }
