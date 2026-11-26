/* New project: Project or Roblox game, a name and Create, or an existing folder, or a chat with no project. */
import { useState } from "preact/hooks";
import { api } from "../api";
import { host } from "../host";
import { useStore } from "../store";

export const KIND_NOTE: Record<string, string> = { plain: "Any project: an app, a site, a tool, or anything else.", game: "A Roblox game: also gets Roblox Studio, the Roblox tools and guides, and Promos." };
const KINDS: [string, string][] = [["plain", "Project"], ["game", "Roblox game"]];

/* a folder made or added here opens in its Chat, with a chat ready for it */
async function opened(project: any) { await host.loadProjects(); host.showProject(project.path, "chat"); }

export function NewProject() {
  const reply = useStore((s) => s.projReply);
  const [kind, setKind] = useState("plain"), [parent, setParent] = useState(""), [git, setGit] = useState(true), [note, setNote] = useState(""), [busy, setBusy] = useState(false);
  const name = () => document.getElementById("np-name") as HTMLInputElement;
  const where = parent || (kind === "plain" ? reply.default_parent_plain : reply.default_parent) || (kind === "plain" ? "your projects folder" : "your Roblox games folder");
  const create = async () => {
    const n = name().value.trim(); if (!n) { setNote("Give the project a name first."); name().focus(); return; }
    setBusy(true); setNote("");
    const r = await api("/api/project", { action: "create", name: n, parent: parent || null, git: git && !!reply.git, kind });
    setBusy(false);
    if (!r.ok) { setNote(r.error || "Could not make the folder."); return; }
    name().value = "";
    if (r.project.note) host.say(r.project.note);
    opened(r.project);
  };
  const add = async () => {
    const pick = await api("/api/project", { action: "pick" }); if (!pick.path) return;
    const r = await api("/api/project", { action: "add", path: pick.path, kind });
    if (!r.ok) { setNote(r.error || "Could not add that folder."); return; }
    opened(r.project);
  };
  return (
    <div class="r-col">
      <div class="np-make">
        <div class="seg np-kind" id="np-kind" role="radiogroup" aria-label="What kind of project">
          {KINDS.map(([k, label]) => <button role="radio" data-k={k} class={k === kind ? "on" : ""} aria-checked={k === kind ? "true" : "false"} onClick={() => { setKind(k); name().focus(); }}>{label}</button>)}
        </div>
        <p class="sub kind-note" id="np-kind-note">{KIND_NOTE[kind]}</p>
        <div class="keyrow"><input type="text" id="np-name" placeholder="Name of the project" autocomplete="off" aria-label="Name of the project" onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); create(); } }} /><button class="pill main" id="np-create" disabled={busy} onClick={create}>Create</button></div>
        <p class="sub where" id="np-where">{"It goes in " + where + "  "}<button class="linkish" id="np-where-b" onClick={async () => { const r = await api("/api/project", { action: "pick" }); if (r.path) setParent(r.path); }}>Change</button></p>
        <div class="act" id="np-git-row" hidden={!reply.git}><button class="switch" id="np-git" role="switch" aria-checked={git ? "true" : "false"} onClick={() => setGit(!git)}><span class="track"><span class="knob"></span></span><span class="swl">Start a git repository</span></button></div>
        <p class="sub warn" id="np-note" hidden={!note}>{note}</p>
      </div>
      <p class="np-other"><button class="linkish" id="np-add" onClick={add}>Add a folder you already have</button><button class="linkish" id="np-chat" onClick={() => host.showProject("", "chat")}>Just chat, without a project</button></p>
    </div>
  );
}

export const newProject = {
  /* the page came to the front: where new projects go is asked for again, and the keyboard goes to the name */
  open() { host.loadProjects(); setTimeout(() => (document.getElementById("np-name") as HTMLInputElement | null)?.focus(), 0); },
};
