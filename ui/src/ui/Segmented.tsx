/* A few choices side by side, one of them on. */
import type { ComponentChildren } from "preact";
type Option<T> = { value: T; label: ComponentChildren; say?: string; data?: Record<string, string> };
type Props<T> = {
  id?: string; cls?: string; label?: string; labelledBy?: string; busy?: boolean; disabled?: boolean;
  options: Option<T>[]; value: T | null; onPick(value: T): void;
  pressed?: boolean;   /* also say which is on with aria-pressed */
};
export function Segmented<T>(p: Props<T>) {
  return (
    <div class={"seg" + (p.cls ? " " + p.cls : "")} id={p.id} role="group" aria-label={p.label} aria-labelledby={p.labelledBy} aria-busy={p.busy === undefined ? undefined : p.busy ? "true" : "false"}>
      {p.options.map((o) => {
        const on = o.value === p.value;
        return <button {...(o.data || {})} class={on ? "on" : ""} aria-label={o.say} aria-pressed={p.pressed ? (on ? "true" : "false") : undefined} disabled={p.disabled} onClick={() => p.onPick(o.value)}>{o.label}</button>;
      })}
    </div>
  );
}
