/* Settings, General: the AI new chats start on, the output folder, and your notes. */
import { api } from "../api";
import { cell, store, useStore } from "../store";
import { host } from "../host";
import { defaultAi } from "../ai";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Press } from "../ui/Press";
import { whenOpened } from "./state";

/* Your notes: counts for everything and for the project in front */
const notes = cell<{ line: string; game: boolean }>({ line: "", game: false });
const count = (c: any, what: string) => what + ": " + c.notes + (c.notes === 1 ? " note" : " notes") + ", " + c.rules + (c.rules === 1 ? " house rule" : " house rules");
async function loadNotes() {
  let r; try { r = await api("/api/notes?path=" + encodeURIComponent(store.get().curProj)); } catch (e) { return; }
  const lines = [count(r.all, "For everything")];
  if (r.game) lines.push(count(r.game, "For this project"));
  notes.set({ line: lines.join(". ") + ".", game: !!r.game });
}
whenOpened(loadNotes);

export function General() {
  const ais = useStore((s) => s.ais), n = notes.use();
  useStore((s) => s.status);
  const start = defaultAi();
  return (
    <>
      <Group id="set-chats-start" title="New chats">
        <SettingsRow id="set-default-ai" title="New chats start on" desc="A chat's AI can be changed at any time from its message box."
          ctl={<div class="r-pills" id="ai-default">{ais.map(([id, name]) => <Press cls={"pill" + (start === id ? " on" : "")} pressed={start === id} onPress={async () => { await api("/api/ai", { choice: id }); await host.refresh(); }}>{name}</Press>)}</div>} />
      </Group>
      <Group id="set-files" title="Files and notes">
        <SettingsRow id="set-output" title="Output folder" desc="Models, textures, icons and renders are saved in one folder."
          ctl={<button class="pill" id="b-out" onClick={() => api("/api/open", { what: "work" })}>Open the output folder</button>} />
        <SettingsRow id="set-notes" find="house rules memory corrections" title="Your notes" desc="When you correct an agent it saves a short note, and later agents read it first."
          more="House rules are the standing preferences every agent follows. Notes for everything stay on this computer; a project's notes live in its folder and travel with it. Open the folder to edit or delete any note."
          ctl={<><button class="pill" id="notes-open" onClick={() => api("/api/notes-open", { scope: "all" })}>Open notes folder</button><button class="pill" id="notes-open-game" hidden={!n.game} onClick={() => api("/api/notes-open", { scope: "game", path: store.get().curProj })}>Open this project's notes</button></>}>
          <p class="sr-status" id="notes-state">{n.line}</p>
        </SettingsRow>
      </Group>
    </>
  );
}
