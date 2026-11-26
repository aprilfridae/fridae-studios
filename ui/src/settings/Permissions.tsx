/* Settings, Permissions: the programs agents may run without asking. A chat offers to add one when it is refused. */
import { useState } from "preact/hooks";
import { useStore } from "../store";
import { host } from "../host";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Field } from "../ui/Field";
import { ListRow } from "../ui/Row";

export function Permissions() {
  const status = useStore((s) => s.status), list: string[] = status.commands || [], known = "ai" in status || "commands" in status;
  const [name, setName] = useState("");
  const add = async () => { const n = name.trim(); if (!n) return; await host.allowCommands([n], true); setName(""); };
  return (
    <>
      <p class="st-lede" id="cmd-state">{known ? "Agents run only the programs listed here. " + (list.length ? "A chat offers to add one when it is refused." : "None yet: a chat offers to add one when it is refused.") : ""}</p>
      <Group id="set-commands" find="allow command run" title="Programs agents may run">
        <div class="sg-list" id="cmd-list">
          {list.map((n) => <ListRow name={n} line="Runs without asking."><button class="pill sm" aria-label={"Remove " + n} onClick={() => host.allowCommands([n], false)}>Remove</button></ListRow>)}
        </div>
        <SettingsRow id="set-allow" title="Allow a program" labelFor="cmd-name" desc="It then runs in every chat without asking."
          ctl={<Field id="cmd-name" value={name} onInput={setName} onEnter={add} placeholder="A program name, like git or python" label="A program to allow"><button class="pill" id="cmd-add" onClick={add}>Allow</button></Field>} />
      </Group>
    </>
  );
}
