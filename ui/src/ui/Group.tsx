/* A titled group of rows on a settings page; the rows sit in one box unless `bare`. */
import type { ComponentChildren } from "preact";
type Props = {
  id?: string; find?: string; hidden?: boolean;
  title: ComponentChildren; titleId?: string; titleCls?: string;
  act?: ComponentChildren;        /* buttons beside the title */
  bare?: boolean;                 /* the children are laid out by the caller, not in the rows' box */
  boxId?: string; boxHidden?: boolean;
  children?: ComponentChildren;
};
export function Group(p: Props) {
  return (
    <section class="st-sec" id={p.id} data-find={p.find} hidden={p.hidden}>
      <div class="st-sec-head"><h2 id={p.titleId} class={p.titleCls}>{p.title}</h2>{p.act !== undefined && <div class="st-sec-act">{p.act}</div>}</div>
      {p.bare ? p.children : <div class="sg" id={p.boxId} hidden={p.boxHidden}>{p.children}</div>}
    </section>
  );
}
