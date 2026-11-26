/* The project panel: a drawer over the page with the project's folder, its notes, its kind, git (commit, history, the
   online copy and GitHub) and Delete. `project` is what the old page (ui/index.html) reaches it through, as
   fridae.project. While it is open the body is marked "pd-open" (never "proj": that is the class of a project's row in
   the sidebar). */
import { useEffect, useRef } from "preact/hooks";
import { api } from "../api";
import { host } from "../host";
import { cell, store, useStore } from "../store";
import { closeSetup, openSetup } from "../windows";
import { frontProj, isGame, isPage, projOf } from "./kind";
import { ghAct, ghProj, loadGithub, setPanelPath } from "./github";

type Git = { installed: boolean; repo: boolean; branch?: string; changes: string[]; commits: { hash: string; when: string; subject: string }[]; identity?: boolean; remote?: string };
const pd = cell({ open: false, path: "", name: "Project", out: "", warn: false, git: null as Git | null, del: false, delOut: "", delBusy: false, n: 0 });
const set = (patch: Partial<ReturnType<typeof pd.get>>) => pd.set({ ...pd.get(), ...patch });
/* a refusal from the last press of Put on GitHub or Push now, per project; the name typed for GitHub, per project */
const ghLine: Record<string, { ok: boolean; text: string } | undefined> = {}, ghName: Record<string, string> = {};

async function git(action: string, extra?: Record<string, unknown>) {
  const pr = frontProj(); if (!pr) return null;
  const r = await api("/api/git", Object.assign({ path: pr.path, action }, extra || {}));
  const patch: any = {};
  if (r.state) patch.git = r.state;
  if (action !== "status" && r.text !== undefined) { patch.out = r.text; patch.warn = !r.ok; }
  set(patch);
  if (r.state) loadGithub(pr.path);
  return r;
}
const body = () => document.body.classList;
export function openProject() {
  const pr = frontProj(); if (!pr || isPage(store.get().nav.view)) return;
  closeSetup();
  setPanelPath(pr.path);
  set({ open: true, path: pr.path, name: pr.name, out: "", warn: false, n: pd.get().n + 1 });
  body().add("pd-open"); host.panelOpened(document.getElementById("pd-close")); git("status");
  host.loadProjects();   /* whether PROJECT.md is there now */
}
export function closeProject() {
  if (!body().contains("pd-open")) return;
  body().remove("pd-open"); setPanelPath(""); set({ open: false }); host.panelClosed();
}

const ghRepoName = (name: string) => (name || "").trim().replace(/[^A-Za-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 100) || "my-project";
function ghWhen(at: number) {
  const d = new Date(at * 1000), today = new Date().toDateString() === d.toDateString();
  return today ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : d.toLocaleString([], { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}
/* a press of Put on GitHub or Push now: started (the page follows it), refused at once (said under the buttons), or
   dropped because another press is on its way (nothing said) */
async function ghStart(path: string, action: string, extra?: Record<string, unknown>) {
  ghLine[path] = undefined; pd.poke();
  const r = await ghAct(action, Object.assign({ project: path }, extra || {}));
  if (r && r.busy) return;
  if (!r) ghLine[path] = { ok: false, text: "Could not reach the app. Try again." };
  else if (!r.ok) ghLine[path] = { ok: false, text: r.text };
  pd.poke();
  if (r && r.ok) git("status");
}
/* Put on GitHub (a new repository under your account, private unless you untick it), or once the project is on
   GitHub, Push now and how the last push went. The paste-an-address box below stays for other places. */
function GitHubBox({ st }: { st: Git }) {
  const gh = useStore((s) => s.gh) || {}, busy = useStore((s) => s.ghBusy), at = ghProj.use(), p = pd.get();
  const priv = useRef<HTMLInputElement>(null);
  const pr = frontProj(), info = pr && at.path === pr.path ? at.info : null;
  if (!(pr && st && st.repo && gh.configured !== undefined && gh.configured !== false)) return <div id="pd-gh"></div>;
  const path = pr.path, said = ghLine[path], last = info && info.last_push, running = !!(last && last.state === "running");
  let parts;
  if (!gh.connected) parts = <><p class="sub">Connect GitHub in Settings, Connections, and this project can go on GitHub with one press.</p><div class="act"><button class="pill" disabled={busy} onClick={() => openSetup("connections")}>Open Settings</button></div></>;
  else if (info && info.repo) parts = <><p class="sub">{"This project is on GitHub at " + info.repo + ". Push now sends your latest commits there."}</p>
    <div class="act"><button class="pill" key="push" disabled={running || busy} onClick={() => ghStart(path, "push")}>{"Push now to " + info.repo}</button><button class="pill" key="open" disabled={busy} onClick={() => api("/api/github/open", { what: "repo", project: path })}>Open on GitHub</button></div></>;
  else if (!(st.commits || []).length) parts = <p class="sub">Commit your files first, then you can put this project on GitHub.</p>;
  else {
    if (ghName[path] === undefined) ghName[path] = ghRepoName(pr.name || p.name);
    parts = <><p class="sub">{"Make a new repository under your GitHub account" + (gh.login ? " (" + gh.login + ")" : "") + " and send this project there. Nothing goes to GitHub until you press the button."}</p>
      <div class="keyrow"><input type="text" id="pd-gh-name" autocomplete="off" spellcheck={false} aria-label="Name on GitHub" value={ghName[path]} onInput={(e) => { ghName[path] = (e.target as HTMLInputElement).value; }} />
        <button class="pill" id="pd-gh-put" disabled={running || busy} onClick={() => ghStart(path, "put", { name: (ghName[path] || "").trim(), private: !!priv.current?.checked })}>Put on GitHub</button></div>
      <label class="sub check-row"><input type="checkbox" ref={priv} defaultChecked /> Private: only you can see it</label></>;
  }
  /* under the buttons: a refusal from the last press, else what the app says about the last Put on GitHub or push */
  const out = said ? said.text : running ? last.text : last ? "Last push " + ghWhen(last.at) + ": " + last.text : "";
  const warn = said ? true : !running && last ? !last.ok : false;
  return <div id="pd-gh">{parts}<p class={"sub" + (warn ? " warn" : "")}>{out}</p></div>;
}

const BLANK: Git = { installed: true, repo: false, changes: [], commits: [], identity: true, remote: "" };
function GitPart({ st, hidden }: { st: Git; hidden: boolean }) {
  const remote = useRef<HTMLInputElement>(null), msg = useRef<HTMLInputElement>(null), uname = useRef<HTMLInputElement>(null), email = useRef<HTMLInputElement>(null);
  /* the address box shows what git has, unless you are typing in it */
  useEffect(() => { const r = remote.current; if (r && document.activeElement !== r) r.value = st.remote || ""; });
  const n = st.changes.length;
  const commit = async () => { const r = await git("commit", { message: msg.current!.value }); if (r && r.ok) msg.current!.value = ""; };
  return (
    <div id="pd-repo" hidden={hidden}>
      <pre id="pd-changes">{st.changes.slice(0, 40).join("\n") + (n > 40 ? "\n... and " + (n - 40) + " more" : "")}</pre>
      <div class="keyrow"><input type="text" id="pd-msg" ref={msg} placeholder="What changed?" autocomplete="off" aria-label="What changed" onKeyDown={(e) => { if (e.key === "Enter") commit(); }} /><button class="pill" id="pd-commit" onClick={commit}>Commit</button></div>
      <div id="pd-ident" hidden={!!st.identity}>
        <p class="sub">Git needs a name and email for your commits. They are saved for this project only.</p>
        <div class="keyrow"><input type="text" id="pd-uname" ref={uname} placeholder="Your name" autocomplete="off" aria-label="Your name" /></div>
        <div class="keyrow"><input type="text" id="pd-email" ref={email} placeholder="Email" autocomplete="off" aria-label="Email" /><button class="pill" id="pd-ident-save" onClick={() => git("identity", { name: uname.current!.value, email: email.current!.value })}>Save</button></div>
      </div>
      <Out />
      <h3>History</h3>
      <pre id="pd-log">{st.commits.map((c) => c.hash + "  " + c.when + "  " + c.subject).join("\n")}</pre>
      <p class="sub" id="pd-nolog" hidden={st.commits.length > 0}>Nothing committed yet.</p>
      <h3>Online copy</h3>
      <GitHubBox st={st} />
      <p class="sub">Paste a repository's address (an empty one on GitHub, say) to keep a copy online.</p>
      <div class="keyrow"><input type="text" id="pd-remote" ref={remote} placeholder="https://github.com/you/project.git" autocomplete="off" spellcheck={false} aria-label="Repository address" /><button class="pill" id="pd-remote-save" onClick={() => git("remote", { url: remote.current!.value })}>Save</button></div>
      <div class="act"><button class="pill" id="pd-push" title="Uploads this project's files to the online copy" disabled={!st.remote} onClick={() => { set({ out: "Pushing...", warn: false }); git("push"); }}>Push</button><button class="pill" id="pd-pull" disabled={!st.remote} onClick={() => { set({ out: "Pulling...", warn: false }); git("pull"); }}>Pull</button></div>
    </div>
  );
}
/* what the last git press printed */
function Out() { const p = pd.use(); return <pre id="pd-out" class={p.warn ? "warn" : undefined}>{p.out}</pre>; }

function gitLine(st: Git | null) {
  if (!st) return "";
  if (!st.installed) return "Git is not installed, so this project has no history. Get it from git-scm.com, then reopen the app.";
  if (!st.repo) return "This folder is not a git repository yet.";
  const n = st.changes.length;
  return "On " + (st.branch || "a new branch") + ". " + (n ? n + (n === 1 ? " file has" : " files have") + " changes that are not committed." : "Everything is committed.");
}

export function ProjectPanel() {
  const p = pd.use();
  useStore((s) => s.projects);
  const pr = projOf(p.path) || { path: p.path, name: p.name };
  const plain = !isGame(pr), st = p.git;
  /* a Roblox game nobody marked: say how Fridae knew */
  const found = pr.kind_set === false && !plain;
  const no = useRef<HTMLButtonElement>(null), del = useRef<HTMLButtonElement>(null);
  const kind = async (k: string) => {
    const r = await api("/api/project", { action: "kind", path: p.path, kind: k });
    if (!r.ok) { set({ out: r.error || "That did not work.", warn: false }); return; }
    await host.loadProjects();
  };
  const ask = (on: boolean) => { set({ del: on, delOut: on ? "" : pd.get().delOut }); setTimeout(() => (on ? no : del).current?.focus(), 0); };
  const remove = async (trash: boolean) => {
    const path = p.path, name = p.name;
    set({ delBusy: true });
    let r: any; try { r = await api("/api/project", { action: "delete", path, trash }); } catch (e) { r = { ok: false, error: "The app did not answer." }; }
    set({ delBusy: false });
    if (!r.ok) { set({ delOut: r.error || "That did not work." }); return; }
    set({ del: false }); closeProject();
    await host.loadProjects(); host.showAll();
    host.say((name || "The project") + (trash ? " is in the Trash." : " is no longer in Fridae. Its folder is untouched."));
  };
  return (
    <div class="plate">
      <header><h2 id="pd-name">{p.name}</h2><button class="icon" id="pd-close" aria-label="Close" onClick={closeProject}><svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><path d="M2 2l8 8M10 2l-8 8" /></svg></button></header>
      <p class="sub" id="pd-path">{p.path}</p>
      <div class="seg np-kind" id="pd-kind" role="radiogroup" aria-label="What kind of project" title="A Roblox game also gets Roblox Studio, the Roblox tools and guides, and Promos. Fridae marks a folder with a place file, a Rojo project or Luau scripts as one by itself.">
        {([["plain", "Project"], ["game", "Roblox game"]] as const).map(([k, label]) => {
          const on = k === (plain ? "plain" : "game");
          return <button role="radio" data-k={k} class={on ? "on" : ""} aria-checked={on ? "true" : "false"} title={k === "game" ? (found ? "Fridae found a Roblox place, a Rojo project or Luau scripts in this folder. Press Project if it is not a Roblox game." : "") : undefined} onClick={() => kind(k)}>{label}</button>;
        })}
      </div>
      <div class="act">
        <button class="pill" id="pd-folder" onClick={() => api("/api/project", { action: "open", path: p.path, what: "folder" })}>Open folder</button>
        <button class="pill" id="pd-bible" hidden={!pr.bible} onClick={() => api("/api/project", { action: "open", path: p.path, what: "bible" })}>Open PROJECT.md</button>
      </div>
      <h3>Git</h3>
      <p class="sub" id="pd-git">{gitLine(st)}</p>
      <div class="act" id="pd-norepo" hidden={!(st && st.installed && !st.repo)}><button class="pill" id="pd-init" onClick={() => git("init")}>Start a git repository</button></div>
      <GitPart st={st && st.repo ? st : BLANK} hidden={!(st && st.repo)} />
      <h3>Delete</h3>
      <div class="act" id="pd-del-start" hidden={p.del}><button class="pill" id="pd-del" ref={del} onClick={() => ask(true)}>Delete project…</button></div>
      <div id="pd-del-ask" hidden={!p.del}>
        <p class="sub">Remove it from Fridae and keep its files, or also move its folder to the Trash (you can put it back from there). Its chats stay in your history.</p>
        <div class="act"><button class="pill" id="pd-del-list" disabled={p.delBusy} onClick={() => remove(false)}>Remove from Fridae</button><button class="pill danger" id="pd-del-trash" disabled={p.delBusy} onClick={() => remove(true)}>Move folder to Trash</button><button class="pill" id="pd-del-no" ref={no} onClick={() => ask(false)}>Cancel</button></div>
      </div>
      <p class="sub" id="pd-del-out" hidden={!p.delOut}>{p.delOut}</p>
    </div>
  );
}

export const project = {
  open: openProject, close: closeProject,
  /* a chat of the project in front finished a turn: the panel, if open, shows git as it is now */
  done(path: string) { if (pd.get().open && pd.get().path === path) git("status"); },
};
