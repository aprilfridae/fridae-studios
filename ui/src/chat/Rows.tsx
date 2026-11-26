/* What a row of the conversation looks like: your message, the app's lines, the agent's turn with its steps, words
   and cards. Each is the markup the old page built by hand, so the page's CSS applies as it is. */
import { useLayoutEffect, useRef, useState } from "preact/hooks";
import { api, TOKEN } from "../api";
import { zoom } from "../review/Lightbox";
import { pressOpenCode, pressSignIn } from "../ai";
import { H } from "./host";
import { askNight, drop, fresh, nightOpen, paintNightTag, partChanged, tlOf, touch, useTick, type Part, type Row } from "./chatStore";
import { copyText, sendApp } from "./compose";
import { extOf, md } from "./md";
import { keep } from "./keep";
import { StartCard } from "./StartCard";

const en = (x: { born: number }) => (fresh(x.born) ? " enter" : "");
const FOLD = <svg width="9" height="9" viewBox="0 0 9 9" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 1.5l3 3-3 3" /></svg>;

/* a file as a small tile: its kind and its name */
export function FileTile({ label }: { label: string }) { return <><span class="ext">{extOf(label)}</span><span class="fname">{label}</span></>; }

/* "copy", there only under the pointer or the keyboard */
function addCopy(node: Element, text: string) {
  const b = document.createElement("button"); b.className = "copy"; b.textContent = "copy";
  b.onclick = async (ev) => { ev.stopPropagation(); b.textContent = (await copyText(text)) ? "copied" : "failed"; setTimeout(() => (b.textContent = "copy"), 1400); };
  node.appendChild(b);
}
/* the agent's words: markdown, made into markup when the words first come into view */
function Prose({ pt }: { pt: Part }) {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const b = ref.current!;
    b.innerHTML = pt.html || (pt.html = md(pt.text));
    addCopy(b, pt.text);
    b.querySelectorAll("pre").forEach((pre) => addCopy(pre, pre.textContent || ""));
  }, [pt]);
  return <div class={"prose" + en(pt)} ref={ref} />;
}
/* the steps of a turn: a list while it works, one line to open them once it is done (the steps of a finished turn
   are only in the page while they are open) */
function Steps({ pt, row }: { pt: Part; row: Row }) {
  const n = pt.list.length, shown = !pt.done || pt.open;
  return (
    <div class={"steps" + (pt.done ? " done" : "") + (pt.open ? " open" : "")}>
      <button class="fold" onClick={() => { pt.open = !pt.open; partChanged(row, pt); }}>{FOLD}<span>{pt.done ? n + (n === 1 ? " step" : " steps") : ""}</span></button>
      {shown && pt.list.map((s: { t: string; born: number }, i: number) => <div class={"step" + (pt.live && i === n - 1 ? " live" : "") + en(s)}>{s.t}</div>)}
    </div>
  );
}
/* an expert the chat started: one line where it was started, which opens the expert's own chat */
function XpLink({ x }: { x: any }) {
  const [who, job] = H.splitExpert(x.name, x.expert), h = H.agents.get(x.id);
  return (
    <button class="xp-link" title={[who, job, x.runsOn ? "on " + x.runsOn : ""].filter(Boolean).join(" · ") + ". Opens its own chat."} onClick={() => H.openChatById(x.id)}>
      <span class={"dot" + (h ? H.xpState(h)[1] : "")} /><b>{who}</b><span class="xp-job">{job}</span><span class="xp-go">Open its chat</span>
    </button>
  );
}
/* A sign-in button beside a failed message (the tool's own sign-in, or Fridae's OpenCode): what a press does and
   what is said next are ui/src/ai.ts; no notice is in view here, so what comes next is said over the message box. */
function SignIn({ ai }: { ai: string }) {
  const [off, setOff] = useState(false);
  const press = async () => {
    setOff(true);
    if (ai === "opencode") { await pressOpenCode("login"); return; }
    const said = await pressSignIn(ai, "login", true);
    H.say(said); setOff(false);
  };
  return <button class="pill sm" disabled={off} onClick={press}>Sign in</button>;
}
/* Gemini textures: the prompt to run in Gemini, and the wait for the image */
function Texture({ pt, row }: { pt: Part; row: Row }) {
  const act = (action: string) => api("/api/texture", { id: pt.req, action });
  const copy = async () => { await act("copy"); pt.copied = true; partChanged(row, pt); setTimeout(() => { pt.copied = false; partChanged(row, pt); }, 1500); };
  return (
    <div class={"ask" + en(pt) + (pt.said ? " done" : "")}>
      <h4>{pt.said || "Make this texture in Gemini: " + pt.name}</h4>
      <p>The first button opens Gemini and pastes this prompt into its message box: press Enter there. Then copy the image it makes (right-click, Copy image). It is picked up here automatically. You can also paste or drop the image into this window. If the prompt does not appear in Gemini, press Ctrl+V.</p>
      <pre>{pt.prompt}</pre>
      <div class="act"><button class="pill main" onClick={() => act("open")}>Open Gemini and paste prompt</button><button class="pill" onClick={copy}>{pt.copied ? "Copied" : "Copy prompt"}</button><button class="pill" onClick={() => act("skip")}>Skip</button></div>
      <div class="wait">Waiting for the image</div>
    </div>
  );
}
/* Programs that can start any other program (git aliases and hooks, env, xargs, find -exec, shells, interpreters...):
   the blocked card says so before the user allows one. */
const RUNS_ANY = ["git", "env", "xargs", "find", "sh", "bash", "zsh", "fish", "dash", "python", "python3", "node", "ruby", "perl", "php", "lua", "make", "npm", "npx", "pnpm", "yarn", "bun", "deno", "cargo", "ssh", "sudo", "nice", "time", "nohup", "timeout", "watch", "awk", "osascript", "open", "pwsh", "powershell", "cmd", "command", "exec", "stdbuf", "caffeinate", "doas", "parallel"];
/* Programs agents may run. Chats run without anyone watching, so a command the user has not allowed is refused; the
   chat then shows what was refused and one button to allow that program from now on. */
function Blocked({ pt, row, p }: { pt: Part; row: Row; p: any }) {
  const names: string[] = pt.commands.map((c: any) => c.name), one = names.length === 1, any = names.filter((n) => RUNS_ANY.includes(n));
  const finish = (said: string) => { pt.said = said; partChanged(row, pt); };
  const yes = async () => {
    if (p.busy) { H.say("Wait for the agent to finish, then allow it."); return; }
    if (!(await H.allowCommands(names, true))) return;
    finish("Allowed: " + names.join(", "));
    sendApp(p, "You may now run " + names.join(", ") + ". Run what was refused and carry on.");
  };
  const done = !!pt.said || pt.shut;
  return (
    <div class={"ask" + en(pt) + (done ? " done" : "")} data-approval={pt.approval ? "1" : undefined}>
      <h4>{pt.said || "The agent was not allowed to run " + names.join(", ")}</h4>
      <p>{"Agents only run programs you have allowed. Allowing " + (one ? "it" : "them") + " lets every Claude, Cursor and OpenCode chat run " + (one ? "this program" : "these programs") + " from now on without asking (you can take that back in Settings). Gemini chats can only use the building tools."
        + (any.length ? " Note: " + any.join(", ") + (any.length === 1 ? " can" : " can each") + " run any other program, so allowing " + (any.length === 1 ? "it" : "them") + " is close to allowing everything." : "")}</p>
      <pre>{pt.commands.map((c: any) => c.command).join("\n")}</pre>
      {!done && <div class="act"><button class="pill main" onClick={yes}>{"Allow " + names.join(", ") + " and carry on"}</button><button class="pill" onClick={() => finish("Not allowed: " + names.join(", "))}>Not now</button></div>}
    </div>
  );
}
/* A command the app did not approve by itself for how it is written (not for its program): the card says why in plain
   words and, unless it could read other programs' settings, offers to run that exact command once in this chat. */
function Refused({ pt, row, p }: { pt: Part; row: Row; p: any }) {
  const commands: any[] = pt.commands, runnable = commands.filter((c) => c.once);
  const finish = (said: string) => { pt.said = said; partChanged(row, pt); };
  const yes = async () => {
    if (p.busy) { H.say("Wait for the agent to finish, then answer."); return; }
    let ok = true;
    for (const c of runnable) { const r = await api("/api/run-once", { agent: p.id, command: c.command }); ok = ok && r && r.ok; }
    if (!ok) { H.say("That command can no longer be run from here."); return; }
    finish("Allowed once");
    sendApp(p, "You may run this exactly as written, one time: " + runnable.map((c) => c.command).join("\n") + "\nRun it now, then carry on.");
  };
  const done = !!pt.said || pt.shut;
  return (
    <div class={"ask" + en(pt) + (done ? " done" : "")} data-approval={pt.approval ? "1" : undefined}>
      <h4>{pt.said || (commands.length === 1 ? "A command was not run" : "Some commands were not run")}</h4>
      <div class="why">
        {commands.map((c) => <>
          {/* shown whole, never in a scrolling box; characters that would not show as themselves are written out */}
          <pre class="whole">{c.shown || c.command}</pre>
          <p>{"Not run because " + c.reason}</p>
          {c.no_once && <p>{"There is no button to run it from here, because " + c.no_once + "."}</p>}
        </>)}
      </div>
      {!done && <div class="act">
        {runnable.length > 0 && <button class="pill main" title="Only this exact command, only in this chat, only one time." onClick={yes}>{runnable.length === 1 ? "Run it once" : "Run these once"}</button>}
        <button class="pill" onClick={() => finish(runnable.length ? "Not run" : "Not run: " + commands.map((c) => c.shown || c.command).join(", "))}>{runnable.length ? "Not now" : "OK"}</button>
      </div>}
    </div>
  );
}

/* one thing in a turn; drawn again only when it changed (pt.v) */
const PartView = keep(function PartView({ pt, row }: { pt: Part; row: Row; v: number }) {
  const p = row.p;
  switch (pt.k) {
    case "steps": return <Steps pt={pt} row={row} />;
    case "prose": return <Prose pt={pt} />;
    case "tag": return <span class={"tag" + en(pt)}>{pt.text}</span>;
    case "xp": return <XpLink x={pt.x} />;
    case "tex": return <Texture pt={pt} row={row} />;
    case "plain": return <div class={"plain-words" + en(pt)}><b>In plain words</b><p>{pt.text}</p></div>;
    case "shots": return <div class={"shots" + en(pt)}>{pt.urls.map((u: string) => <button class="shot" onClick={(ev) => zoom((ev.currentTarget.firstChild as HTMLImageElement).src)}><img src={u + "?t=" + TOKEN} alt="Result image" /></button>)}</div>;
    case "blocked": return <Blocked pt={pt} row={row} p={p} />;
    case "refused": return <Refused pt={pt} row={row} p={p} />;
    case "err": return <div class={"err" + en(pt)}>{pt.text}{pt.signIn && <SignIn ai={pt.signIn} />}</div>;
    case "line": return <div class={"promo-line" + en(pt)}><span>{pt.text}</span><button class="linkish" onClick={pt.press}>{pt.go}</button></div>;
  }
  return null;
}, (a, b) => a.pt === b.pt && a.v === b.v);

/* the agent's turn: who answered, then its steps, words and cards in the order they came */
function Turn({ row }: { row: Row }) {
  const p = row.p, u = row.undo;
  const undo = async () => {
    if (u.said) return;
    u.text = "Undoing..."; touch(row);
    const r = await api("/api/plugin", { action: "revert", agent: p.id });
    u.text = r.text || "Could not undo."; u.said = true; touch(row);
  };
  return (
    <div class={"turn" + en(row)}>
      <div class="label">{row.label}</div>
      {row.parts.map((pt: Part) => <PartView key={pt.id} pt={pt} row={row} v={pt.v} />)}
      {u && <button class={"undo-turn" + (u.said ? " said" : "")} title="Undoes the changes agents made in Studio since this turn began, and puts scripts back as they were. It stops if your own edit is the newest step." onClick={undo}>{u.text}</button>}
    </div>
  );
}

/* a new chat mentions only what is missing for its work: its AI, and Studio for a Roblox project */
function Needs() {
  useTick();
  return <>{H.helloNeeds().map(([ok, name, what]: [boolean, string, string]) => <div class="c-row"><span class={"dot" + (ok ? " ok" : "")} /><div><b>{name}</b><small>{what}</small></div></div>)}</>;
}
function Hello({ row }: { row: Row }) {
  return (
    <div class={"hello" + en(row)}>
      <img class="sigil" src="/logo.png" alt="" /><h1>What are we <span>building today?</span></h1>
      <p class="lede">{row.lede || "Say it in plain words. I show you what I made before calling it done."}</p>
      <div class="hello-needs"><Needs /></div>
      <StartCard p={row.p} />
    </div>
  );
}

/* The card that asks before undoing one chat's night. It checks once more that the night is still there to undo
   before it calls the app. While the call is on its way the chat is marked (nightUndoing), so its undo is offered
   nowhere else until the app has answered. */
function NightAsk({ row }: { row: Row }) {
  const p = row.p, tl = tlOf(p);
  const when = new Date((p.night.start || 0) * 1000).toLocaleString([], { weekday: "short", hour: "numeric", minute: "2-digit" });
  const what = [p.night.studio ? "the agents' changes in Studio since then, step by step, stopping at the first step that is yours; and the text of every script goes back to how it was then, your own and other chats' script edits included (first, the scripts as they are now are saved in the game's folder, in .fridae/before-undo/ under the time of the undo)" : "", p.night.commit ? "the project's files, as they were at the checkpoint (the night's versions stay in git history)" : ""].filter(Boolean);
  const undo1 = async (filesOnly: boolean) => {
    if (p.nightUndoing) return;                                               /* already on its way */
    if (!nightOpen(p) || !H.agents.has(p.id)) { drop(tl, row); return; }      /* undone meanwhile, at work again, or closed */
    row.busy = filesOnly ? "files" : "go"; touch(row);
    p.nightUndoing = true; paintNightTag(p);
    let r; try { r = await api("/api/undo-night", { agent: p.id, files_only: filesOnly }); } catch (e) { r = null; }
    p.nightUndoing = false;
    if (r && r.ok) { drop(tl, row); p.nightUndone = true; paintNightTag(p); return; }
    paintNightTag(p);
    row.busy = "";
    /* it stays open and says why; when the app refused only Studio's part, the files alone can still go back */
    if (r && r.files_only) { row.said = r.text || " "; row.files = true; }
    else row.said = (r && r.text) || "Could not undo the night.";
    touch(row);
  };
  return (
    <div class={"ask night-ask" + en(row)} data-chat={p.id}>
      <h4>Undo the night?</h4>
      <p>{row.said ? row.said.trim() : "Goes back to " + when + ", when the overnight run in “" + p.name + "” began. Undone: " + what.join("; and ") + ". The task cards are kept. Studio must still be open on the same place."}</p>
      <div class="act">
        <button class="pill main" disabled={!!row.busy} onClick={() => undo1(false)}>{row.busy === "go" ? "Undoing..." : "Undo the night"}</button>
        {/* shown only after the app refused Studio's part: the files go back, Studio is left as it is */}
        <button class="pill" hidden={!row.files} disabled={!!row.busy} onClick={() => undo1(true)}>{row.busy === "files" ? "Undoing..." : "Undo the files only"}</button>
        <button class="pill" disabled={!!row.busy} onClick={() => drop(tl, row)}>Keep it</button>
      </div>
    </div>
  );
}

export function RowBody({ row }: { row: Row }) {
  const p = row.p;
  switch (row.k) {
    case "turn": return <Turn row={row} />;
    case "you": return (
      <div class={"you" + en(row)}>
        {row.images && <div class="pics">{row.images.map((src: string) => <img src={src + "?t=" + TOKEN} alt="Attached image" onClick={(ev) => zoom((ev.currentTarget as HTMLImageElement).src)} />)}</div>}
        {row.files && <div class="docs">{row.files.map((label: string) => <span class="d"><FileTile label={label} /></span>)}</div>}
        <div>{row.text}</div>
      </div>
    );
    case "tag": return <span class={(row.cls || "tag") + en(row)}>{row.text}</span>;
    case "hello": return <Hello row={row} />;
    case "xp": return <XpLink x={row.x} />;
    case "night": return <span class={"tag" + en(row)}><span>{row.text}</span>{!row.old && <button class="linkish" hidden={!row.btn} title="Put Studio back as it was before the overnight run began" onClick={() => askNight(p)}>Undo the night</button>}</span>;
    case "nightask": return <NightAsk row={row} />;
    case "card": return <div class={"ask" + en(row)}><h4>{row.title}</h4><p style="white-space: pre-line">{row.text}</p></div>;
    case "wait": return <div class={"studio-wait" + en(row)}><span>Waiting for you in Review</span><button class="linkish" onClick={() => H.openInReview(p.project && p.project.path)}>Open Review</button></div>;
    case "queue": return (
      <div class="qbox">
        {row.items.map((q: any) => (
          <div class={"you queued" + en(row)} key={q.qid}>
            <div>{q.text}</div>
            <span class="qmeta">{row.paused ? "Held while the studio run is paused. Sent when you press Resume." : row.kept ? "Kept. Sent with your next message." : "Waiting. Sent the moment it finishes."} <button onClick={() => api("/api/unqueue", { agent: p.id, qid: q.qid })}>Remove</button></span>
          </div>
        ))}
      </div>
    );
  }
  return <div />;
}
