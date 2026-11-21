// Turns a recipe (rbxpromo/recipe.py, version 1) into the composition's props and the layout of every text box.
// Runs in Node (scripts/render.mjs bundles it with esbuild); the composition only draws what this decided, so
// layout.json matches the video exactly.
import metricsJson from './captions/metrics.json';
import {CAPTION_STYLES, type CaptionPreset} from './captions/presets';
import {layoutTitle, paginate, titleBox, type Box, type Highlight, type Metrics, type Page, type TextLayout, type Word} from './captions/paginate';
import {PROMO_STYLES, TRANSITIONS, type PromoStyle} from './styles/index';

const metrics = metricsJson as unknown as Metrics;
export const WIDTH = 1080;
export const HEIGHT = 1920;

export type MediaInfo = {url: string; width: number; height: number; duration: number; hasAudio: boolean; hasVideo: boolean};
export type Zoom = {kind: 'punch' | 'in' | 'out'; t: number};
export type Focus = {t: number; x: number};

type SceneBase = {from: number; frames: number; transIn: string; transOut: string};
export type ClipScene = SceneBase & {
  kind: 'clip';
  src: string;
  srcW: number;
  srcH: number;
  trimBefore: number; // frames (composition fps) into the source
  speed: number;
  cropX: number;
  focus: Focus[] | null;
  zoom: Zoom[];
  volume: number;
};
export type TitleScene = SceneBase & {kind: 'title'; text: string; layout: TextLayout; style: PromoStyle};
export type Scene = ClipScene | TitleScene;

export type EndCard = {from: number; frames: number; title: TextLayout | null; cta: TextLayout | null; ctaBox: Box | null};
export type MusicTrack = {src: string; trimBefore: number; segment: number; volume: number; duck: [number, number][]};
export type PromoProps = {
  fps: number;
  width: number;
  height: number;
  frames: number;
  style: PromoStyle;
  scenes: Scene[];
  captions: {preset: CaptionPreset; pages: Page[]} | null;
  end: EndCard | null;
  music: MusicTrack | null;
  voice: {src: string; frames: number; volume: number} | null;
};
export type LayoutBox = {from: number; to: number; x: number; y: number; w: number; h: number; kind: 'caption' | 'title' | 'end'};

export class PlanError extends Error {}

/** The end card's title animation: typing and count-in read badly over footage, so they rise instead. */
export const endAnim = (a: string) => (a === 'type' || a === 'count' ? 'rise' : a);

const r3 = (v: number) => Math.round(v * 1000) / 1000;

export function parseZoom(z: unknown, where: string): Zoom[] {
  if (z == null || z === '') return [];
  if (typeof z !== 'string') throw new PlanError(`${where}: 'zoom' must be text like "punch@0.4" or "in".`);
  return z.split(',').map((part) => {
    const [kind, at] = part.trim().split('@');
    if (kind !== 'punch' && kind !== 'in' && kind !== 'out')
      throw new PlanError(`${where}: unknown zoom "${part.trim()}". Use punch@<seconds>, in or out (comma-separated).`);
    const t = at === undefined ? 0 : Number(at);
    if (!Number.isFinite(t) || t < 0) throw new PlanError(`${where}: zoom time in "${part.trim()}" must be seconds >= 0.`);
    return {kind, t};
  });
}

export function buildPlan(recipe: any, media: Record<string, MediaInfo>, resolve: (p: string) => string, words: Word[] | null): {props: PromoProps; layout: LayoutBox[]; duration: number} {
  const fps: number = recipe.fps;
  const style = PROMO_STYLES[recipe.style];
  if (!style) throw new PlanError(`Unknown style '${recipe.style}'. Allowed styles: ${Object.keys(PROMO_STYLES).join(', ')}.`);
  const audio = recipe.audio ?? {};
  const music = audio.music ? media[resolve(audio.music)] : null;
  const voice = audio.voice ? media[resolve(audio.voice)] : null;
  const underAudio = !!(music || voice);
  // game sound: audio.clip_volume (0 mutes it), kept at 0.3 of that under music or voice
  const clipVolume = Number(audio.clip_volume ?? 1);

  // scene timing: cumulative rounding keeps the total exact
  let t = 0;
  const raw = recipe.scenes as any[];
  const bounds = raw.map((s) => {
    const d = 'title' in s ? Number(s.duration) : (Number(s.out) - Number(s.in)) / Number(s.speed ?? 1);
    const a = Math.round(t * fps);
    t += d;
    return [a, Math.round(t * fps)];
  });
  const frames = bounds.length ? bounds[bounds.length - 1][1] : 0;
  const trans = raw.map((s, i) => {
    const tr = s.transition ?? (i === 0 ? 'cut' : style.transitions.default);
    if (!TRANSITIONS.includes(tr)) throw new PlanError(`scene ${i}: unknown transition '${tr}'. Allowed: ${TRANSITIONS.join(', ')}.`);
    return i === 0 ? 'cut' : tr;
  });

  const layout: LayoutBox[] = [];
  const scenes: Scene[] = raw.map((s, i) => {
    const [a, b] = bounds[i];
    const base = {from: a, frames: Math.max(1, b - a), transIn: trans[i], transOut: trans[i + 1] ?? 'cut'};
    if ('title' in s) {
      const st = (s.style && PROMO_STYLES[s.style]) || style;
      const tl = layoutTitle(String(s.title), st.title, metrics, WIDTH, HEIGHT, st.title.y, 3);
      layout.push({from: a / fps, to: b / fps, ...titleBox(tl, st.title.stroke, st.title.anim), kind: 'title'});
      return {...base, kind: 'title', text: String(s.title), layout: tl, style: st} as TitleScene;
    }
    const m = media[resolve(s.clip)];
    if (!m || !m.hasVideo) throw new PlanError(`scene ${i}: ${s.clip} has no video track the renderer can read.`);
    const focus = Array.isArray(s.focus)
      ? (s.focus as any[]).filter((k) => Number.isFinite(k?.t) && Number.isFinite(k?.x)).map((k) => ({t: Number(k.t), x: Math.min(1, Math.max(0, Number(k.x)))})).sort((p, q) => p.t - q.t)
      : null;
    return {
      ...base,
      kind: 'clip',
      src: m.url,
      srcW: m.width,
      srcH: m.height,
      trimBefore: Math.round(Number(s.in) * fps),
      speed: Number(s.speed ?? 1),
      cropX: Number(s.crop?.x ?? 0.5),
      focus: focus && focus.length ? focus : null,
      zoom: parseZoom(s.zoom ?? style.rhythm.zoom, `scene ${i}`),
      volume: clipVolume * (underAudio ? 0.3 : 1),
    } as ClipScene;
  });

  // end card over the last seconds
  let end: EndCard | null = null;
  if (recipe.end_card && frames > 0) {
    const n = Math.max(1, Math.round(Math.min(style.end.seconds, (frames / fps) * 0.4) * fps));
    const from = frames - n;
    const tf = {...style.title, size: Math.min(style.title.size, 9), anim: endAnim(style.title.anim)};
    const title = recipe.end_card.title ? layoutTitle(recipe.end_card.title, tf, metrics, WIDTH, HEIGHT, 0.42, 2) : null;
    const ctaFont = {...style.title, size: 3.6, lineHeight: 1.2, letterSpacing: Math.min(style.title.letterSpacing, 0.08), stroke: 0, anim: 'rise'};
    const ctaY = title ? (title.box.y + title.box.h) / HEIGHT + 0.075 : 0.5;
    const cta = recipe.end_card.cta ? layoutTitle(recipe.end_card.cta, ctaFont, metrics, WIDTH, HEIGHT, ctaY, 1, 0.8) : null;
    const ctaBox = cta ? {x: cta.box.x - cta.fontSize * 0.9, y: cta.box.y - cta.fontSize * 0.45, w: cta.box.w + cta.fontSize * 1.8, h: cta.box.h + cta.fontSize * 0.9} : null;
    if (title) layout.push({from: from / fps, to: frames / fps, ...titleBox(title, style.title.stroke, tf.anim), kind: 'end'});
    if (ctaBox) layout.push({from: from / fps, to: frames / fps, ...ctaBox, kind: 'end'});
    end = {from, frames: n, title, cta, ctaBox};
  }

  // captions: never under the end card
  let captions: PromoProps['captions'] = null;
  const caps = recipe.captions;
  if (caps && caps.source !== 'none' && words && words.length) {
    const name = caps.style ?? style.captions;
    const preset = CAPTION_STYLES[name];
    if (!preset) throw new PlanError(`Unknown caption style '${name}'. Allowed: ${Object.keys(CAPTION_STYLES).join(', ')}.`);
    const cuts = bounds.slice(1).map(([a]) => a / fps);
    const pages = paginate(words, {style: preset, metrics, width: WIDTH, height: HEIGHT, cuts, highlight: (caps.highlight ?? 'auto') as Highlight, duration: (end ? end.from : frames) / fps});
    for (const p of pages) layout.push({from: r3(p.start), to: r3(p.end), ...p.box, kind: 'caption'});
    captions = {preset, pages};
  }

  // audio: music loops from music_start, ducks under the voice, fades out at the end
  let musicTrack: MusicTrack | null = null;
  if (music) {
    const start = Math.max(0, Number(audio.music_start ?? 0) || 0);
    const segment = Math.floor((music.duration - start) * fps);
    if (segment < 1) throw new PlanError(`audio music_start (${start} s) is past the end of the music (${music.duration.toFixed(1)} s).`);
    const duck: [number, number][] = voice && audio.duck_music_under_voice !== false ? [[0, Math.round(voice.duration * fps)]] : [];
    musicTrack = {src: music.url, trimBefore: Math.round(start * fps), segment, volume: Number(audio.music_volume ?? 0.8), duck};
  }
  const voiceTrack = voice ? {src: voice.url, frames: Math.round(voice.duration * fps), volume: Number(audio.voice_volume ?? 1)} : null;

  for (const b of layout) {
    b.x = r3(Math.max(0, b.x));
    b.y = r3(Math.max(0, b.y));
    b.w = r3(Math.min(b.w, WIDTH - b.x));
    b.h = r3(Math.min(b.h, HEIGHT - b.y));
    b.from = r3(b.from);
    b.to = r3(b.to);
  }
  layout.sort((p, q) => p.from - q.from);
  const props: PromoProps = {fps, width: WIDTH, height: HEIGHT, frames, style, scenes, captions, end, music: musicTrack, voice: voiceTrack};
  return {props, layout, duration: frames / fps};
}
