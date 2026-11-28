/* Settings, Appearance: light, dark or the computer's own; the colour theme; text size; the sidebar's width.
   Each choice is drawn as a small picture of the window in its own colours: the list on the left, a chat, the
   message box (for "system", half light and half dark). The themes themselves, and putting one on the page, are the
   old page's (they are applied before anything is drawn); this page asks for them through the host. */
import { useStore } from "../store";
import { host } from "../host";
import { setSideW } from "../sidebar/width";
import { Group } from "../ui/Group";
import { SettingsRow } from "../ui/SettingsRow";
import { Segmented } from "../ui/Segmented";

function wire(c: any, clip?: string) {
  const line = "rgb(127 127 127 / 0.25)";
  const cut = clip === "left" ? "clip-path:polygon(0 0, calc(50% - 1px) 0, calc(50% - 1px) 100%, 0 100%);" : clip === "right" ? "clip-path:polygon(calc(50% + 1px) 0, 100% 0, 100% 100%, calc(50% + 1px) 100%);" : "";
  const i = (css: string, inner?: string) => `<i style="${css}">${inner || ""}</i>`;
  const still = "position:static;";
  return i(cut,
    i("inset:0;background:" + c.canvas)
    + i("top:0;bottom:0;left:0;width:22%;background:" + c.sidebar + ";box-shadow:inset -1px 0 0 " + line)
    + i("left:3%;top:8%;height:8%;width:16%;border-radius:6px;background:" + c.surface + ";box-shadow:inset 0 0 0 1px " + line)
    + i("left:3%;top:22%;height:7%;width:16%;border-radius:6px;background:" + c.accentSurface)
    + i("left:3%;top:32%;height:7%;width:16%;border-radius:6px;opacity:0.7;background:" + c.messageSurface)
    + i("left:3%;top:42%;height:7%;width:16%;border-radius:6px;opacity:0.5;background:" + c.messageSurface)
    + i("right:28%;top:11%;height:9%;width:24%;border-radius:8px;background:" + c.messageSurface)
    + i("left:27%;top:28%;height:5%;width:34%;border-radius:4px;background:" + line)
    + i("left:27%;top:38%;height:5%;width:26%;border-radius:4px;background:" + line)
    + i("bottom:8%;left:26%;right:6%;height:15%;display:flex;align-items:center;justify-content:space-between;padding:0 4px;border-radius:6px;background:" + c.surface + ";box-shadow:inset 0 0 0 1px " + line,
      i(still + "height:26%;width:34%;border-radius:999px;opacity:0.7;background:" + line) + i(still + "height:58%;aspect-ratio:1;border-radius:999px;background:" + c.messageAction))
    + i("right:5%;top:8%;height:46%;width:20%;border-radius:8px;background:" + c.surface + ";box-shadow:inset 0 0 0 1px " + line + ", 0 2px 5px rgb(0 0 0 / 0.14)",
      [0, 1, 2].map((row) => i("left:11%;right:11%;top:" + (10 + row * 30) + "%;height:20%;display:flex;align-items:center;gap:4px",
        i(still + "height:26%;aspect-ratio:1;border-radius:999px;opacity:0.55;background:" + (row === 1 ? c.messageAction : line)) + i(still + "height:30%;width:52%;border-radius:4px;background:" + line))).join("")));
}
function Wire({ id, mode }: { id: string; mode: string }) {
  const t = host.themes(); if (!t) return <span class="th-wire" aria-hidden="true"></span>;
  const html = mode === "system" ? wire(t.colors(id, "light"), "left") + wire(t.colors(id, "dark"), "right") : wire(t.colors(id, mode));
  return <span class="th-wire" aria-hidden="true" dangerouslySetInnerHTML={{ __html: html }}></span>;
}
/* A theme's swatch, after T3 Code's theme circles (ThemePreviewCircles.tsx and packages/shared/src/themePreview.ts;
   MIT, see THIRD_PARTY.md): the theme's background, a little lighter or darker, with two soft blurred glows on it, one
   in its accent colour and one in its send button's, at opposite corners. */
const SPEC: Record<string, { target: string; accent: [number, number]; action: [number, number]; mid: number }> = {
  light: { target: "#ffffff", accent: [72, 22], action: [18, 82], mid: 72 },
  dark: { target: "#09090b", accent: [28, 78], action: [82, 18], mid: 62 },
};
function swatchStyle(c: any, mode: string) {
  const s = SPEC[mode];
  return {
    backgroundColor: `color-mix(in oklab, ${c.canvas} 80%, ${s.target})`,
    backgroundImage: `radial-gradient(circle at ${s.accent[0]}% ${s.accent[1]}%, ${c.accent} 0%, color-mix(in oklab, ${c.accent} ${s.mid}%, transparent) 28%, transparent 58%), radial-gradient(circle at ${s.action[0]}% ${s.action[1]}%, color-mix(in oklab, ${c.messageAction} 45%, transparent) 0%, transparent 55%)`,
  };
}
const SUN = <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linecap="round" aria-hidden="true"><circle cx="6" cy="6" r="2.2" /><path d="M6 1v1.2M6 9.8V11M1 6h1.2M9.8 6H11M2.5 2.5l.8.8M8.7 8.7l.8.8M2.5 9.5l.8-.8M8.7 3.3l.8-.8" /></svg>;
const MOON = <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.2" stroke-linejoin="round" aria-hidden="true"><path d="M9.8 7.4A4 4 0 0 1 4.6 2.2a4 4 0 1 0 5.2 5.2Z" /></svg>;
/* The ring round the circle in use, bent out round its sun or moon the way the bars' line runs round their tabs: right on
   the circle's edge (the swatch is 56px across in the middle of the 68px button; the 2px ring covers its outermost 2px)
   and right round the badge (20px, 2px in from the button's corner), joined by two small round curves where the ring
   leaves the circle. Worked out once. */
const RING = (() => {
  const C1 = 34, R = 27, C2 = 56, rb = 11, f = 3;
  const d = Math.SQRT2 * (C2 - C1), ux = Math.SQRT1_2, uy = Math.SQRT1_2;
  const a = ((R + f) ** 2 - (rb + f) ** 2 + d * d) / (2 * d), h = Math.sqrt((R + f) ** 2 - a * a);
  const at = (cx: number, cy: number, tx: number, ty: number, r: number) => { const L = Math.hypot(tx - cx, ty - cy); return [cx + (tx - cx) / L * r, cy + (ty - cy) / L * r]; };
  /* the two curves' middles: one on the circle's right (A), one on its bottom (B) */
  const Fa = [C1 + a * ux + h * uy, C1 + a * uy - h * ux], Fb = [C1 + a * ux - h * uy, C1 + a * uy + h * ux];
  const T1a = at(C1, C1, Fa[0], Fa[1], R), T1b = at(C1, C1, Fb[0], Fb[1], R);
  const T2a = at(C2, C2, Fa[0], Fa[1], rb), T2b = at(C2, C2, Fb[0], Fb[1], rb);
  /* the badge's arc runs clockwise from A's side to B's, round its outside */
  const ang = (x: number, y: number) => Math.atan2(y - C2, x - C2);
  let sweep = ang(T2b[0], T2b[1]) - ang(T2a[0], T2a[1]); if (sweep < 0) sweep += 2 * Math.PI;
  const p = (v: number[]) => v[0].toFixed(2) + " " + v[1].toFixed(2);
  return `M${p(T1b)}A${R} ${R} 0 1 1 ${p(T1a)}A${f} ${f} 0 0 0 ${p(T2a)}A${rb} ${rb} 0 ${sweep > Math.PI ? 1 : 0} 1 ${p(T2b)}A${f} ${f} 0 0 0 ${p(T1b)}Z`;
})();
const Ring = () => <svg class="th-ring" viewBox="0 0 68 68" aria-hidden="true"><path d={RING} /></svg>;
function Swatch({ c, mode }: { c: any; mode: string }) {
  return <span class={"th-sw " + (mode === "light" ? "sw-l" : "sw-d")} aria-hidden="true"><i style={swatchStyle(c, mode)} /></span>;
}
const MODES = [["system", "System", "Follow the computer's light or dark setting"], ["light", "Light", "Use light mode"], ["dark", "Dark", "Use dark mode"]];

export function Appearance() {
  const theme = useStore((s) => s.theme), scale = useStore((s) => s.scale), sideW = useStore((s) => s.sideW);
  const t = host.themes();
  return (
    <>
      <section class="st-sec" id="set-mode" data-find="light dark system appearance">
        <div class="st-sec-head"><h2>Mode</h2></div>
        <div class="th-modes" id="th-modes" role="group" aria-label="Appearance mode">
          {t && MODES.map(([mode, name, label]) => (
            <button class="th-tile" type="button" aria-label={label} aria-pressed={theme.mode === mode ? "true" : "false"} onClick={() => host.setTheme({ id: theme.id, mode })}><Wire id={theme.id} mode={mode} /><span>{name}</span></button>
          ))}
        </div>
      </section>
      <section class="st-sec" id="set-theme" data-find="colour color palette theme">
        <div class="st-sec-head"><h2>Theme</h2></div>
        <div class="th-grid" id="th-grid">
          {t && t.ORDER.map((id) => {
            const on = theme.id === id, name = t.THEMES[id].name;
            /* the card picks the theme and keeps the mode; a circle picks the theme in that mode only */
            return <div class={"th-card" + (on ? " on" : "")} data-theme={id} onClick={() => host.setTheme({ id, mode: theme.mode })}>
              <span class="th-sws">{["light", "dark"].map((mode) => {
                const picked = on && (theme.mode === mode || theme.mode === "system");
                return <button class="th-swb" type="button" aria-pressed={picked ? "true" : "false"} aria-label={"Use " + name + " in " + mode + " mode" + (picked ? ", in use now" : "")} title={"Use " + name + " in " + mode + " mode only"}
                  onClick={(ev) => { ev.stopPropagation(); host.setTheme({ id, mode }); }}>
                  <Swatch c={t.colors(id, mode)} mode={mode} />{picked && <Ring />}{picked && <span class="th-badge">{mode === "light" ? SUN : MOON}</span>}</button>;
              })}</span>
              <span class="th-name"><button type="button" aria-pressed={on ? "true" : "false"} aria-label={"Use the " + name + " theme" + (on ? ", in use now" : "")} onClick={(ev) => { ev.stopPropagation(); host.setTheme({ id, mode: theme.mode }); }}>{name}</button>{id === "fridae" && <small>Fridae's own</small>}</span></div>;
          })}
        </div>
      </section>
      <Group id="set-text" title="Text">
        <SettingsRow id="set-text-size" find="zoom font bigger smaller scale" title="Text size" desc="Ctrl (Cmd on a Mac) and + or - does the same; 0 resets it."
          ctl={<div class="scale-row"><button class="pill" id="sc-down" aria-label="Smaller" onClick={() => host.setScale(scale - 10)}>A&minus;</button><input type="range" id="sc-range" min="70" max="200" step="5" aria-label="Text size" value={scale} onInput={(e) => host.setScale(+(e.target as HTMLInputElement).value)} /><button class="pill" id="sc-up" aria-label="Bigger" onClick={() => host.setScale(scale + 10)}>A+</button><output id="sc-val">{scale + "%"}</output><button class="linkish" id="sc-reset" onClick={() => host.setScale(100)}>Reset</button></div>} />
      </Group>
      <Group id="set-side" title="Sidebar">
        <SettingsRow id="set-side-width" find="sidebar width resize narrow wide projects list" title="Sidebar width" desc="Or drag the sidebar's right edge. A double-click there puts it back."
          ctl={<Segmented id="side-w-seg" label="Sidebar width" pressed value={sideW} onPick={(w) => setSideW(w, true)}
            options={[{ value: 200, label: "Narrow", data: { "data-w": "200" } }, { value: 232, label: "Default", data: { "data-w": "232" } }, { value: 320, label: "Wide", data: { "data-w": "320" } }]} />} />
      </Group>
    </>
  );
}
