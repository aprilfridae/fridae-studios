/* Which of its AI's accounts a chat runs on, picked in the model picker (Composer.tsx), under the effort: only Claude
   and Gemini can have more than one (Settings, Connections adds them). The menu lists the user's
   own account and each added one that is set up (status.ai.accounts[ai].accounts[].ready: its folder is the app's and an
   account of its own is signed in there). A pick counts from the chat's next message (/api/chat-account); the chat
   stays on it, and still moves on by itself when that account runs out of usage. On the empty page the pick waits
   for the chat the first message makes (C.draftAccount, sendFromDraft). Not for a chat on another computer: the
   accounts listed are this computer's. */
import { api } from "../api";
import { H } from "./host";
import { poke } from "./chatStore";
import { C } from "./compose";
import { boxMachine } from "../remote/remote";

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
  return "Out of usage, back in about " + (mins < 90 ? mins + " min" : Math.round(mins / 60) + " h");
};

/* the accounts to offer for a chat of `ai`, and which it is on, or null when there is nothing to pick: on another
   computer (its accounts are not this one's), in an expert's chat, or with one account and the chat not on an added one */
export function accountChoice(p: any, ai: string): { list: Acct[]; n: number } | null {
  if (boxMachine() || H.isHelper(p)) return null;
  const list = pickable(ai);
  /* which account it is on: a pick that waits for the next message, else the one it runs on; before its first
     message, nothing is decided yet (0: the user's own unless that one is out of usage) */
  const draft = !p && C.draftAccount && C.draftAccount.ai === ai ? list.find((a) => a.name === C.draftAccount!.name) : null;
  const n: number = p ? p.accountPick || p.account || (p.started ? 1 : 0) : draft ? draft.n : 0;
  return list.length < 2 && n < 2 ? null : { list, n };
}

/* the picker's account part: a heading, one row per account (a tick on the one in use), and a line on how it works */
export function AccountSection({ p, ai }: { p: any; ai: string }) {
  const got = accountChoice(p, ai);
  if (!got) return null;
  const { list, n } = got;
  const choose = async (a: Acct) => {
    if (!p) { C.draftAccount = { ai, name: a.name }; poke(); return; }
    const r = await api("/api/chat-account", { agent: p.id, account: a.name }).catch(() => ({ ok: false }));
    if (!r.ok) H.say(r.error || "Couldn't switch the account. Try again in a moment.");
  };
  const missing = notReady(ai);
  return (
    <div class="pk-acct">
      <div class="pk-eh"><span>Account</span><em>{n ? title(n).replace("Your own account", "Yours") : "Auto"}</em></div>
      <div class="pk-er-list" id="account-pick" role="radiogroup" aria-label={"Which " + H.aiName(ai) + " account it uses"}>
        {list.map((a) => {
          const small = [a.who, usedLine(a.used)].filter(Boolean).join(" · ");
          return (
            <button class="pk-er" role="radio" data-account={a.name} aria-checked={a.n === n ? "true" : "false"} title={(small ? small + ". " : "") + "From your next message."} onClick={() => choose(a)}>
              <span><b>{title(a.n).replace("Your own account", "Your account")}</b>{small && <small>{small}</small>}</span>{TICK}
            </button>
          );
        })}
      </div>
      <p class="ac-note">{"From your next message. If it runs out of usage, the chat moves on to your next account by itself."
        + (missing.length ? " " + (missing.length === 1 ? "Account " + missing[0] + " isn't" : "Accounts " + missing.join(", ") + " aren't") + " signed in yet: Settings, Connections." : "")}</p>
    </div>
  );
}
