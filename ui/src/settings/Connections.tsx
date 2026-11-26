/* Settings, Connections: each AI with whether it is found on this computer, who is signed in and what it needs from
   you; more accounts; Fridae's own OpenCode sign-in; GitHub; Roblox Studio and its plugin; Blender; the promo engine;
   the full check. */
import { useRef, useState } from "preact/hooks";
import { api } from "../api";
import { cell, store, useStore } from "../store";
import { host } from "../host";
import { ghAct } from "../project/github";
import { engineInstall } from "../promos/engine";
import { accError, accountOf, accountsOf, noticeOf, openCodeSignIn, pressAccount, pressOpenCode, pressSignIn, signInLine, signInOf } from "../ai";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Switch } from "../ui/Switch";
import { Press } from "../ui/Press";
import { DotRow, ListRow } from "../ui/Row";

/* one Add or Remove at a time: the rows move up when one is removed, so a second click must never land on the next
   account's Remove while the first is still on its way (and for a moment after, for a double click's second half) */
const accBusy = cell(false);
async function account(ai: string, action: string, name: string) {
  if (accBusy.get()) return;
  accBusy.set(true);
  try { await pressAccount(ai, action, name); } finally { accBusy.poke(); setTimeout(() => accBusy.set(false), 600); }
}
const SignIn = ({ ai, label, action, busy }: { ai: string; label: string; action: string; busy?: boolean }) =>
  <Press disabled={busy} onPress={() => (ai === "opencode" ? pressOpenCode(action) : pressSignIn(ai, action))}>{label}</Press>;

/* what this AI needs from you now, under its row: one line and its buttons */
function Notice({ ai }: { ai: string }) {
  const n = noticeOf(ai);
  return <div class="notice">{n && <span class="nt"><span>{n.text}</span>{n.buttons.map(([label, action]) => <SignIn ai={ai} label={label} action={action} />)}</span>}</div>;
}
/* Fridae's own OpenCode sign-in, under OpenCode's row: where it stands, in one short line (what to do about it is the
   notice above it), and Sign in again once it is signed in. */
function OpenCodeRow({ busy }: { busy: boolean }) {
  const s = openCodeSignIn();
  const what = s.signed.length ? "Signed in: " + s.signed.join(", ") + "." : s.pending ? "Signed in." : s.known ? "Not signed in. Your own OpenCode is never changed." : "Checking the sign-in...";
  return <ListRow cls="oc-row" name="Fridae's OpenCode" line={what}><div class="act">{s.signed.length > 0 && !s.buttons.length && <SignIn ai="opencode" label="Sign in again" action="login" busy={busy} />}</div></ListRow>;
}
/* More accounts, under the AI's own row (which is account 1): one line per added account with its sign-in (the same
   line and buttons as any sign-in) or who is signed in, and Remove; then one line to add another. Your own account is
   never changed from here. */
function AccountRows({ ai, busy }: { ai: string; busy: boolean }) {
  const acc = accountOf(ai), list = accountsOf(ai);
  const left = (acc.left || []).length ? "A removed account's folder could not be deleted and may still hold its sign-in. Delete it yourself: " + acc.left.join(", ") + " " : "";
  return (
    <>
      {list.map((a) => {
        const id = a.id, s = signInOf(id), line = signInLine(id), waiting = (s.login || {}).state === "waiting";
        const text = a.changed ? "Fridae has stopped using this account: its keychain settings were changed from outside Fridae. Check that your own Gemini sign-in still works, then remove this account and add it again."
          : a.broken ? "Fridae does not use this account: its folder was changed from outside, or its keychain could not be opened. Remove it and add it again."
          : line.text ? line.text
          : s.state === "in" && s.same ? "Signed in as " + s.who + ", the same account as one above it, so it adds no usage. Remove it, sign out of that account in your browser, and add another."
          : s.state === "in" && s.by_key ? "A key or token on this computer signs it in, not an account of its own, so chats do not move to it."
          : s.state === "in" ? "Signed in" + (s.who ? " as " + s.who : "") + "." + (a.used ? " Out of usage for now." : "")
          : s.checked ? "Could not check this account just now. Fridae tries again every few minutes."
          : "Checking the sign-in...";
        return (
          <ListRow cls="oc-row" name={"Account " + a.n} line={text}><div class="act">
            {/* (an account the app will not use offers no sign-in: only Remove) */}
            {!a.broken && !a.changed && line.buttons.map(([label, action]) => <SignIn ai={id} label={label} action={action} busy={busy} />)}
            {!waiting && <button class="pill sm" disabled={busy} onClick={() => account(ai, "remove", a.name)}>Remove</button>}
          </div></ListRow>
        );
      })}
      <ListRow cls="oc-row" name={list.length ? "Another account" : "More accounts"} line={accError[ai] || left + (list.length ? "Add as many as you have. They are used in this order, your own first." : "None. Add one to carry on when your account runs out of usage.")}>
        <div class="act">{list.length < (acc.most || 20) && <button class="pill sm" disabled={busy} onClick={() => account(ai, "add", "")}>Add account</button>}</div>
      </ListRow>
    </>
  );
}
function Ais() {
  const status = useStore((s) => s.status), ais = useStore((s) => s.ais), busy = accBusy.use();
  const ai = status.ai || {}, have = ai.installed || {};
  if (!("ai" in status)) return <div class="sg-list" id="ai-list"></div>;
  return (
    <div class="sg-list" id="ai-list">
      {ais.map(([id, name, via, how]) => {
        const si = signInOf(id), signedIn = si.offered && si.state === "in" ? " Signed in" + (si.who ? " as " + si.who : "") + "." : "";
        return [
          <DotRow ai={id} dot={have[id] ? "ok" : ""} name={name + " · " + via}><small>{have[id] ? "Found on this computer." + signedIn : "Not found. " + how}</small><Notice ai={id} /></DotRow>,
          id === "opencode" && have[id] ? <OpenCodeRow busy={busy} /> : null,
          have[id] && accountOf(id).offered ? <AccountRows ai={id} busy={busy} /> : null,
        ];
      })}
    </div>
  );
}
/* GitHub's row, laid out like the AIs' rows. Fridae keeps the sign-in to itself: agents never get it, and nothing is
   sent to GitHub unless you press a button in a project. Connecting is a one-time code entered on GitHub. */
function GitHub() {
  const gh = useStore((s) => s.gh), note = useStore((s) => s.ghNote), busy = useStore((s) => s.ghBusy), [copied, setCopied] = useState("");
  if (!gh) return <div class="sg-list" id="gh-row"></div>;
  const p = gh.pending, act = (label: string, f: () => void, off?: boolean) => <button class="pill sm" disabled={busy || off} onClick={f}>{label}</button>;
  const copy = async () => {
    let ok = false; try { ok = !!(await api("/api/copy", { text: p.user_code })).ok; } catch (e) {}
    setCopied(ok ? "Copied" : "Could not copy"); setTimeout(() => setCopied(""), 1500);
  };
  return (
    <div class="sg-list" id="gh-row">
      <DotRow dot={gh.connected ? "ok" : p ? "live" : ""} name="GitHub">
        {gh.connected ? <small>{gh.avatar_url && /^https:\/\//.test(gh.avatar_url) && <img class="gh-avatar" alt="" src={gh.avatar_url} />}{"Connected as " + (gh.login || "your GitHub account") + ". Fridae only uses it when you press Put on GitHub or Push now in a project."}</small>
          : <small>{p ? "Enter this code on GitHub's page, then press Authorize. Fridae carries on by itself once you have."
            : gh.configured === false ? "GitHub sign-in isn't set up in this copy of Fridae yet."
            : "Not connected. Connect to put your projects on GitHub and send your work there when you press a button. Fridae asks GitHub to let it make and update your repositories (private ones too) and to see your username."}</small>}
        <div class="gh-code" hidden={gh.connected || !p}>{!gh.connected && p ? p.user_code : ""}</div>
        {/* what an AI could do with it, said plainly: in Full auto an AI runs commands as you */}
        <small class="gh-note">{gh.configured !== false ? "In Full auto, an AI runs commands as you and could read this sign-in." : ""}</small>
        <div class="notice">{note || gh.error ? note || gh.error : gh.forgot ? <><span>Fridae forgot your sign-in. To fully remove access, revoke Fridae Studios on GitHub.</span>{act("Open GitHub settings", () => api("/api/github/open", { what: "settings" }))}</> : null}</div>
        <div class="act">
          {gh.connected ? act("Disconnect", () => ghAct("disconnect"))
            : p ? <>{act(copied || "Copy", copy)}{act("Open GitHub", () => api("/api/github/open", { what: "device" }))}{act("Cancel", () => ghAct("cancel"))}</>
            : gh.configured === false ? act("Connect", () => {}, true)
            : act("Connect", () => ghAct("connect"))}
        </div>
      </DotRow>
    </div>
  );
}
/* The Studio plugin, as one line. `note`: what the last press of Install or Remove said, shown until the app's status
   is asked again. */
const plugNote = cell<{ text: string; at: number }>({ text: "", at: -1 });
function pluginLine(pl: any, note: string) {
  if (note) return note;
  if (!pl.listening) return "Another copy of Fridae is already linked to Studio, so this window is not.";
  if (pl.connected) return "Connected" + (pl.place && pl.place.name ? " to " + pl.place.name : "") + "." + (pl.running_version && pl.running_version !== pl.wanted_version ? " Studio runs an older copy: press Update plugin, then restart Studio." : "");
  if (pl.installed) return "Installed" + (pl.current ? "" : " (an older copy: press Update plugin)") + ", not connected. " + (pl.studio_running ? "Restart Studio so it loads the plugin, and allow it to talk to this app." : "It connects when Studio is open.");
  return "Not installed. It tells the agent what you have selected, and lets you undo a turn.";
}
const Need = ({ ok, name, what }: { ok: boolean; name: string; what: string }) => <DotRow dot={ok ? "ok" : ""} name={name}><small>{what}</small></DotRow>;
function Studio() {
  const status = useStore((s) => s.status), polls = useStore((s) => s.polls), note = plugNote.use(), pl = status.plugin || {};
  const known = status.studio !== undefined, said = (text: string) => plugNote.set({ text, at: store.get().polls });
  return (
    <Group id="set-studio" find="plugin roblox" title="Roblox Studio">
      <div class="sg-list" id="needs">
        {known && <Need ok={!!status.studio} name="Roblox Studio" what={status.studio ? "Its MCP server is on, so agents in Roblox projects can work in your place." : "For Roblox projects. In Studio, open Assistant settings and turn on the MCP server."} />}
        {known && <Need ok={!!pl.connected} name="Studio plugin" what={pluginLine(pl, note.at === polls ? note.text : "")} />}
      </div>
      <SettingsRow id="set-plugin" title="Install or remove the plugin" desc="Tells the agent what you have selected in Studio, shows it new errors, and lets you undo a whole turn."
        ctl={<>
          <button class="pill" id="plug-install" title={"ai" in status ? "The plugin tells the agent what you have selected in Studio, shows it new errors, and lets you undo a whole turn. It talks only to this app on this computer." : undefined}
            onClick={async () => { const r = await api("/api/plugin", { action: "install" }); await host.refresh(); if (!r.ok) said(r.error || "Could not install the plugin."); }}>{pl.installed ? (pl.current ? "Reinstall plugin" : "Update plugin") : "Install plugin"}</button>
          <button class="pill" id="plug-remove" hidden={!pl.installed} onClick={async () => { await api("/api/plugin", { action: "remove" }); await host.refresh(); said("Removed. Studio stops using it the next time it starts."); }}>Remove plugin</button>
        </>} />
    </Group>
  );
}
function Blender() {
  const status = useStore((s) => s.status);
  return (
    <Group id="set-blender" title="Blender">
      <div class="sg-list" id="needs-blender">
        {status.studio !== undefined && <Need ok={!!status.blender} name="Blender" what={status.blender ? "Found. Roblox projects use it for 3D models, icons and animations." : "For Roblox projects. Not found: install Blender, then open Settings again."} />}
      </div>
    </Group>
  );
}
/* The promo engine: the same row the Promos tab shows, shorter, at its top. The old page works out what it says. */
function Engine() {
  const e = useStore((s) => s.engine) || { text: "Checking...", button: "", error: "", bar: -1, ready: false, running: false, title: undefined as string | undefined };
  const bar = useRef(-1); if (e.bar >= 0) bar.current = e.bar;
  return (
    <Group id="set-engine" find="promo video" title="Promo engine">
      <SettingsRow cls={"eng-row" + (e.ready ? " ready" : "")} title="Promo engine" desc={e.text} descCls="eng-state" descTitle={e.title} ctlCls="act"
        ctl={<button class="pill eng-install" hidden={!e.button} disabled={e.running} onClick={() => engineInstall()}>{e.button || "Install"}</button>}>
        <div class="eng-bar" hidden={e.bar < 0}><i style={bar.current >= 0 ? "width: " + bar.current + "%" : undefined}></i></div><pre class="eng-error">{e.error}</pre>
      </SettingsRow>
    </Group>
  );
}
function Check() {
  const out = useStore((s) => s.setupOut);
  return (
    <Group id="set-check" title="Check everything">
      <SettingsRow id="set-full-check" title="Run a full check" desc="Finds every program the tools use and says how to fix what is missing."
        ctl={<button class="pill" id="b-check" onClick={() => { store.set({ setupOut: "Checking..." }); api("/api/setup", {}); }}>Run full check</button>} after={<pre id="setup-out">{out}</pre>} />
      <SettingsRow id="set-other-apps" title="Connect other apps" desc="Adds these tools to Claude Code, Antigravity or Cursor, to use them outside this window."
        ctl={<button class="pill" id="b-agents" title="Adds these tools to Claude Code, Antigravity or Cursor, to use them outside this window." onClick={async () => { const r = await api("/api/connect", {}); store.set({ setupOut: r.text }); }}>Connect other apps</button>} />
    </Group>
  );
}
export function Connections() {
  const status = useStore((s) => s.status);
  return (
    <>
      <Group id="set-ais" find="claude gemini chatgpt cursor opencode account sign in" title="AIs">
        <Ais />
        <SettingsRow id="set-env-keys" find="api key environment billing" title="Use API keys from my environment" labelFor="env-keys" desc="Off: an AI signed in with your plan is never billed to an API key set on this computer."
          more="Off: an AI signed in with your plan is never billed to an API key set on this computer, such as ANTHROPIC_API_KEY. Turn it on if you sign in with a key or a company gateway. It covers Claude, Cursor and ChatGPT; OpenCode never gets these keys, since each one would add a provider billed to it."
          ctl={<Switch id="env-keys" on={!!status.env_keys} onChange={async (on) => { await api("/api/env-keys", { on }); await host.refresh(); }} />} />
      </Group>
      <Group id="set-github" title="GitHub"><GitHub /></Group>
      <Studio />
      <Blender />
      <Engine />
      <Check />
    </>
  );
}
