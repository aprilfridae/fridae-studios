/* A new chat's setup: a folder and repository for the project, then (a Roblox game) new game or existing. Its state
   is the chat's own `p.start` ({ step, name, parent, git, kind, msg, paint }), which the page also sets when the chat
   is tied to a project. On an empty page (`p.draft`) the chat is made first; its own card then carries on. */
import { api } from "../api";
import { H } from "./host";
import { effortOf, focusBox, kickoffChat, modelOf } from "./compose";

export function StartCard({ p }: { p: any }) {
  const st = p.start;
  if (!st) return null;
  const projects = H.projects, pr = p.project;
  const go = (step: string, msg?: string) => { st.step = step; st.msg = msg || ""; st.paint(); };
  async function realize() { if (!p.draft) return p; const id = await H.ensureChat(p.key); return id ? H.agents.get(id) : null; }
  async function use(body: any) {
    const q = await realize(); if (!q) return;
    const r = await api("/api/project", Object.assign({ agent: q.id }, body));
    if (!r.ok) { if (q.start) { q.start.msg = r.error || "That did not work."; q.start.paint(); } return go(st.step, r.error || "That did not work."); }
    q.project = r.project; H.paintProject(); H.loadProjects();
    if (q !== p) { if (q.start) { q.start.step = H.startStep(r.project); q.start.msg = r.project.note || ""; q.start.paint(); } return; }
    go(H.startStep(r.project), r.project.note || "");
  }
  const kickoff = async (kind: string) => { const q = await realize(); if (q) kickoffChat(q, kind); };
  const create = () => use({ action: "create", name: st.name, parent: st.parent || null, git: st.git && projects.git, kind: st.kind });
  const acts: Record<string, () => void> = {
    new: () => { st.focus = true; go("new"); },
    back: () => go("folder"),
    skip: () => { p.start = null; st.paint(); focusBox(); },
    git: () => { st.git = !st.git; st.paint(); },
    pick: async () => { const r = await api("/api/project", { action: "pick" }); if (r.path) use({ action: "add", path: r.path }); },
    where: async () => { const r = await api("/api/project", { action: "pick" }); if (r.path) { st.parent = r.path; st.paint(); } },
    kp: async () => { const q = await realize(); if (q) api("/api/send", { agent: q.id, text: "Read through this project and get up to speed: what it is, how it is built and run, and its tests.", model: modelOf(q) || null, effort: effortOf(q), selftest: false, gemini: false, attachments: [] }); },
    kn: () => kickoff("new"),
    ke: () => kickoff("existing"),
  };
  let body;
  if (st.step === "folder") {
    body = <>
      <h4>Start a folder for this project?</h4><p>A folder keeps the project's notes, files and history in one place.</p>
      <div class="act"><button class="pill main" onClick={acts.new}>New folder</button><button class="pill" onClick={acts.pick}>Existing folder</button><button class="pill" onClick={acts.skip}>Skip, just chat</button></div>
      {projects.projects.length > 0 && <><p class="sub">Or carry on with a project:</p><div class="act">{projects.projects.slice(0, 6).map((x: any) => <button class="pill" onClick={() => (p.draft ? H.showProject(x.path, "chat") : use({ action: "use", path: x.path }))}>{x.name}</button>)}</div></>}
    </>;
  } else if (st.step === "new") {
    body = <>
      <h4>New project folder</h4>
      <div class="seg np-kind" role="radiogroup" aria-label="What kind of project">{[["plain", "Project"], ["game", "Roblox game"]].map(([k, t]) => <button role="radio" class={st.kind === k ? "on" : ""} aria-checked={st.kind === k ? "true" : "false"} onClick={() => { st.kind = k; st.paint(); }}>{t}</button>)}</div>
      <p class="sub">{H.KIND_NOTE[st.kind]}</p>
      <div class="keyrow"><input type="text" placeholder="Name of the project" aria-label="Name of the project" autocomplete="off" value={st.name} ref={(i) => { if (i && st.focus) { st.focus = false; i.focus(); } }}
        onInput={(e) => (st.name = e.currentTarget.value)} onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); create(); } }} /></div>
      <p class="sub where">It goes in {st.parent || H.parentFor(st.kind)} &nbsp;<button class="linkish" onClick={acts.where}>Change</button></p>
      {projects.git
        ? <><div class="act"><button class="switch" role="switch" aria-checked={st.git ? "true" : "false"} onClick={acts.git}><span class="track"><span class="knob" /></span><span class="swl">Start a git repository</span></button></div><p class="sub">Git keeps a history of the project that you can look back through and return to.</p></>
        : <p class="sub">Git is not installed, so no history is kept. You can add it later from git-scm.com.</p>}
      <div class="act"><button class="pill main" onClick={create}>Create</button><button class="pill" onClick={acts.back}>Back</button></div>
    </>;
  } else if (st.step === "kind") {
    body = <>
      <h4>Is this a new game, or one you are already working on?</h4><p>{(pr ? pr.name : "") + (pr && pr.git ? " · git repository" : "")}</p>
      <div class="act"><button class="pill main" onClick={acts.kn}>New game</button><button class="pill" onClick={acts.ke}>Already working on it</button></div>
      <p class="sub">New game: I ask about the idea before building. Already working on it: I read the place and the folder first.</p>
    </>;
  } else {
    const game = !H.plainProj(pr);
    body = <>
      <h4>{pr ? pr.name : "Project"}</h4><p>{pr && pr.bible ? "PROJECT.md is read first. Say what you want to work on." : "Say what you want to work on."}</p>
      {game ? <div class="act"><button class="pill" onClick={acts.ke}>Read through the game again</button></div> : <div class="act"><button class="pill" onClick={acts.kp}>Read through the project</button></div>}
    </>;
  }
  return <div class="ask start">{body}{st.msg && <p class="sub warn">{st.msg}</p>}</div>;
}
