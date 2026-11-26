/* A longer explanation, folded behind one word. */
import type { ComponentChildren } from "preact";
export function More(p: { children: ComponentChildren; word?: string; id?: string; hidden?: boolean; cls?: string }) {
  return <details class={"moreinfo" + (p.cls ? " " + p.cls : "")} id={p.id} hidden={p.hidden}><summary>{p.word || "More"}</summary>{typeof p.children === "string" ? <p>{p.children}</p> : p.children}</details>;
}
