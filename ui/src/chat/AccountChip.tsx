/* The account chip in the message box, beside the AI chip: which of its AI's accounts a chat runs on, and a menu to pick
   another (only Claude and Gemini can have more than one: Settings, Connections adds them). The menu lists the user's
   own account and each added one that is set up (status.ai.accounts[ai].accounts[].ready: its folder is the app's and an
   account of its own is signed in there). A pick counts from the chat's next message (/api/chat-account); the chat
   stays on it, and still moves on by itself when that account runs out of usage. On the empty page the pick waits
   for the chat the first message makes (C.draftAccount, sendFromDraft). Not for a chat on another computer: the
   accounts listed are this computer's. */
import { api } from "../api";
import { H } from "./host";
import { poke } from "./chatStore";
import { C, closePops, togglePop } from "./compose";
import { boxMachine } from "../remote/remote";

const PERSON = <svg class="ac-icon" width="13" height="13" viewBox="0 0 13 13" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="6.5" cy="4.5" r="2.25" /><path d="M2.25 11.25c.6-2.1 2.3-3.25 4.25-3.25s3.65 1.15 4.25 3.25" /></svg>;
const DOWN = <svg class="down" width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M4.5 6.5 8 10l3.5-3.5" /></svg>;
const TICK = <span class="tick"><svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M1.5 5.5l2.5 2.5 4.5-5.5" /></svg></span>;

export type Acct = { name: string; n: number; who: string; used: number | null };
const accountsOf = (ai: string): any => (((H.status || {}).ai || {}).accounts || {})[ai] || {};
const signInOf = (id: string): any => (((H.status || {}).ai || {}).sign_in || {})[id] || {};

/* the accounts a chat of `ai` can be put on: the user's own first, then each added one that is set up, in their order */
export function pickable(ai: string): Acct[] {
  const s = accountsOf(ai);
  const out: Acct[] = [{ name: "", n: 1, who: signInOf(ai).who || "", used: s.used || null }];
  for (const a of s.accounts || []) if (a.ready) out.push({ name: a.name, n: a.n, who: signInOf(a.id).who || "", used: a.used || null });
  return out;
}
/* added accounts that are not set up yet: named under the menu, never offered */
const notReady = (ai: string): number[] => (accountsOf(ai).accounts || []).filter((a: any) => !a.ready).map((a: any) => a.n);

/* an account as the chat names it: in a reply's label ("account 2", "your account"), and in the chip */
export const accountWord = (n: number) => (n >= 2 ? "account " + n : "your account");
const title = (n: number) => (n >= 2 ? "Account " + n : "Your own account");
const usedLine = (until: number | null) => {
  if (!until) return "";
  const mins = Math.max(1, Math.ceil((until - Date.now() / 1000) / 60));
  return "Out of usage for now, back in about " + (mins < 90 ? mins + " minutes" : Math.round(mins / 60) + " hours");
};

export function AccountChip({ p, ai }: { p: any; ai: string }) {
  if (boxMachine() || H.isHelper(p)) return null;
  const list = pickable(ai), open = C.pop === "account";
  /* which account it is on: a pick that waits for the next message, else the one it runs on; before its first
     message, nothing is decided yet ("Auto": the user's own unless that one is out of usage) */
  const draft = !p && C.draftAccount && C.draftAccount.ai === ai ? list.find((a) => a.name === C.draftAccount!.name) : null;
  const n: number = p ? p.accountPick || p.account || (p.started ? 1 : 0) : draft ? draft.n : 0;
  /* only while there is a choice, or the chat is on an added account */
  if (list.length < 2 && n < 2) return null;
  const choose = async (a: Acct) => {
    closePops(); document.getElementById("account")?.focus();
    if (!p) { C.draftAccount = { ai, name: a.name }; poke(); return; }
    const r = await api("/api/chat-account", { agent: p.id, account: a.name }).catch(() => ({ ok: false }));
    if (!r.ok) H.say(r.error || "Couldn't switch the account. Try again in a moment.");
  };
  const label = n ? title(n).replace("Your own account", "Your account") : "Auto";
  const missing = notReady(ai);
  return (
    <span class="pick">
      <button class="chip ac" id="account" aria-haspopup="menu" aria-expanded={open ? "true" : "false"}
        title={(n ? "Runs on " + H.aiName(ai) + " " + accountWord(n) : "Starts on your own " + H.aiName(ai) + " account, or the next one if yours is out of usage") + ". Press to pick which account it uses from your next message."}
        onClick={(ev) => { ev.stopPropagation(); togglePop("account"); }}>{PERSON}<span>{label}</span>{DOWN}</button>
      <div class={"menu" + (open ? " on" : "")} id="account-menu" role="menu" aria-label={"Which " + H.aiName(ai) + " account it uses"}>
        <h5>{H.aiName(ai) + " account"}</h5>
        {list.map((a) => {
          const small = [a.who, usedLine(a.used)].filter(Boolean).join(" · ");
          return (
            <button class="opt" role="menuitemradio" data-account={a.name} aria-checked={a.n === n ? "true" : "false"} onClick={() => choose(a)}>
              {TICK}<span><b>{title(a.n)}</b>{small && <small>{small}</small>}</span>
            </button>
          );
        })}
        <p class="ac-note">{"From your next message. If it runs out of usage, the chat moves on to your next account by itself."
          + (missing.length ? " " + (missing.length === 1 ? "Account " + missing[0] + " isn't" : "Accounts " + missing.join(", ") + " aren't") + " signed in yet: Settings, Connections." : "")}</p>
      </div>
    </span>
  );
}
