/* The pop-up at the bottom left: each account's limits, small, today's total and what each AI used. The old page
   opens and closes it (its button is in the sidebar). */
import { useStore } from "../store";
import { host } from "../host";
import { aiName } from "../ai";
import { More } from "../ui/More";
import { LimitWindows, useNow } from "./Bars";
import { checkedLine, limitAccounts, quietLimits } from "./limits";
import { fmtTokens } from "../util";

export function UsagePop() {
  const status = useStore((s) => s.status), ais = useStore((s) => s.ais), u = useStore((s) => s.usage), now = useNow();
  const accounts = limitAccounts(status), quiet = quietLimits(accounts, ais);
  return (
    <>
      <div class="pop-head"><h5>Plan limits</h5><button class="linkish" id="use-open" onClick={() => host.openUsage()}>Open Usage</button></div>
      <div id="use-limits">
        {accounts.length ? accounts.map((a) => (
          <div class="pop-acct"><b>{a.label}</b>{a.windows.length > 0 && <LimitWindows windows={a.windows} now={now} compact />}
            {a.error ? <small>{a.error}</small> : !a.windows.length ? <small>No limits reported.</small> : null}
            {a.checked ? <small>{checkedLine(a) + "."}</small> : null}</div>
        )) : <div class="u-row"><small>Shown after Claude’s next turn: it reports them each time it answers.</small></div>}
        {quiet && <p class="u-note">{quiet}</p>}
      </div>
      <h5>Used today</h5>
      <p class="u-total" id="use-total"><b>{u ? (u.first ? "…" : fmtTokens(u.all.total - u.all.cached)) : "0"}</b> tokens today</p>
      <div id="use-rows">
        {u && u.rows.length ? u.rows.map(([id, , shown, said]) => <div class="u-row"><b>{aiName(id)}</b><span class="n">{shown}</span><small>{said}</small></div>)
          : u ? <div class="u-row"><small>Nothing yet today.</small></div> : null}
      </div>
      <More word="How this is counted">New tokens, as each AI reports them: a guide, not a bill. Tokens read, written and stored in cache are counted after every turn. Re-reads from cache are listed but not counted: they cost a fraction. Plans are limited by their own rules.</More>
    </>
  );
}
