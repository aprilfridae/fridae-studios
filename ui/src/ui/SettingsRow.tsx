/* One row of a settings page: a title and a line about it on the left, its control on the right. */
import type { ComponentChildren } from "preact";
import { More } from "./More";

type Props = {
  id?: string; find?: string; cls?: string; hidden?: boolean;
  title: ComponentChildren; titleId?: string; labelFor?: string;   /* labelFor: the title is the label of that control */
  desc?: ComponentChildren; descId?: string; descCls?: string; descTitle?: string;
  more?: ComponentChildren;        /* a longer explanation, folded behind "More" */
  children?: ComponentChildren;    /* more lines under the description (a status line) */
  ctl?: ComponentChildren; ctlCls?: string;
  after?: ComponentChildren;       /* under the whole row, inside its box */
};
export function SettingsRow(p: Props) {
  return (
    <div class={"sr" + (p.cls ? " " + p.cls : "")} id={p.id} data-find={p.find} hidden={p.hidden}>
      <div class="sr-in">
        <div class="sr-text">
          <h3 id={p.titleId}>{p.labelFor ? <label for={p.labelFor}>{p.title}</label> : p.title}</h3>
          {p.desc !== undefined && <p id={p.descId} class={p.descCls} title={p.descTitle}>{p.desc}</p>}
          {p.more && <More>{p.more}</More>}
          {p.children}
        </div>
        {p.ctl !== undefined && <div class={"sr-ctl" + (p.ctlCls ? " " + p.ctlCls : "")}>{p.ctl}</div>}
      </div>
      {p.after}
    </div>
  );
}
