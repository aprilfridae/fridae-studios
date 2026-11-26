/* Settings, Guide: what the program can do and how the app works, one short line each, the rest behind "More".
   A row with an example is a button that closes Settings and puts the example in the message box (or opens its page). */
import { host } from "../host";
import { Group } from "../ui/Group";
import { More } from "../ui/More";
import { Arrow } from "../ui/icons";
import { CONTROLS, GUIDE, type Row } from "./guide-data";

function GuideRow({ row: [name, what, example, more] }: { row: Row }) {
  const Tag = (example ? "button" : "div") as any;
  return <div class="g-item"><Tag class="g-row" onClick={example ? () => host.example(example) : undefined}><b>{name}</b>{example ? Arrow : null}<small>{what}</small></Tag>{more && <More>{more}</More>}</div>;
}
export function Guide() {
  return (
    <>
      <Group id="set-guide" title="What it can do" bare>
        <p class="st-note">Pick one to put an example in the message box.</p>
        <div class="sg"><div class="sg-list" id="gd-list"><div>{GUIDE.map(([title, rows]) => [<div class="g-group">{title}</div>, ...rows.map((r) => <GuideRow row={r} />)])}</div></div></div>
      </Group>
      <Group id="set-guide-app" title="How the app works">
        <div class="sg-list" id="gd-controls">{CONTROLS.map((r) => <GuideRow row={r} />)}</div>
      </Group>
    </>
  );
}
