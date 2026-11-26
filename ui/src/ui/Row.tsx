/* The small rows lists are made of: a dot and two lines (what is connected), or a name, a line and a button. */
import type { ComponentChildren } from "preact";
export function DotRow(p: { dot?: string; name: ComponentChildren; children?: ComponentChildren; tag?: "div" | "li"; ai?: string }) {
  const Tag = (p.tag || "div") as any;
  return <Tag class="c-row" data-ai={p.ai}><span class={"dot" + (p.dot ? " " + p.dot : "")}></span><div><b>{p.name}</b>{p.children}</div></Tag>;
}
export function ListRow(p: { name: ComponentChildren; line: ComponentChildren; cls?: string; children?: ComponentChildren }) {
  return <div class={"g-row" + (p.cls ? " " + p.cls : "")}><b>{p.name}</b><small>{p.line}</small>{p.children}</div>;
}
