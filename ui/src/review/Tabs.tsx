/* A tab per project with something waiting, and how many things need the user there. The arrow keys, Home and End
   go from project to project. */
import { useEffect, useRef } from "preact/hooks";

type Tab = { path: string; name: string; n: number };

export function Tabs({ tabs, on, pick }: { tabs: Tab[]; on: string; pick: (path: string) => void }) {
  const bar = useRef<HTMLDivElement>(null), held = useRef(false);
  /* the keyboard stays in the tabs: on the chosen one after an arrow key, or after the tab it was on has gone */
  useEffect(() => {
    const el = bar.current;
    if (!held.current || !el || el.contains(document.activeElement)) return;
    const b = el.querySelector<HTMLElement>('[aria-selected="true"]');
    if (b) b.focus(); else held.current = false;
  });
  const onKey = (ev: KeyboardEvent) => {
    const buttons = [...bar.current!.querySelectorAll("button")], i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0 || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(ev.key)) return;
    ev.preventDefault();
    const to = ev.key === "Home" ? 0 : ev.key === "End" ? tabs.length - 1 : (i + (ev.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length;
    pick(tabs[to].path);
    buttons[to].focus();
  };
  return (
    <div class="rv-tabs" id="rv-tabs" role="tablist" aria-label="Projects with something waiting" hidden={!tabs.length} ref={bar}
      onKeyDown={onKey} onFocusIn={() => { held.current = true; }} onFocusOut={() => { held.current = false; }}>
      {tabs.map((t, i) => (
        <button key={t.path} id={"rv-tab-" + i} data-path={t.path} role="tab" aria-selected={t.path === on ? "true" : "false"} aria-controls="rv-items" tabIndex={t.path === on ? 0 : -1}
          aria-label={t.n ? t.name + ", " + t.n + (t.n === 1 ? " thing needs you" : " things need you") : undefined} onClick={() => pick(t.path)}>
          <span>{t.name}</span>{t.n ? <i>{t.n}</i> : null}
        </button>
      ))}
    </div>
  );
}
