/* Overview's graph: each project a square (the logo), its chats around it, each expert tied to the Lead that started
   it, saved chats dim, and the last few steps of anything that is working. Drag to arrange, scroll to zoom,
   double-click to fit.
   It is drawn only while something changes and only while it can be seen: nodes still finding their place, a drag,
   the zoom easing to fit, or a working chat pulsing. Settled and with nothing at work, no frame is asked for at all
   (`og.raf` is 0); a look at what there is to draw every 900 ms (`tick`) and the pointer wake it again. With
   Overview not in front, or the window hidden, no frame is asked for and the look every 900 ms stops (`seen`).
   The layout cools as it goes (`og.heat`): at full strength for two seconds after it starts, then easing off over
   about five more, so it always comes to rest instead of drifting for ever. */
import { store } from "../store";
import { host } from "../host";
import { cssZoom } from "../util";
import { XP_IDLE, pageStart, xpOld, xpWhen } from "../sidebar/fold";
import { isChat } from "../sidebar/model";
import { bornAt, memberOf, splitExpert } from "../sidebar/team";

type Node = { id: string; kind: "project" | "chat" | "expert" | "saved" | "step"; label: string; x: number; y: number; vx: number; vy: number;
  path?: string; lead?: string | null; busy?: boolean; weight?: number; pane?: string; swarm?: string; chat?: string; born?: number; of?: string; newest?: boolean };
type Drag = { node: Node | null; moved: boolean; px0: number; py0: number; px: number; py: number };
const og = { nodes: new Map<string, Node>(), links: [] as [string, string][], zoom: 1, cx: 0, cy: 0, panX: 0, panY: 0, hover: null as Node | null, drag: null as Drag | null,
  raf: 0, timer: 0, mine: false, asleep: false, heat: 1, age: 0, calm: 0, sig: "", shape: "", drawn: 0, frames: 0, last: 0, still: false, dpr: 1 };
/* What a check may look at: how many frames were asked for and drawn, whether one is waiting, and each node with
   where it is in the window. */
export function stats() {
  const b = cv ? space() : null;
  return { frames: og.frames, drawn: og.drawn, waiting: !!og.raf, timer: !!og.timer, asleep: og.asleep, zoom: og.zoom, mine: og.mine,
    nodes: [...og.nodes.values()].map((n) => ({ id: n.id, kind: n.kind, label: n.label, busy: !!n.busy, swarm: n.swarm || "",
      x: b ? ((n.x - og.cx) * og.zoom + b.width / 2 + og.panX) * b.z + b.left : 0, y: b ? ((n.y - og.cy) * og.zoom + b.height / 2 + og.panY) * b.z + b.top : 0 })) };
}
let cv: HTMLCanvasElement | null = null;
/* Overview is in front and the window can be seen: only then is anything drawn or looked for */
const seen = () => !!cv && store.get().nav.view === "all" && !document.hidden;
/* less motion: nothing drifts or pulses. The nodes are put straight where they come to rest, and a working chat is
   bright with a ring that does not move. */
{ const m = window.matchMedia("(prefers-reduced-motion: reduce)"); og.still = m.matches; m.addEventListener("change", () => { og.still = m.matches; poke(); }); }

function wanted() {
  const s = store.get(), chats = s.chats, current = s.nav.chat, saved = s.saved.chats || [], byId = new Map(chats.map((p) => [p.id, p]));
  const want = new Map<string, any>(), links: [string, string][] = [];
  /* A project's Lead is the project itself in the graph: one node, drawn as the logo, that opens the Lead's chat. The
     Lead is the chat leading the project's run, else its newest chat named "Lead: ...". `alias` sends the Lead's own
     node, and every tie to it, to the project's. */
  const alias = new Map<string, string>(), leadOf = new Map<string, string>();
  const as = (id: string) => alias.get(id) || id;
  const add = (id: string, kind: string, label: string, extra?: any) => { if (alias.has(id)) return; want.set(id, Object.assign({ id, kind, label }, extra || {})); };
  const link = (a: string, b: string) => { a = as(a); b = as(b); if (a !== b && want.has(a) && want.has(b)) links.push([a, b]); };
  /* an expert idle for longer than XP_IDLE is left out, as in the side list, unless the side list shows its chat's
     earlier experts */
  const now = Date.now();
  const xpShown = (id: string, parent: string, busy: boolean, when: number) => !parent || busy || id === current || now - when < XP_IDLE || xpOld.has(parent);
  const open = chats.filter((p) => (p.started || p.busy) && (!p.helper || xpShown(p.id, p.parent || p.id, p.busy, xpWhen.get(p.id) || p.when || (p.replay ? 0 : pageStart))));
  const openIds = new Set(open.map((p) => p.id));
  const savedChats = saved.filter((c: any) => !byId.has(c.id) && xpShown(c.id, c.parent, false, xpWhen.get(c.id) || (c.updated || 0) * 1000)).slice(0, 60);
  const isLeadName = (name: string) => /^lead\b/i.test(name || "");
  for (const x of s.projects) {
    const r: any = s.review[x.path], run = r && r.run && r.run.lead;
    const cands = open.filter((p) => !p.helper && p.project === x.path).map((p) => ({ id: p.id, name: p.name, at: bornAt(p.id) }))
      .concat(savedChats.filter((c: any) => !c.parent && (c.project || "") === x.path).map((c: any) => ({ id: c.id, name: c.name, at: bornAt(c.id) })));
    const lead = cands.find((c) => c.id === run) || cands.filter((c) => isLeadName(c.name)).sort((a, b) => b.at - a.at)[0];
    if (!lead) continue;
    leadOf.set(x.path, lead.id);
    alias.set("chat:" + lead.id, "proj:" + x.path); alias.set("saved:" + lead.id, "proj:" + x.path);
  }
  const steps = (id: string) => (s.steps[id] ? s.steps[id].n : 0);
  for (const x of s.projects) {
    const lid = leadOf.get(x.path), lp = lid && byId.get(lid);
    add("proj:" + x.path, "project", x.name, { path: x.path, lead: lid || null, busy: open.some((p) => p.busy && p.project === x.path), weight: lp ? Math.min(6, steps(lp.id) / 3) : 0 });
  }
  if (open.some((p) => isChat(p) && !p.own)) add("proj:", "project", "No project", { path: "", busy: open.some((p) => p.busy && isChat(p) && !p.own) });
  for (const p of open) {
    const label = p.helper ? splitExpert(p.name, p.expert)[0] : p.name, m = p.helper && memberOf(s.teams, p.id);
    /* a member of a team's run: `swarm` is its state there (parked is drawn dimmer, being checked with a ring) */
    add("chat:" + p.id, p.helper ? "expert" : "chat", label, { pane: p.id, busy: !!p.busy, swarm: (m && m.status) || "", weight: Math.min(p.helper ? 4 : 8, steps(p.id) / 3) });
  }
  for (const c of savedChats) add("saved:" + c.id, "saved", c.name, { chat: c.id, weight: Math.min(6, (c.messages || 1) / 2) });
  for (const p of open) {
    if (p.helper && want.has(as("chat:" + p.parent))) link("chat:" + p.parent, "chat:" + p.id);
    else link("proj:" + p.project, "chat:" + p.id);
    if (p.busy) ((s.steps[p.id] || { recent: [] }).recent || []).filter((r) => Date.now() - r.t < 45000).forEach((r, i, all) => { const id = "step:" + p.id + ":" + r.t; add(id, "step", r.label, { born: r.t, of: as("chat:" + p.id), newest: i === all.length - 1 }); links.push([as("chat:" + p.id), id]); });
  }
  for (const c of savedChats) {
    if (c.parent && (openIds.has(c.parent) || want.has(as("saved:" + c.parent)))) link(openIds.has(c.parent) ? "chat:" + c.parent : "saved:" + c.parent, "saved:" + c.id);
    else link("proj:" + (c.project || ""), "saved:" + c.id);
  }
  /* a saved chat of no project, with no square to hang from, is left out */
  for (const id of [...want.keys()]) if (id.startsWith("saved:") && !links.some((l) => l[1] === id)) want.delete(id);
  return { want, links };
}
/* Bring the nodes into line with what there is to show. Says what changed: "layout" when a project, a chat or a tie
   between them came or went (the nodes must find their places again), "look" when only what is drawn did (a step
   taken, a chat starting or stopping work, a name), false when nothing did. */
function sync(): false | "look" | "layout" {
  const { want, links } = wanted();
  for (const id of [...og.nodes.keys()]) if (!want.has(id)) og.nodes.delete(id);
  for (const [id, w] of want) {
    const have = og.nodes.get(id);
    if (have) Object.assign(have, w);
    else {
      /* a new node appears beside what it is tied to, so the picture grows instead of jumping */
      const tie = links.find((l) => l[1] === id && og.nodes.has(l[0])) || links.find((l) => l[0] === id && og.nodes.has(l[1]));
      const near = tie ? og.nodes.get(tie[0] === id ? tie[1] : tie[0])! : null, a = Math.random() * 6.283, d = near ? 40 : 120 + Math.random() * 80;
      og.nodes.set(id, Object.assign({ x: (near ? near.x : 0) + Math.cos(a) * d, y: (near ? near.y : 0) + Math.sin(a) * d, vx: 0, vy: 0 }, w));
    }
  }
  og.links = links.filter((l) => og.nodes.has(l[0]) && og.nodes.has(l[1]));
  const sig = [...want.values()].map((n) => n.id + "|" + n.label + "|" + (n.busy ? 1 : 0) + "|" + (n.newest ? 1 : 0) + "|" + (n.weight || 0).toFixed(1) + "|" + (n.swarm || "")).join("\n") + "\n" + og.links.map((l) => l[0] + ">" + l[1]).join("\n");
  if (sig === og.sig) return false;
  og.sig = sig;
  const shape = [...want.values()].filter((n) => n.kind !== "step").map((n) => n.id).join("\n") + "\n" + og.links.filter((l) => !l[1].startsWith("step:")).map((l) => l[0] + ">" + l[1]).join("\n");
  const kind = shape === og.shape ? "look" : "layout"; og.shape = shape;
  /* the same picture in words, for a reader who cannot see the canvas */
  const all = [...og.nodes.values()], count = (k: string) => all.filter((n) => n.kind === k).length, plural = (n: number, w: string) => n + " " + w + (n === 1 ? "" : "s");
  const busy = all.filter((n) => n.busy && n.kind !== "project").map((n) => n.label);
  if (cv) cv.setAttribute("aria-label", "Graph: " + [plural(count("project"), "project"), plural(count("chat"), "open chat"), plural(count("expert"), "expert"), plural(count("saved"), "saved chat")].join(", ") + ". " + (busy.length ? "Working now: " + busy.slice(0, 8).join(", ") + "." : "Nothing is working."));
  return kind;
}
const NODE = { project: 10, chat: 6, expert: 4, saved: 3, step: 2.5 };
/* a project is drawn as the Fridae logo, turning while anything in it works */
const ogLogo = new Image(); ogLogo.onload = () => poke(); ogLogo.src = "/logo.png";
/* the furthest out the graph goes, by the fit or by the wheel: in a small space (large text, a narrow window) the
   whole of it is still inside */
const ZOOM_MIN = 0.15;
const radius = (n: Node) => NODE[n.kind] + (n.weight || 0) * 0.6;
/* one step of the layout; false once every node has come to rest (and until something wakes it) */
function step() {
  if (og.asleep) return false;
  const nodes = [...og.nodes.values()], k = og.heat;
  for (let i = 0; i < nodes.length; i++) {
    const a = nodes[i];
    a.vx -= a.x * 0.0009 * k; a.vy -= a.y * 0.0009 * k;               /* a gentle pull to the middle */
    for (let j = i + 1; j < nodes.length; j++) {
      const b = nodes[j]; let dx = a.x - b.x, dy = a.y - b.y; const d2 = dx * dx + dy * dy || 0.01;
      const f = Math.min(3, 2600 / d2) * k, d = Math.sqrt(d2); dx /= d; dy /= d;   /* everything pushes everything else away */
      a.vx += dx * f; a.vy += dy * f; b.vx -= dx * f; b.vy -= dy * f;
    }
  }
  for (const [ia, ib] of og.links) {
    const a = og.nodes.get(ia)!, b = og.nodes.get(ib)!; const dx = b.x - a.x, dy = b.y - a.y, d = Math.sqrt(dx * dx + dy * dy) || 0.01;
    const want = b.kind === "step" ? 46 : b.kind === "saved" ? 84 : b.kind === "expert" ? 70 : 140, f = (d - want) * 0.012 * k;   /* a tie holds two nodes at a set distance */
    a.vx += dx / d * f; a.vy += dy / d * f; b.vx -= dx / d * f; b.vy -= dy / d * f;
  }
  let fastest = 0;
  for (const n of nodes) { if (og.drag && og.drag.node === n) { n.vx = n.vy = 0; continue; } n.vx *= 0.86; n.vy *= 0.86; n.x += n.vx; n.y += n.vy; fastest = Math.max(fastest, Math.abs(n.vx), Math.abs(n.vy)); }
  /* cooling: full strength for the first 120 steps after it starts (a drag keeps it warm), then easing off */
  if (og.drag) og.heat = Math.max(og.heat, 0.5); else if (++og.age > 120) og.heat *= 0.985;
  /* at rest: it has cooled right down, or nothing has moved a fiftieth of a pixel for half a second of frames */
  og.calm = fastest < 0.02 ? og.calm + 1 : 0;
  if ((og.heat < 0.01 || og.calm > 30) && !og.drag) { og.asleep = true; for (const n of nodes) n.vx = n.vy = 0; }
  return !og.asleep;
}
/* The graph's space: its width and height in the page's own px (a px of the stylesheet, which the text-size setting
   scales), where it starts in the window, and that scale. Engines disagree on whether an element's rectangle is
   already scaled when the text size is not 100% (its offsetWidth never is), so the rectangle is compared with the
   offset width and brought to window px either way. Everything the graph draws and every pointer place goes
   through this, so names grow with the text size and a press lands on the node under it. */
/* canvas px per page px: the display's density, times the text size (engines differ on whether the density already
   counts a web view's zoom, so it is counted again, up to 4: drawn finer than the screen at worst, never blurred) */
function sharp(density: number) {
  const scale = store.get().scale, z = (scale > 0 ? scale : 100) / 100;
  return cssZoom() !== 1 ? density * z : Math.min(4, Math.max(density, density * z));
}
function space() {
  const el = cv!.parentNode as HTMLElement, r = el.getBoundingClientRect(), z = cssZoom();
  const ratio = el.offsetWidth ? r.width / el.offsetWidth : 1, scaled = Math.abs(ratio - z) <= Math.abs(ratio - 1), up = scaled ? 1 : z;
  return { z, left: r.left * up, top: r.top * up, width: r.width * up / z, height: r.height * up / z };
}
/* draw it once; true while the zoom is still easing towards the fit */
function draw(now: number) {
  const box0 = space(), dpr = sharp(og.dpr = window.devicePixelRatio || 1), INK = host.ink() || "236,236,238";
  if (!cv || !box0.width || !box0.height) return false;
  if (cv.width !== Math.round(box0.width * dpr) || cv.height !== Math.round(box0.height * dpr)) { cv.width = Math.round(box0.width * dpr); cv.height = Math.round(box0.height * dpr); }
  const c = cv.getContext("2d")!; c.setTransform(dpr, 0, 0, dpr, 0, 0); c.clearRect(0, 0, box0.width, box0.height);
  /* names are 12px on the screen whatever the zoom (13px for a project), growing only when zoomed well in */
  const FONT = "px Geist, 'Segoe UI', sans-serif", named = (n: Node) => n.kind !== "saved" && n.kind !== "step";
  const short = (n: Node) => (n.label.length > 30 ? n.label.slice(0, 29) + "…" : n.label), px = (n: Node) => (n.kind === "project" ? 13 : 12);
  let easing = false;
  if (!og.mine && og.nodes.size && !og.drag) {
    /* fit: the middle of what there is goes to the middle of the space, at the zoom that keeps every node, and
       every name that always shows, inside it with a margin (held still while a node is dragged, so the picture
       does not slide under the pointer). The margin keeps clear of the line over the top, the "?" in the corner
       and a name under its node */
    let x0 = Infinity, x1 = -Infinity, y0 = Infinity, y1 = -Infinity;
    for (const n of og.nodes.values()) { x0 = Math.min(x0, n.x); x1 = Math.max(x1, n.x); y0 = Math.min(y0, n.y); y1 = Math.max(y1, n.y); }
    const mx = (x0 + x1) / 2, my = (y0 + y1) / 2;
    let fit = 1.8;
    for (const n of og.nodes.values()) {
      let half = 12;
      if (named(n)) { c.font = (n.kind === "project" ? "500 " : "") + px(n) + FONT; half = c.measureText(short(n)).width / 2 + 16; }
      if (Math.abs(n.x - mx) > 1) fit = Math.min(fit, (box0.width / 2 - half) / Math.abs(n.x - mx));
      if (Math.abs(n.y - my) > 1) fit = Math.min(fit, (box0.height / 2 - 60) / (Math.abs(n.y - my) + radius(n)));
    }
    fit = Math.max(ZOOM_MIN, fit);
    const near = Math.abs(fit - og.zoom) < 0.002 && Math.abs(mx - og.cx) < 0.5 && Math.abs(my - og.cy) < 0.5;
    if (og.still || near) { og.zoom = fit; og.cx = mx; og.cy = my; }
    else { og.zoom += (fit - og.zoom) * 0.06; og.cx += (mx - og.cx) * 0.06; og.cy += (my - og.cy) * 0.06; easing = true; }
  }
  c.translate(box0.width / 2 + og.panX, box0.height / 2 + og.panY); c.scale(og.zoom, og.zoom); c.translate(-og.cx, -og.cy);
  const near = new Set<string>(); if (og.hover) { const h = og.hover.id; near.add(h); og.links.forEach(([a, b]) => { if (a === h) near.add(b); if (b === h) near.add(a); }); }
  const lit = (id: string) => !og.hover || near.has(id);
  c.lineWidth = 1 / og.zoom;
  for (const [ia, ib] of og.links) {
    const a = og.nodes.get(ia)!, b = og.nodes.get(ib)!, on = og.hover && near.has(ia) && near.has(ib);
    c.strokeStyle = "rgba(" + INK + "," + (on ? 0.5 : og.hover ? 0.04 : a.busy || b.busy ? 0.22 : 0.1) + ")";
    c.beginPath(); c.moveTo(a.x, a.y); c.lineTo(b.x, b.y); c.stroke();
  }
  const pulse = og.still ? 0.5 : 0.5 + 0.5 * Math.sin(now / 420), size = (p: number) => p / Math.min(1.4, og.zoom);
  const names: any[] = [];
  for (const n of og.nodes.values()) {
    const r = Math.max(radius(n), 1.5 / og.zoom), dim = lit(n.id) ? 1 : 0.18;   /* never under 1.5px on the screen */
    const parked = n.swarm === "parked" && !n.busy;   /* a parked member rests: drawn dimmer, its name too */
    const shade = (n.kind === "saved" ? 0.3 : n.kind === "step" ? 0.55 : n.busy ? 1 : n.kind === "chat" || n.kind === "expert" ? 0.62 : 0.85) * (parked ? 0.5 : 1);
    if (n.busy) { c.fillStyle = "rgba(" + INK + "," + (0.05 + 0.07 * pulse) * dim + ")"; c.beginPath(); c.arc(n.x, n.y, r + 7 + 5 * pulse, 0, 6.283); c.fill(); }
    c.fillStyle = c.strokeStyle = "rgba(" + INK + "," + shade * dim + ")";
    if (n.kind === "project" && ogLogo.complete && ogLogo.naturalWidth) {
      const s = r * 2.6;
      c.save(); c.globalAlpha = (n.busy ? 1 : 0.9) * dim; c.translate(n.x, n.y);
      if (n.busy && !og.still) c.rotate((now / 1600) % 6.2832);   /* one turn every ten seconds */
      c.drawImage(ogLogo, -s / 2, -s / 2, s, s); c.restore();
    } else {
      c.beginPath();
      if (n.kind === "project") c.rect(n.x - r, n.y - r, r * 2, r * 2); else c.arc(n.x, n.y, r, 0, 6.283);
      c.fill();
    }
    /* a member whose work is being checked: one thin, still ring around it */
    if (n.swarm === "verifying") { c.strokeStyle = "rgba(" + INK + "," + 0.5 * dim + ")"; c.beginPath(); c.arc(n.x, n.y, r + 3.5, 0, 6.283); c.stroke(); }
    /* names: always for a project, a chat and an expert; for a saved chat under the pointer or zoomed well in; for
       a step, the one its chat is on now (all of its last steps under the pointer or zoomed in). They are drawn
       after the dots, the more important first. */
    const wordy = named(n) || n === og.hover || og.zoom > 1.5 || (n.kind === "step" && lit(n.id) && (n.newest || !!og.hover || og.zoom >= 1));
    if (!wordy) continue;
    const l: any = { text: short(n), font: (n.kind === "project" ? "500 " : "") + size(px(n)) + FONT, fill: "rgba(" + INK + "," + (n === og.hover || n.busy || n.kind === "project" ? 0.95 : n.kind === "step" ? 0.55 : parked ? 0.4 : 0.62) * dim + ")",
      rank: n === og.hover ? 0 : n.kind === "project" ? 1 : n.kind === "chat" ? (n.busy ? 2 : 3) : n.kind === "expert" ? 4 : n.kind === "step" ? (n.newest ? 5 : 6) : 7 };
    const chat = n.kind === "step" && og.nodes.get(n.of!);
    if (chat) {
      /* a step's name sits beside it, on the side away from its chat, so it never lies across the chat's own */
      const side = n.x < chat.x ? -1 : 1;
      l.align = side < 0 ? "right" : "left"; l.base = "middle"; l.x = n.x + side * (r + size(6)); l.y = n.y;
    } else { l.align = "center"; l.base = "alphabetic"; l.x = n.x; l.y = n.y + r + size(14); }
    names.push(l);
  }
  /* a name that would lie across one already drawn is left out; the node under the pointer always shows its own */
  const taken: number[][] = [], tall = size(15);
  names.sort((a, b) => a.rank - b.rank);
  for (const l of names) {
    c.font = l.font;
    const wide = c.measureText(l.text).width, x0 = l.align === "center" ? l.x - wide / 2 : l.align === "right" ? l.x - wide : l.x, y0 = l.base === "middle" ? l.y - tall / 2 : l.y - tall * 0.8;
    if (l.rank > 0 && taken.some((t) => x0 < t[2] && x0 + wide > t[0] && y0 < t[3] && y0 + tall > t[1])) continue;
    taken.push([x0, y0, x0 + wide, y0 + tall]);
    c.fillStyle = l.fill; c.textAlign = l.align; c.textBaseline = l.base; c.fillText(l.text, l.x, l.y);
  }
  c.textBaseline = "alphabetic";
  og.drawn++;
  return easing;
}
/* A frame: move, draw, and ask for the next one only if there is a reason to. */
function frame(now: number) {
  og.raf = 0; og.frames++;
  if (!seen()) return;
  let moving;
  if (og.still && !og.drag) { for (let i = 0; i < 400 && step(); i++); moving = !og.asleep; if (moving) { og.raf = requestAnimationFrame(frame); return; } }   /* straight to rest, then drawn once */
  else moving = step();
  const pulsing = !og.still && [...og.nodes.values()].some((n) => n.busy);
  /* a pulse alone needs no more than thirty frames a second */
  const easing = !moving && pulsing && !og.drag && now - og.last < 30 ? false : (og.last = now, draw(now));
  if (moving || easing || pulsing || og.drag) og.raf = requestAnimationFrame(frame);
}
/* draw once more (the pointer moved over it, it was zoomed or panned, its space changed) */
export function poke() { if (seen() && !og.raf) og.raf = requestAnimationFrame(frame); }
/* the layout has something to do again (a node came or went, one was dragged) */
function wake() { og.asleep = false; og.calm = 0; og.heat = Math.max(og.heat, 0.3); poke(); }
/* is there something else to show? Every 900 ms while it can be seen, and whenever the store says something changed */
export function tick() {
  if (!seen()) return;
  if (og.dpr !== (window.devicePixelRatio || 1)) poke();   /* the display's density changed without a word */
  const changed = sync();
  /* with less motion the nodes are put straight to rest, which is only worth doing when the layout itself changed:
     a step taken by a working chat just appears beside it */
  if (changed === "layout" || (changed && !og.still)) wake(); else if (changed) poke();
}
/* Overview came to the front, or the window came back: the first time, the layout starts from scratch at full
   strength; coming back to it, only what changed stirs it. Out of sight, nothing runs at all. */
export function watch() {
  if (!seen()) { if (og.raf) cancelAnimationFrame(og.raf); og.raf = 0; clearInterval(og.timer); og.timer = 0; return; }
  const first = !og.nodes.size;
  if (sync()) { if (first) { og.heat = 1; og.age = 0; og.asleep = false; og.calm = 0; } else wake(); }
  og.last = 0; poke();
  if (!og.timer) og.timer = setInterval(tick, 900) as unknown as number;
}
/* where the pointer is, in the graph's own coordinates, and which node is under it */
function at(ev: MouseEvent) {
  const b = space(), px = (ev.clientX - b.left) / b.z, py = (ev.clientY - b.top) / b.z;   /* in the page's own px, from the graph's corner */
  const x = (px - b.width / 2 - og.panX) / og.zoom + og.cx, y = (py - b.height / 2 - og.panY) / og.zoom + og.cy;
  let hit: Node | null = null, best = 1e9;
  for (const n of og.nodes.values()) { const d = Math.hypot(n.x - x, n.y - y); if (d < radius(n) + 7 / og.zoom && d < best) { best = d; hit = n; } }
  return { x, y, px, py, hit };
}
/* a press that never got its release (a menu opened over it, the window lost the keyboard) is over */
function drop() { if (!og.drag) return; const d = og.drag; og.drag = null; if (d.moved && d.node) wake(); else poke(); }

/* the graph is drawn on this canvas from now on; what comes back lets go of it */
export function attach(canvas: HTMLCanvasElement) {
  cv = canvas;
  const off: (() => void)[] = [];
  const on = (t: EventTarget, name: string, f: (ev: any) => void, how?: AddEventListenerOptions) => { t.addEventListener(name, f, how); off.push(() => t.removeEventListener(name, f, how)); };
  on(canvas, "mousemove", (ev: MouseEvent) => {
    const here = at(ev), d = og.drag;
    if (d) {
      /* a press becomes a drag once the pointer has gone three px from where it went down; until then it is still a
         press, and letting go opens what is under it */
      if (!d.moved && Math.hypot(here.px - d.px0, here.py - d.py0) <= 3) return;
      d.moved = true;
      if (d.node) { d.node.x = here.x; d.node.y = here.y; wake(); } else { og.panX += here.px - d.px; og.panY += here.py - d.py; og.mine = true; poke(); }
      d.px = here.px; d.py = here.py; return;
    }
    if (og.hover !== here.hit) { og.hover = here.hit; poke(); }
    canvas.classList.toggle("over", !!here.hit && here.hit.kind !== "step");
  });
  /* the left button only: any other leaves the graph alone (and opens nothing) */
  on(canvas, "mousedown", (ev: MouseEvent) => { if (ev.button !== 0) return; const here = at(ev); og.drag = { node: here.hit, moved: false, px0: here.px, py0: here.py, px: here.px, py: here.py }; });
  on(canvas, "contextmenu", drop);
  on(window, "blur", drop);
  on(window, "mouseup", (ev: MouseEvent) => {
    if (ev.button !== 0) return;
    const d = og.drag; og.drag = null; if (!d) return;
    if (d.moved) { if (d.node) wake(); return; }   /* a dragged node is let go: the rest settle around it */
    if (!d.node) return;
    const n = d.node;
    /* a project opens its Lead's chat, or its Review when it has no Lead; a chat opens in Chat, an expert too */
    if (n.kind === "project") { if (n.lead) host.openChatById(n.lead); else if (n.path) host.openInReview(n.path); else host.showProject(n.path || "", "chat"); }
    else if (n.kind === "chat" || n.kind === "expert") host.openChatById(n.pane!);
    else if (n.kind === "saved") host.openChatById(n.chat!);
  });
  on(canvas, "mouseleave", () => { if (og.hover) { og.hover = null; poke(); } });
  on(canvas, "dblclick", () => { og.mine = false; og.panX = og.panY = 0; poke(); });   /* double-click: fit it to the space again */
  /* The wheel zooms the graph, down to the smallest zoom the fit itself may choose (ZOOM_MIN). A pinch (the wheel
     with Ctrl or Command) always does, and stops here, so it never also changes the page's text size. */
  on(canvas, "wheel", (ev: WheelEvent) => {
    const pinch = ev.ctrlKey || ev.metaKey;
    ev.preventDefault(); ev.stopPropagation();
    const by = pinch ? Math.max(0.8, Math.min(1.25, Math.exp(-ev.deltaY * 0.01))) : ev.deltaY < 0 ? 1.1 : 0.9;
    og.mine = true; og.zoom = Math.max(ZOOM_MIN, Math.min(3, og.zoom * by)); poke();
  }, { passive: false });
  const ro = new ResizeObserver(poke); ro.observe(canvas.parentNode as Element); off.push(() => ro.disconnect());
  /* a sharper or coarser display (the window moved to another screen): draw again at its density */
  let live = true;
  const density = () => { const m = window.matchMedia("(resolution: " + (window.devicePixelRatio || 1) + "dppx)"); m.addEventListener("change", () => { if (!live) return; poke(); density(); }, { once: true }); };
  density(); off.push(() => { live = false; });
  /* the window was hidden or shown again; the colours changed */
  on(document, "visibilitychange", watch);
  on(window, "themechange", poke);
  watch();
  return () => { off.forEach((f) => f()); if (og.raf) cancelAnimationFrame(og.raf); og.raf = 0; clearInterval(og.timer); og.timer = 0; cv = null; };
}
