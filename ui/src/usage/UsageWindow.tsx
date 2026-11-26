/* The Usage window: every account's plan limits as bars ("% left", the even-pace line, when it starts again, when it
   was last checked), then today's tokens, by AI and in total. The old page shows and hides the window. */
import { useState } from "preact/hooks";
import { useStore, type Sum } from "../store";
import { host } from "../host";
import { closeUsage } from "../windows";
import { aiName } from "../ai";
import { More } from "../ui/More";
import { X } from "../ui/icons";
import { ago, fmtTokens } from "../util";
import { LimitWindows, useNow } from "./Bars";
import { checkedLine, limitAccounts, quietLimits } from "./limits";
import { loadUsage } from "./usage";

const NONE: Sum = { total: 0, input: 0, output: 0, cached: 0, cost: 0, turns: 0, quiet: 0 };

export function UsageWindow() {
  const status = useStore((s) => s.status), ais = useStore((s) => s.ais), usage = useStore((s) => s.usage), now = useNow();
  useStore((s) => s.usageOpen);   /* "checked 5 min ago" is worded afresh each time the window opens */
  const [busy, setBusy] = useState(false);
  const accounts = limitAccounts(status), newest = Math.max(0, ...accounts.map((a) => a.checked));
  const u = usage || { all: NONE, rows: [], first: false }, all = u.all;
  const most = Math.max(1, ...u.rows.map(([, s]) => s.total - s.cached));
  const again = async () => { setBusy(true); try { await Promise.all([host.refresh(), loadUsage()]); } finally { setBusy(false); } };
  const metric = (label: string, value: string) => <div><dt>{label}</dt><dd>{value}</dd></div>;
  return (
    <div class="st-main">
      <header class="st-head">
        <nav aria-label="Usage breadcrumb"><ol class="st-crumb"><li aria-current="page">Usage</li></ol></nav>
        <span class="st-head-note" id="us-checked">{newest ? "Limits checked " + ago(newest) : ""}</span>
        <span class="grow"></span>
        <button class={"icon" + (busy ? " busy" : "")} id="us-refresh" aria-label="Check again" title="Check again" onClick={again}><svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9" /><path d="M13.5 2.5v3h-3" /></svg></button>
        <button class="icon su-close" id="us-close" aria-label="Close" onClick={() => closeUsage()}>{X}</button>
      </header>
      <div class="su-body" id="us-body"><div class="st-page wide us-page">
        <section class="us-sec" aria-labelledby="us-h-limits">
          <h2 id="us-h-limits">Plan limits</h2>
          <div class="us-accounts" id="us-limits">
            {accounts.length ? accounts.map((a) => (
              <article class="us-acct"><header><h3>{a.label}</h3><span>{checkedLine(a)}</span></header>
                {a.windows.length > 0 && <LimitWindows windows={a.windows} now={now} />}
                {a.error ? <p class="us-err"><svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="8" cy="8" r="5.5" /><path d="M8 7.5V11M8 5v.1" /></svg><span>{a.error}</span></p>
                  : !a.windows.length ? <p class="us-note">No limits reported.</p> : null}</article>
            )) : <p class="us-note">Shown after Claude’s next turn: it reports them each time it answers.</p>}
          </div>
          <p class="us-note" id="us-quiet">{quietLimits(accounts, ais)}</p>
        </section>
        <section class="us-today" aria-labelledby="us-h-today">
          <div class="us-total">
            <h2 id="us-h-today">Used today</h2>
            <p class="us-big" id="us-total">{u.first ? "…" : fmtTokens(all.total - all.cached)}</p>
            <p class="us-note" id="us-turns">{"New tokens · " + all.turns + (all.turns === 1 ? " turn" : " turns") + (all.quiet ? " · " + all.quiet + " not reported" : "")}</p>
          </div>
          <div class="us-ais" id="us-ais">
            {u.rows.length ? u.rows.map(([id, s, shown, said]) => (
              <div class="us-ai"><div><b>{aiName(id)}</b><span class="n">{shown}</span></div><span class="us-share" aria-hidden="true"><i style={"width: " + Math.round(Math.max(0, s.total - s.cached) / most * 100) + "%"}></i></span><small>{said}</small></div>
            )) : <p class="us-note">Nothing yet today.</p>}
          </div>
        </section>
        <section class="us-sec" aria-labelledby="us-h-totals">
          <h2 id="us-h-totals">Totals</h2>
          <dl class="us-metrics" id="us-metrics">
            {metric("New tokens", fmtTokens(all.total - all.cached))}{metric("Read", fmtTokens(all.input))}{metric("Written", fmtTokens(all.output))}
            {metric("Read from cache", fmtTokens(all.cached))}{metric("About, at API prices", all.cost ? "$" + all.cost.toFixed(2) : "–")}
          </dl>
          <p class="us-note">New tokens, as each AI reports them. A guide, not a bill.</p>
          <More>Tokens read, written and stored in cache are counted after every turn. Re-reads from cache are listed but not counted: they cost a fraction. Plans are limited by their own rules.</More>
        </section>
      </div></div>
    </div>
  );
}
