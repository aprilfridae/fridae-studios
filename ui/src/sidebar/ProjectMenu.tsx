/* A project's menu: Rename, Open folder, Delete. It opens at the pointer on a right-click, under the row's button,
   or beside the row for Shift+F10 and the Menu key. The arrow keys move in it, Escape closes it, and the keyboard
   goes back to the row. */
import { useEffect, useLayoutEffect, useRef } from "preact/hooks";
import { api } from "../api";
import { cell, store } from "../store";
import { host } from "../host";
import { askDelete } from "./DeleteProject";
import { renameProject } from "./Rename";

type At = Element | { x: number; y: number };
/* the menu that is open (`on`), or the one last shown: its rows stay as they were while it fades */
export const menu = cell<{ path: string; name: string; at: At; on: boolean; n: number }>({ path: "", name: "Project", at: { x: 0, y: 0 }, on: false, n: 0 });
/* the project whose menu is open, "" for none */
export const menuPath = () => (menu.get().on ? menu.get().path : "");
/* a project's row in the sidebar */
export const projRow = (path: string) => [...document.querySelectorAll<HTMLElement>("#proj-list .proj")].find((b) => b.dataset.k === "p:" + path);

export function closeMenu(refocus: boolean) {
  const m = menu.get(); if (!m.on) return;
  menu.set({ ...m, on: false });
  const row = refocus && projRow(m.path); if (row) row.focus();
}
/* `at`: the row's button or the row itself (the menu opens under it), or a pointer's { x, y } */
export function openMenu(path: string, at: At) {
  const pr = store.get().projects.find((x) => x.path === path);
  if (!pr) return;
  closeMenu(false); host.closePops();
  menu.set({ path, name: pr.name, at, on: true, n: menu.get().n + 1 });
}

export function ProjectMenu() {
  const m = menu.use(), ref = useRef<HTMLDivElement>(null);
  /* where: the pointer, or under what opened it; kept inside the window */
  useLayoutEffect(() => {
    const el = ref.current; if (!el || !m.on) return;
    const z = store.get().scale / 100, at = m.at as any, r = at.getBoundingClientRect ? at.getBoundingClientRect() : null;
    const x = (r ? r.left : at.x) / z, y = (r ? r.bottom + 4 : at.y) / z, w = el.offsetWidth, h = el.offsetHeight;
    el.style.left = Math.max(8, Math.min(x, innerWidth / z - w - 8)) + "px";
    el.style.top = Math.max(8, Math.min(y, innerHeight / z - h - 8)) + "px";
    const first = el.querySelector("button"); if (first) first.focus();
  }, [m.n]);
  useEffect(() => {
    const away = (ev: MouseEvent) => { const t = ev.target as Element; if (menuPath() && !t.closest("#proj-menu") && !t.closest(".p-more")) closeMenu(false); };
    const shut = () => closeMenu(false);
    document.addEventListener("mousedown", away); window.addEventListener("blur", shut); window.addEventListener("resize", shut);
    return () => { document.removeEventListener("mousedown", away); window.removeEventListener("blur", shut); window.removeEventListener("resize", shut); };
  }, []);
  const keys = (ev: KeyboardEvent) => {
    const items = [...ref.current!.querySelectorAll("button")], i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (ev.key === "Escape") { ev.preventDefault(); ev.stopPropagation(); closeMenu(true); }
    else if (ev.key === "Tab") { ev.preventDefault(); closeMenu(true); }
    else if (ev.key === "ArrowDown" || ev.key === "ArrowUp") { ev.preventDefault(); items[(i + (ev.key === "ArrowDown" ? 1 : items.length - 1)) % items.length].focus(); }
    else if (ev.key === "Home" || ev.key === "End") { ev.preventDefault(); items[ev.key === "Home" ? 0 : items.length - 1].focus(); }
  };
  const path = m.path, item = (text: string, run: () => void) => <button role="menuitem" tabIndex={-1} onClick={() => { closeMenu(false); run(); }}>{text}</button>;
  return (
    <div class={"menu ctx" + (m.on ? " on" : "")} id="proj-menu" ref={ref} role="menu" aria-label={m.name} onKeyDown={keys}>
      {path !== "" && item("Rename", () => renameProject(path))}
      {path !== "" && item("Open folder", () => { api("/api/project", { action: "open", path, what: "folder" }); const row = projRow(path); if (row) row.focus(); })}
      {path !== "" && <hr />}
      {path !== "" && item("Delete project…", () => askDelete(path))}
    </div>
  );
}
