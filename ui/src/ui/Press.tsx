/* A button whose press takes a moment: greyed until it is done, so a second press never lands. */
import type { ComponentChildren } from "preact";
import { useState } from "preact/hooks";
type Props = { cls?: string; id?: string; title?: string; say?: string; disabled?: boolean; hidden?: boolean; pressed?: boolean; style?: string; onPress(): unknown; children: ComponentChildren };
export function Press(p: Props) {
  const [busy, setBusy] = useState(false);
  const press = async () => { setBusy(true); try { await p.onPress(); } finally { setBusy(false); } };
  return <button class={p.cls || "pill sm"} id={p.id} title={p.title} aria-label={p.say} aria-pressed={p.pressed === undefined ? undefined : p.pressed ? "true" : "false"} style={p.style} hidden={p.hidden} disabled={busy || p.disabled} onClick={press}>{p.children}</button>;
}
