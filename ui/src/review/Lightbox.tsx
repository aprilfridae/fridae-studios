/* A picture, large, over the page: from Review and from the chat's pictures. With several, the arrow keys go
   through them; Escape or a press anywhere closes it and the keyboard goes back to where it was. */
import { useEffect, useRef } from "preact/hooks";
import { store, useStore, type Light } from "../store";

export function zoom(src: string, list?: string[], from?: HTMLElement | null) {
  const all = list && list.length ? list : [src];
  store.set({ light: { list: all, at: Math.max(0, all.indexOf(src)), from: from || null } });
}
function close() {
  const l = store.get().light;
  if (!l) return;
  store.set({ light: null });
  if (l.from && l.from.isConnected) l.from.focus();
}

export function Lightbox() {
  const light = useStore((s) => s.light), last = useRef<Light | null>(null);
  if (light) last.current = light;   /* the picture stays while the box fades out */
  useEffect(() => {
    /* in the capture phase, so these keys reach nothing under the picture */
    const onKey = (e: KeyboardEvent) => {
      const l = store.get().light;
      if (!l) return;
      if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
        e.preventDefault(); e.stopPropagation();
        if (l.list.length > 1) store.set({ light: { ...l, at: (l.at + (e.key === "ArrowRight" ? 1 : l.list.length - 1)) % l.list.length } });
      } else if (e.key === "Escape") { e.preventDefault(); e.stopImmediatePropagation(); close(); }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);
  const l = last.current;
  return (
    <div class={"light" + (light ? " on" : "")} id="light" role="dialog" aria-label="Picture" onClick={close}>
      <img alt="" src={l ? l.list[l.at] : undefined} />
      <span>{l && l.list.length > 1 ? l.at + 1 + " of " + l.list.length : null}</span>
    </div>
  );
}
