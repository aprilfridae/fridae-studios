/* The Settings window: the list of pages on the left with a search over every row of every page, the page's name at
   the top, and the pages. A narrow window shows the list as a rail of icons (the page's CSS does that).
   Search looks through every section and row of every page (their titles, the page's name, and a few extra words in
   data-find), best match first; picking one opens its page, brings the row to the middle and pulses its edge twice.
   "/" or Ctrl+F puts the cursor in the search box while Settings is open. The search, its ranking and the jump are
   adapted from T3 Code's settings (MIT; see THIRD_PARTY.md). */
import { useEffect, useRef, useState } from "preact/hooks";
import { useStore } from "../store";
import { host } from "../host";
import { closeSetup } from "../windows";
import { Chevron, Lens, X } from "../ui/icons";
import { NAV, PAGES } from "./nav";
import { showPage, whenPageChanges } from "./state";
import { skillsWaiting } from "./skills-data";
import { General } from "./General";
import { Appearance } from "./Appearance";
import { Remote } from "./Remote";
import { Connections } from "./Connections";
import { Experts } from "./Experts";
import { Permissions } from "./Permissions";
import { Images } from "./Images";
import { Skills } from "./Skills";
import { Knowledge } from "./Knowledge";
import { Chats } from "./Chats";
import { Guide } from "./Guide";
import { About } from "./About";

type Item = { id: string; to: string; where: string; title: string; terms: string };
const norm = (t: string) => String(t || "").toLowerCase().normalize("NFKD").replace(/[^a-z0-9]+/g, " ").trim();
/* everything that can be found: each page, and each titled section and row on it that is not hidden */
function items(root: HTMLElement): Item[] {
  const out: Item[] = [];
  root.querySelectorAll<HTMLElement>(".su-body > section[data-s]").forEach((page) => {
    const to = page.dataset.s!, where = page.dataset.name!;
    out.push({ id: "", to, where, title: where, terms: "" });
    page.querySelectorAll<HTMLElement>(".st-sec[id], .sr[id]").forEach((t) => {
      if (t.closest("[hidden]")) return;
      const h = t.matches(".st-sec") ? t.querySelector(".st-sec-head h2") : t.querySelector(".sr-text h3");
      if (!h) return;
      /* a heading's own text, without a count beside it */
      const title = [...h.childNodes].filter((n) => n.nodeType === 3 || (n.nodeType === 1 && (n as Element).tagName === "LABEL")).map((n) => n.textContent).join(" ").trim();
      if (title && !(title === where && !t.dataset.find)) out.push({ id: t.id, to, where, title, terms: t.dataset.find || "" });
    });
  });
  return out;
}
function search(root: HTMLElement, query: string): Item[] {
  const q = norm(query); if (!q) return [];
  const tokens = q.split(" ");
  return items(root).flatMap((item, index) => {
    const title = norm(item.title), fields = [title, norm(item.where), norm(item.terms)];
    if (!tokens.every((tok) => fields.some((f) => f.includes(tok)))) return [];
    const rank = title === q ? 5 : title.startsWith(q) ? 4 : title.includes(q) ? 3 : tokens.every((tok) => title.includes(tok)) ? 2 : fields.some((f) => f.includes(q)) ? 1 : 0;
    return [{ item, index, rank }];
  }).sort((a, b) => b.rank - a.rank || a.index - b.index).map((x) => x.item);
}
/* bring a row to the middle, hand it the keyboard, and pulse its edge until the keyboard leaves it */
function scrollTo(target: HTMLElement) {
  const calm = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const marked = target.querySelector(":scope > [data-settings-scroll-target]");
  const to = marked || (target.tagName === "SECTION" && target.firstElementChild ? target.firstElementChild : target);
  to.scrollIntoView({ behavior: calm ? "auto" : "smooth", block: "center" });
  if (!target.hasAttribute("tabindex")) target.setAttribute("tabindex", "-1");
  target.focus({ preventScroll: true });
  target.classList.remove("settings-search-target-pulse");
  if (calm) return;
  void target.offsetWidth;
  target.classList.add("settings-search-target-pulse");
  target.addEventListener("blur", () => target.classList.remove("settings-search-target-pulse"), { once: true });
}

export function Settings() {
  const { page, n, open } = useStore((s) => s.settings), status = useStore((s) => s.status);
  const waiting = skillsWaiting.use();
  const [query, setQuery] = useState(""), [at, setAt] = useState(0), [focused, setFocused] = useState(false);
  const input = useRef<HTMLInputElement>(null), body = useRef<HTMLDivElement>(null), nav = useRef<HTMLDivElement>(null);
  const searching = query.trim().length > 0;
  const root = document.getElementById("sd"), found = searching && root ? search(root, query) : [];
  const sel = Math.min(at, Math.max(found.length - 1, 0));
  const clear = () => { setQuery(""); setAt(0); };
  const here = PAGES.find((p) => p.id === page) || PAGES[0];

  useEffect(() => whenPageChanges(() => { if (body.current) body.current.scrollTop = 0; }), []);
  /* opened: the search is empty again and the keyboard is on the page's button in the list */
  useEffect(() => { if (n) { clear(); setFocused(false); host.panelOpened(nav.current && nav.current.querySelector("button.on")); } }, [n]);
  useEffect(() => { const b = document.getElementById("st-r-" + sel); if (b) b.scrollIntoView({ block: "nearest" }); }, [sel, query]);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (!open || e.defaultPrevented || e.isComposing) return;
      const find = e.key.toLowerCase() === "f" && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey;
      const slash = e.key === "/" && !e.metaKey && !e.ctrlKey && !e.altKey;
      if (!find && !slash) return;
      const t = e.target;
      if (slash && t instanceof HTMLElement && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.tagName === "SELECT" || t.isContentEditable)) return;
      e.preventDefault(); setFocused(true); input.current!.focus(); input.current!.select();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [open]);

  const go = (item: Item) => {
    clear(); setFocused(false);
    if (item.to !== page) showPage(item.to);
    /* the page is drawn before the next frame, so the row is there to scroll to */
    requestAnimationFrame(() => {
      const target = item.id ? document.getElementById(item.id) : null;
      if (target) scrollTo(target); else (nav.current!.querySelector('[data-s="' + item.to + '"]') as HTMLElement).focus({ preventScroll: true });
    });
  };
  const searchKey = (e: KeyboardEvent) => {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Escape" && (query || e.repeat)) { e.preventDefault(); e.stopPropagation(); clear(); return; }
    if (!found.length) return;
    if (e.key === "ArrowDown") { e.preventDefault(); setAt((sel + 1) % found.length); }
    else if (e.key === "ArrowUp") { e.preventDefault(); setAt((sel - 1 + found.length) % found.length); }
    else if (e.key === "Enter") { e.preventDefault(); if (found[sel]) go(found[sel]); }
  };
  /* the arrow keys walk the list of pages without opening them */
  const navKey = (e: KeyboardEvent) => {
    const t = e.target;
    if (e.defaultPrevented || e.altKey || e.ctrlKey || e.metaKey || !(t instanceof HTMLElement) || !t.matches("[data-s]")) return;
    const all = [...nav.current!.querySelectorAll<HTMLElement>("[data-s]")], i = all.indexOf(t);
    const next = e.key === "ArrowDown" ? Math.min(i + 1, all.length - 1) : e.key === "ArrowUp" ? Math.max(i - 1, 0) : e.key === "Home" ? 0 : e.key === "End" ? all.length - 1 : -1;
    if (next < 0) return;
    e.preventDefault(); all[next].scrollIntoView({ block: "nearest" }); all[next].focus({ preventScroll: true });
  };
  const version = "version" in status || "ai" in status ? "Version " + (status.version || "") : "";
  const sec = (id: string, name: string, Page: () => any) => <section data-s={id} data-name={name} class={page === id ? "on" : ""}><div class="st-page"><Page /></div></section>;

  return (
    <>
      <nav class={"st-nav" + (focused || searching ? " searching" : "")} aria-label="Settings sections">
        <div class="st-nav-in">
          <div class="st-search">
            {Lens}
            <input type="text" id="st-q" ref={input} value={query} placeholder="Search" aria-label="Search settings" role="combobox" aria-autocomplete="list" aria-expanded={found.length ? "true" : "false"}
              aria-controls={found.length ? "st-results" : undefined} aria-activedescendant={found[sel] ? "st-r-" + sel : undefined} autocomplete="off" spellcheck={false}
              onInput={(e) => { setAt(0); setQuery((e.target as HTMLInputElement).value); }} onFocus={() => setFocused(true)} onBlur={() => { if (!input.current!.value.trim()) setFocused(false); }} onKeyDown={searchKey} />
            <button class="st-clear" id="st-clear" aria-label="Clear settings search" hidden={!searching} onClick={() => { clear(); input.current!.focus(); }}>{X}</button>
            <kbd id="st-kbd" hidden={searching}>/</kbd>
          </div>
          <p class="st-none" id="st-none" role="status" hidden={!searching || found.length > 0}>No settings found</p>
          <ul class="st-menu" id="st-results" role="listbox" aria-label="Settings search results" hidden={!found.length}>
            {found.map((item, i) => (
              <li role="presentation"><button id={"st-r-" + i} role="option" tabindex={-1} aria-selected={i === sel ? "true" : "false"} onMouseMove={() => { if (sel !== i) setAt(i); }} onClick={() => go(item)}>
                {(PAGES.find((p) => p.id === item.to) || PAGES[0]).icon}<span><b>{item.title}</b><small>{item.where}</small></span></button></li>
            ))}
          </ul>
          <ul class="st-menu st-rail-search"><li><button id="st-open" aria-label="Search settings" onClick={() => { setFocused(true); input.current!.focus(); }}>{Lens}</button></li></ul>
          <div class="st-groups" id="su-nav" ref={nav} hidden={searching} onKeyDown={navKey}>
            {NAV.map((g, gi) => (
              <div class="st-group"><h3 id={"st-g" + gi}>{g.title}</h3><ul class="st-menu" role="group" aria-labelledby={"st-g" + gi}>
                {g.pages.map((p) => (
                  <li><button data-s={p.id} class={page === p.id ? "on" : ""} aria-current={page === p.id ? "page" : undefined} onClick={() => showPage(p.id)}>{p.icon}<span>{p.name}</span>
                    {p.id === "skills" && <span class="su-n" id="sk-nav-n" hidden={!waiting} title={waiting ? waiting + (waiting === 1 ? " decision waits" : " decisions wait") : undefined}>{waiting || ""}</span>}</button></li>
                ))}
              </ul></div>
            ))}
          </div>
        </div>
        <div class="st-nav-foot"><span class="ver" id="su-ver">{version}</span></div>
      </nav>
      <div class="st-main">
        <header class="st-head">
          <nav aria-label="Settings breadcrumb"><ol class="st-crumb"><li>Settings</li><li aria-hidden="true" class="st-sep">{Chevron}</li><li id="st-here" aria-current="page">{here.name}</li></ol></nav>
          <span class="grow"></span>
          <button class="icon su-close" id="b-close" aria-label="Close" onClick={() => closeSetup()}>{X}</button>
        </header>
        <div class="su-body" data-settings-page-scroll="" ref={body}>
          {sec("general", "General", General)}
          {sec("appearance", "Appearance", Appearance)}
          {sec("remote", "Other computers", Remote)}
          {sec("connections", "Connections", Connections)}
          {sec("experts", "Experts", Experts)}
          {sec("permissions", "Permissions", Permissions)}
          {sec("images", "Images", Images)}
          {sec("skills", "Skills", Skills)}
          {sec("knowledge", "Knowledge", Knowledge)}
          {sec("chats", "Chats", Chats)}
          {sec("guide", "Guide", Guide)}
          {sec("about", "About", About)}
        </div>
      </div>
    </>
  );
}
