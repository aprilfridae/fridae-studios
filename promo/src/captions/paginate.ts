// Caption pagination and text layout for promos, shared by the Remotion composition and scripts/render.mjs
// (which writes layout.json from the same code). Caption rules adapted from Subtle's scripts/group.py and
// scripts/spoken_numbers.py (https://github.com/0xpratzyy/subtle):
// Copyright (c) 2026 Refs (refs.video) — MIT License (see licenses/subtle-LICENSE.txt).
// Rules kept: numbers as one token, one highlight per page, no page across a cut, break on pauses and sentence
// ends, pages sized from real glyph widths. Rewritten in TypeScript without a browser: widths come from
// metrics.json (per-character advances of the bundled fonts), so Node can lay out exactly what Chrome draws.
//
// Plain TypeScript only (no enums, namespaces or parameter properties): Node's type stripping runs the tests.
import type {CaptionCase, CaptionPreset} from './presets.ts';

/** Where text may go, as fractions of the frame: clear of TikTok/Reels/Shorts top bar, right rail and caption bar. */
export const SAFE = {x0: 0.12, x1: 0.84, y0: 0.1, y1: 0.78};

export type Word = {text: string; start: number; end: number};
export type Token = Word & {number?: boolean};
export type FontMetrics = {default: number; adv: Record<string, number>};
export type Metrics = Record<string, FontMetrics>;
export type Box = {x: number; y: number; w: number; h: number};
export type Highlight = 'auto' | 'numbers' | 'off';

export type PageWord = {
  text: string; // as drawn (case applied)
  start: number;
  end: number;
  emphasis: boolean;
  number: boolean;
  line: number;
  x: number; // left edge, px
  w: number; // width, px
  fs: number; // font size of this word, px
};
export type Page = {
  index: number;
  start: number;
  end: number;
  words: PageWord[];
  lines: number;
  fontSize: number; // base font size, px
  lineHeight: number; // px
  top: number; // top of the text block, px
  blockW: number;
  box: Box; // text block plus container padding (and tilt), px — what layout.json reports
  tilt: number; // degrees (ticket style)
};

export const applyCase = (text: string, c: CaptionCase) =>
  c === 'upper' ? text.toUpperCase() : c === 'lower' ? text.toLowerCase() : text;

export function textWidth(text: string, file: string, fs: number, ls: number, metrics: Metrics): number {
  const m = metrics[file];
  let w = 0;
  let n = 0;
  for (const ch of text) {
    w += m?.adv[ch] ?? m?.default ?? 0.6;
    n++;
  }
  return (w + ls * n) * fs;
}

// ---------------------------------------------------------------- numbers as one token

const UNITS: Record<string, number> = {
  zero: 0, one: 1, two: 2, three: 3, four: 4, five: 5, six: 6, seven: 7, eight: 8, nine: 9, ten: 10,
  eleven: 11, twelve: 12, thirteen: 13, fourteen: 14, fifteen: 15, sixteen: 16, seventeen: 17, eighteen: 18,
  nineteen: 19,
};
const TENS: Record<string, number> = {twenty: 20, thirty: 30, forty: 40, fifty: 50, sixty: 60, seventy: 70, eighty: 80, ninety: 90};
const SCALES: Record<string, number> = {thousand: 1e3, million: 1e6, billion: 1e9};
const PREFIX_UNITS: Record<string, string> = {dollars: '$', dollar: '$', bucks: '$', euros: '€', euro: '€', pounds: '£'};
const SUFFIX_UNITS: Record<string, string> = {percent: '%', robux: ' Robux'};

const split = (raw: string) => {
  const m = /^([("'“‘¿¡]*)(.*?)([.,!?;:)"'”’…]*)$/u.exec(raw.trim());
  return m ? {lead: m[1], core: m[2], trail: m[3]} : {lead: '', core: raw.trim(), trail: ''};
};

/** Value of one spoken number word ("twenty-five" counts as one), or null. */
function wordValue(core: string): {kind: 'unit' | 'tens' | 'hundred' | 'scale'; v: number} | {kind: 'compound'; v: number} | null {
  const w = core.toLowerCase();
  if (w in UNITS) return {kind: 'unit', v: UNITS[w]};
  if (w in TENS) return {kind: 'tens', v: TENS[w]};
  if (w === 'hundred') return {kind: 'hundred', v: 100};
  if (w in SCALES) return {kind: 'scale', v: SCALES[w]};
  const h = /^([a-z]+)-([a-z]+)$/.exec(w);
  if (h && h[1] in TENS && h[2] in UNITS && UNITS[h[2]] < 10) return {kind: 'compound', v: TENS[h[1]] + UNITS[h[2]]};
  return null;
}

const DIGITS = /^\d{1,3}(,\d{3})+(\.\d+)?$|^\d+(\.\d+)?$/;
const commas = (n: number) => {
  const [int, frac] = String(n).split('.');
  return int.replace(/\B(?=(\d{3})+(?!\d))/g, ',') + (frac ? '.' + frac : '');
};

/** Spoken numbers become one token in digits: ["five","thousand","dollars"] -> "$5,000". */
export function mergeNumbers(words: Word[]): Token[] {
  const out: Token[] = [];
  let i = 0;
  while (i < words.length) {
    const first = split(words[i].text);
    let j = i;
    let value: number | null = null;
    let fromWords = false;
    if (DIGITS.test(first.core)) {
      value = Number(first.core.replace(/,/g, ''));
      j = i + 1;
    } else {
      let total = 0;
      let current = 0;
      let k = i;
      let lastKind = '';
      while (k < words.length) {
        const s = split(words[k].text);
        if (k > i && split(words[k - 1].text).trail) break; // punctuation ends the number
        const core = s.core.toLowerCase();
        if (core === 'and' && (lastKind === 'hundred' || lastKind === 'scale') && k + 1 < words.length) {
          const nx = wordValue(split(words[k + 1].text).core);
          if (nx && (nx.kind === 'unit' || nx.kind === 'tens' || nx.kind === 'compound')) {
            k++;
            continue;
          }
          break;
        }
        const wv = wordValue(s.core);
        if (!wv) break;
        if (wv.kind === 'hundred') current = (current || 1) * 100;
        else if (wv.kind === 'scale') {
          total += (current || 1) * wv.v;
          current = 0;
        } else current += wv.v;
        lastKind = wv.kind;
        k++;
      }
      if (k > i) {
        value = total + current;
        fromWords = true;
        j = k;
      }
    }
    if (value === null) {
      out.push({...words[i]});
      i++;
      continue;
    }
    // a unit word right after the number joins it
    let prefix = '';
    let suffix = '';
    let unitUsed = false;
    const lastTrail = split(words[j - 1].text).trail;
    if (j < words.length && !lastTrail) {
      const u = split(words[j].text).core.toLowerCase();
      if (u in PREFIX_UNITS) prefix = PREFIX_UNITS[u];
      else if (u in SUFFIX_UNITS) suffix = SUFFIX_UNITS[u];
      if (prefix || suffix) {
        unitUsed = true;
        j++;
      }
    }
    const nWords = j - i - (unitUsed ? 1 : 0);
    const merge = unitUsed || nWords >= 2 || (fromWords && value >= 10) || !fromWords;
    if (!merge) {
      out.push({...words[i]});
      i++;
      continue;
    }
    const digits = !fromWords && !unitUsed ? first.core : commas(value);
    const trail = split(words[j - 1].text).trail;
    out.push({text: first.lead + prefix + digits + suffix + trail, start: words[i].start, end: words[j - 1].end, number: true});
    i = j;
  }
  return out;
}

// ---------------------------------------------------------------- line breaking

/** Line index per word: greedy fill, then (for 2–3 lines) the split with the narrowest widest line. */
function wrap(widths: number[], chars: number[], space: number, avail: number, maxChars: number): number[] {
  const lines: number[] = [];
  let line = 0;
  let lw = 0;
  let lc = 0;
  widths.forEach((w, i) => {
    if (i > 0 && (lw + space + w > avail || lc + 1 + chars[i] > maxChars)) {
      line++;
      lw = w;
      lc = chars[i];
    } else {
      lw += (i > 0 ? space : 0) + w;
      lc += (i > 0 ? 1 : 0) + chars[i];
    }
    lines.push(line);
  });
  const n = line + 1;
  if (n < 2 || n > 3 || widths.length < 2) return lines;
  const lineW = (a: number, b: number) => widths.slice(a, b).reduce((s, w) => s + w, 0) + space * (b - a - 1);
  const k = widths.length;
  let best = lines;
  let bestMax = Infinity;
  const consider = (cuts: number[]) => {
    const bounds = [0, ...cuts, k];
    let mx = 0;
    for (let b = 0; b + 1 < bounds.length; b++) mx = Math.max(mx, lineW(bounds[b], bounds[b + 1]));
    if (mx <= avail && mx < bestMax) {
      bestMax = mx;
      best = widths.map((_, i) => cuts.filter((c) => i >= c).length);
    }
  };
  if (n === 2) for (let a = 1; a < k; a++) consider([a]);
  else for (let a = 1; a < k - 1; a++) for (let b = a + 1; b < k; b++) consider([a, b]);
  return best;
}

const sumLines = (lineOf: number[], widths: number[], space: number) => {
  const out: number[] = [];
  lineOf.forEach((l, i) => (out[l] = (out[l] === undefined ? 0 : out[l] + space) + widths[i]));
  return out;
};

// ---------------------------------------------------------------- pages

export type PaginateOpts = {
  style: CaptionPreset;
  metrics: Metrics;
  width: number;
  height: number;
  cuts: number[]; // seconds: no page shows across one
  highlight: Highlight;
  duration?: number; // seconds: words after the end are dropped
};

const STOP = new Set(
  'the a an and or but so to of in on at for with from by is are was were be been it its this that these those you your we our they their i me my he she his her them just very really then than as if into out up down over about what when where who how all can will would could should have has had do does did not no yes get got'.split(' '),
);
const SENTENCE_END = /[.!?…]["'”’)]*$/u;
const HOLD = 0.25; // seconds a page lingers after its last word

function pickEmphasis(tokens: Token[], mode: Highlight): number {
  if (mode === 'off') return -1;
  const num = tokens.findIndex((t) => t.number);
  if (num >= 0 || mode === 'numbers') return num;
  let best = -1;
  let bestLen = 3;
  tokens.forEach((t, i) => {
    const core = split(t.text).core.toLowerCase();
    if (!STOP.has(core) && core.length > bestLen) {
      best = i;
      bestLen = core.length;
    }
  });
  return best;
}

/** Group words into caption pages and lay each page out in pixels. */
export function paginate(words: Word[], o: PaginateOpts): Page[] {
  const st = o.style;
  const H = o.height;
  const W = o.width;
  const cuts = [...o.cuts].sort((a, b) => a - b);
  const duration = o.duration ?? Infinity;
  const tokens = mergeNumbers(
    words.filter((w) => w.text.trim() && w.start < duration).map((w) => ({text: w.text.trim(), start: w.start, end: Math.max(w.end, w.start + 0.01)})),
  ).sort((a, b) => a.start - b.start);
  const side = (t: Token) => cuts.filter((c) => c <= (t.start + t.end) / 2 + 1e-9).length;
  const baseFs = (st.size / 100) * H;
  const safeW = (SAFE.x1 - SAFE.x0) * W;
  const textOf = (t: Token) => applyCase(t.text, st.case);
  const measure = (t: Token, fs: number) => textWidth(textOf(t), st.file, fs, st.letterSpacing, o.metrics);
  const spaceW = (fs: number) => textWidth(' ', st.file, fs, st.letterSpacing, o.metrics);

  const fits = (list: Token[]) => {
    if (list.length === 1) return true;
    const avail = safeW - 2 * st.padX * baseFs;
    const widths = list.map((t) => measure(t, baseFs));
    if (widths.some((w) => w > avail)) return false;
    const lines = wrap(widths, list.map((t) => [...textOf(t)].length), spaceW(baseFs), avail, st.maxCharsPerLine);
    return lines[lines.length - 1] + 1 <= Math.min(st.maxLines, 3);
  };

  const groups: Token[][] = [];
  let cur: Token[] = [];
  for (const tok of tokens) {
    if (cur.length) {
      const prev = cur[cur.length - 1];
      const brk =
        cur.length >= st.maxWords || tok.start - prev.end > st.gapBreak || side(tok) !== side(prev) ||
        SENTENCE_END.test(prev.text) || !fits([...cur, tok]);
      if (brk) {
        groups.push(cur);
        cur = [];
      }
    }
    cur.push(tok);
  }
  if (cur.length) groups.push(cur);

  // timing: from the first word to a little after the last, never across a cut or into the next page
  const times = groups.map((g) => {
    const s = side(g[0]);
    const cutBefore = s > 0 ? cuts[s - 1] : 0;
    const cutAfter = s < cuts.length ? cuts[s] : Infinity;
    return {start: Math.max(g[0].start, cutBefore), cutAfter, last: g[g.length - 1].end};
  });
  const pages: Page[] = [];
  groups.forEach((g, gi) => {
    const t = times[gi];
    const next = gi + 1 < times.length ? times[gi + 1].start : Infinity;
    const limit = Math.min(next, t.cutAfter, duration);
    let end = Math.min(t.last + HOLD, limit);
    end = Math.max(end, Math.min(t.start + st.minShow, limit));
    if (!(end > t.start)) return; // nothing left to show (a word squeezed against a cut)
    const emph = pickEmphasis(g, o.highlight);
    pages.push(layoutPage(g, emph, t.start, end, pages.length, o));
  });
  return pages;
}

const TILTS = [-3, 2.2, -1.6, 2.8, -2.4, 1.6];

function layoutPage(g: Token[], emph: number, start: number, end: number, index: number, o: PaginateOpts): Page {
  const st = o.style;
  const W = o.width;
  const H = o.height;
  const texts = g.map((t) => applyCase(t.text, st.case));
  const chars = texts.map((t) => [...t].length);
  const scale = (i: number) => (i === emph ? st.emphasisScale : 1);
  const maxLines = Math.min(st.maxLines, 3);
  let fs = (st.size / 100) * H;
  let lineOf: number[] = [];
  let widths: number[] = [];
  let space = 0;
  for (let iter = 0; iter < 60; iter++) {
    const avail = (SAFE.x1 - SAFE.x0) * W - 2 * st.padX * fs;
    widths = texts.map((t, i) => textWidth(t, st.file, fs * scale(i), st.letterSpacing, o.metrics));
    space = textWidth(' ', st.file, fs, st.letterSpacing, o.metrics);
    lineOf = wrap(widths, chars, space, avail, g.length === 1 ? Infinity : st.maxCharsPerLine);
    const widest = Math.max(...sumLines(lineOf, widths, space));
    if (lineOf[lineOf.length - 1] + 1 <= maxLines && widest <= avail + 0.01) break;
    fs *= widest > avail ? Math.max(0.5, Math.min(0.96, avail / widest)) : 0.96;
  }
  const nLines = lineOf[lineOf.length - 1] + 1;
  const lineW = sumLines(lineOf, widths, space);
  const tallest = Math.max(...texts.map((_, i) => scale(i)));
  const lineH = fs * st.lineHeight * Math.max(1, tallest * 0.92);
  const blockW = Math.max(...lineW);
  const blockH = lineH * nLines;
  const cx = ((SAFE.x0 + SAFE.x1) / 2) * W;
  const tilt = st.anim === 'ticket' ? TILTS[index % TILTS.length] : 0;
  const padX = st.padX * fs;
  const padY = st.padY * fs;
  let bw = blockW + 2 * padX;
  let bh = blockH + 2 * padY;
  if (tilt) {
    const r = (Math.abs(tilt) * Math.PI) / 180;
    [bw, bh] = [bw * Math.cos(r) + bh * Math.sin(r), bw * Math.sin(r) + bh * Math.cos(r)];
  }
  let cy = st.y * H;
  cy = Math.min(Math.max(cy, SAFE.y0 * H + bh / 2), SAFE.y1 * H - bh / 2);
  const top = cy - blockH / 2;
  const cursor: number[] = lineW.map((w) => cx - w / 2);
  const words: PageWord[] = g.map((t, i) => {
    const x = cursor[lineOf[i]];
    cursor[lineOf[i]] += widths[i] + space;
    return {text: texts[i], start: t.start, end: t.end, emphasis: i === emph, number: !!t.number, line: lineOf[i], x, w: widths[i], fs: fs * scale(i)};
  });
  return {
    index, start, end, words, lines: nLines, fontSize: fs, lineHeight: lineH, top, blockW,
    box: {x: cx - bw / 2, y: cy - bh / 2, w: bw, h: bh}, tilt,
  };
}

// ---------------------------------------------------------------- titles and end cards

export type TextFont = {file: string; size: number; lineHeight: number; letterSpacing: number; case: CaptionCase};
export type TextLayout = {fontSize: number; lineHeight: number; lines: {text: string; x: number; y: number; w: number}[]; box: Box};

/** Wrap a title into at most `maxLines` balanced lines inside the safe width, centred at height fraction `y`. */
export function layoutTitle(text: string, font: TextFont, metrics: Metrics, W: number, H: number, y: number, maxLines = 3, maxWidth = 1): TextLayout {
  const words = applyCase(text, font.case).split(/\s+/).filter(Boolean);
  const avail = (SAFE.x1 - SAFE.x0) * W * maxWidth;
  let fs = (font.size / 100) * H;
  let lineOf: number[] = [];
  let widths: number[] = [];
  let space = 0;
  for (let iter = 0; iter < 80; iter++) {
    widths = words.map((w) => textWidth(w, font.file, fs, font.letterSpacing, metrics));
    space = textWidth(' ', font.file, fs, font.letterSpacing, metrics);
    lineOf = wrap(widths, widths.map(() => 0), space, avail, Infinity);
    const widest = Math.max(0, ...sumLines(lineOf, widths, space));
    if ((lineOf[lineOf.length - 1] ?? 0) + 1 <= maxLines && widest <= avail + 0.01) break;
    fs *= widest > avail ? Math.max(0.5, Math.min(0.95, avail / widest)) : 0.95;
  }
  const lineW = sumLines(lineOf, widths, space);
  const lh = fs * font.lineHeight;
  const n = lineW.length;
  const blockH = lh * n;
  const cx = ((SAFE.x0 + SAFE.x1) / 2) * W;
  const cy = Math.min(Math.max(y * H, SAFE.y0 * H + blockH / 2), SAFE.y1 * H - blockH / 2);
  const top = cy - blockH / 2;
  const lines = lineW.map((w, li) => ({
    text: words.filter((_, i) => lineOf[i] === li).join(' '),
    x: cx - w / 2,
    y: top + li * lh,
    w,
  }));
  const blockW = Math.max(0, ...lineW);
  return {fontSize: fs, lineHeight: lh, lines, box: {x: cx - blockW / 2, y: top, w: blockW, h: blockH}};
}
