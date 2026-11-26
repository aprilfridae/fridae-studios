/* A text box with its button beside it. Enter in the box presses `onEnter`. */
import type { ComponentChildren, Ref } from "preact";
type Props = {
  id?: string; type?: string; placeholder?: string; label?: string; value?: string; maxlength?: number; spellcheck?: boolean;
  onInput?(value: string): void; onEnter?(): void; inputRef?: Ref<HTMLInputElement>; children?: ComponentChildren;
};
export function Field(p: Props) {
  return (
    <div class="keyrow">
      <input type={p.type || "text"} id={p.id} ref={p.inputRef} value={p.value} maxlength={p.maxlength} placeholder={p.placeholder} autocomplete="off" spellcheck={p.spellcheck === undefined ? false : undefined} aria-label={p.label}
        onInput={p.onInput && ((e) => p.onInput!((e.target as HTMLInputElement).value))}
        onKeyDown={p.onEnter && ((e) => { if (e.key === "Enter" && !e.isComposing) { e.preventDefault(); p.onEnter!(); } })} />
      {p.children}
    </div>
  );
}
