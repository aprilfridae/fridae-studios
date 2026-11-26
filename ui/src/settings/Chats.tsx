/* Settings, Chats: saved chats. Deleting is behind a typed word, and it can be undone. */
import { useState } from "preact/hooks";
import { api } from "../api";
import { useStore } from "../store";
import { host } from "../host";
import { aiName } from "../ai";
import { ago } from "../util";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Field } from "../ui/Field";
import { ListRow } from "../ui/Row";

export function Chats() {
  const saved = useStore((s) => s.saved), [open, setOpen] = useState(false), [word, setWord] = useState("");
  useStore((s) => s.ais);
  const all = saved.chats || [], gone = saved.deleted || [], unlocked = word.trim().toLowerCase() === "delete";
  const row = (c: any, label: string, act: () => void, locked?: boolean) => (
    <ListRow name={c.name} line={aiName(c.ai || "claude") + " · " + ago(c.updated) + " · " + c.messages + (c.messages === 1 ? " message" : " messages")}>
      <button class="pill sm" aria-label={label + " " + c.name} disabled={locked} title={locked === undefined ? undefined : locked ? "Type delete in the box above first." : "Move this chat to Deleted."} onClick={act}>{label}</button>
    </ListRow>
  );
  return (
    <Group id="set-saved" find="delete history restore" title="Saved chats">
      <SettingsRow id="set-saved-manage" title="Saved chats" descId="saved-state"
        desc={all.length + (all.length === 1 ? " saved chat" : " saved chats") + (gone.length ? " · " + gone.length + " deleted" : "") + ". Open chats are not counted."}
        ctl={<button class="pill" id="saved-manage" onClick={() => { setWord(""); if (!open) host.reloadSaved(); setOpen(!open); }}>Delete a chat</button>}
        after={<div id="saved-panel" hidden={!open}>
          <p class="sub">Type delete to unlock one deletion. A deleted chat can be put back below.</p>
          <Field id="saved-word" value={word} onInput={setWord} placeholder="Type delete" label="Type delete to unlock one deletion" />
          <div id="saved-list">
            {all.map((c) => row(c, "Delete", async () => { await api("/api/delete-chat", { id: c.id, confirm: word }); setWord(""); await host.reloadSaved(); }, !unlocked))}
            {!all.length && <p class="sub">No saved chats to delete.</p>}
          </div>
          <div class="g-group" id="saved-gone-head" hidden={gone.length === 0}>Deleted</div>
          <div id="saved-gone">{gone.map((c) => row(c, "Put back", async () => { await api("/api/restore-chat", { id: c.id }); await host.reloadSaved(); }))}</div>
        </div>} />
    </Group>
  );
}
