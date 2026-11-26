/* Rename: the name is typed where it stands. Enter (or leaving the box) keeps it, Escape leaves it as it was. Only
   the name Fridae shows changes: the folder on disk keeps its own, so chats and git still find it. */
import { api } from "../api";
import { store } from "../store";
import { host } from "../host";
import { projRow } from "./ProjectMenu";

export function renameProject(path: string) {
  const pr = store.get().projects.find((x) => x.path === path), row = projRow(path), inp = document.getElementById("proj-rename") as HTMLInputElement | null, list = document.getElementById("proj-list");
  if (!pr || !row || !inp || !list) return;
  Object.assign(inp.style, { left: row.offsetLeft + "px", top: row.offsetTop - list.scrollTop + "px", width: row.offsetWidth + "px", height: row.offsetHeight + "px" });
  inp.value = pr.name; inp.title = "Fridae's name for it. The folder keeps its own: " + path;
  inp.hidden = false; inp.focus(); inp.select();
  let over = false;
  const end = async (keep: boolean) => {
    if (over) return; over = true;
    const name = inp.value.replace(/\s+/g, " ").trim(), back = document.activeElement === inp;
    inp.onkeydown = inp.onblur = null; inp.hidden = true;
    if (keep && name && name !== pr.name) {
      let r; try { r = await api("/api/project", { action: "rename", path, name }); } catch (e) { r = null; }
      if (!r || !r.ok) host.say((r && r.error) || "Could not rename the project.");
      else await host.projectRenamed(path, name);
    }
    if (back) { const again = projRow(path); if (again) again.focus(); }
  };
  inp.onkeydown = (ev) => { ev.stopPropagation(); if (ev.key === "Enter" && !ev.isComposing) { ev.preventDefault(); end(true); } else if (ev.key === "Escape") { ev.preventDefault(); end(false); } };
  inp.onblur = () => end(true);
}
/* the box itself: out of sight until a project is being renamed (its place, its words and whether it shows are set
   above, so it is drawn once and left alone) */
export function RenameBox() {
  return <input class="field p-rename" id="proj-rename" type="text" maxLength={80} aria-label="The project's name" hidden />;
}
