// Caption style presets, adapted from Subtle's styles/*.json (punch, highlighter, karaoke, bottom, impact, glass,
// ticket, cinema): https://github.com/0xpratzyy/subtle
// Copyright (c) 2026 Refs (refs.video) — MIT License (see licenses/subtle-LICENSE.txt).
// Changes: flattened into one typed table, sizes and positions tuned for 9:16 promos and kept inside the
// platform safe area, emphasis scales reduced so a highlighted word never forces an extra line.
//
// Plain data only (no enums, namespaces or parameter properties) so Node's type stripping can load it.

export type CaptionCase = 'upper' | 'lower' | 'sentence';

export type CaptionPreset = {
  name: string;
  family: string; // CSS family name the font is registered under
  file: string; // file in public/fonts (also the key into metrics.json)
  weight: number;
  case: CaptionCase;
  size: number; // font size, % of the frame height
  lineHeight: number; // em
  letterSpacing: number; // em
  maxWords: number;
  maxLines: number;
  maxCharsPerLine: number;
  gapBreak: number; // seconds of silence that start a new page
  minShow: number; // seconds a page stays up at least (when nothing follows)
  y: number; // centre of the block, fraction of the frame height
  padX: number; // container padding around the block, em (counts into the layout box)
  padY: number;
  color: string;
  active: string; // colour of the word being spoken (when the style marks it)
  emphasis: string; // colour of the page's highlighted word
  emphasisScale: number;
  stroke: number; // outline width, em (0 = none)
  strokeColor: string;
  shadow: string; // CSS text-shadow ('' = none); 'em' units scale with the font
  anim: 'punch' | 'highlighter' | 'karaoke' | 'bottom' | 'impact' | 'glass' | 'ticket' | 'cinema';
  box?: string; // highlighter box / ticket label / glass pill colour
  dim?: number; // opacity of upcoming words
  activeScale?: number; // scale of the word being spoken (counted into the layout box)
};

const preset = (p: CaptionPreset) => p;

export const CAPTION_STYLES: Record<string, CaptionPreset> = {
  punch: preset({
    name: 'punch', family: 'Montserrat Black', file: 'Montserrat-Black.ttf', weight: 900, case: 'upper',
    size: 5.0, lineHeight: 1.1, letterSpacing: -0.005, maxWords: 4, maxLines: 2, maxCharsPerLine: 14,
    gapBreak: 0.35, minShow: 0.4, y: 0.62, padX: 0, padY: 0.1,
    color: '#FFFFFF', active: '#C8FF2E', emphasis: '#C8FF2E', emphasisScale: 1.0,
    stroke: 0.07, strokeColor: '#000000', shadow: '0 0.08em 0.1em rgba(0,0,0,0.5)', anim: 'punch', activeScale: 1.08,
  }),
  highlighter: preset({
    name: 'highlighter', family: 'Montserrat ExtraBold', file: 'Montserrat-ExtraBold.ttf', weight: 800, case: 'upper',
    size: 3.8, lineHeight: 1.35, letterSpacing: 0, maxWords: 4, maxLines: 2, maxCharsPerLine: 20,
    gapBreak: 0.35, minShow: 0.35, y: 0.56, padX: 0.2, padY: 0.2,
    color: '#FFFFFF', active: '#FFFFFF', emphasis: '#45F715', emphasisScale: 1.25,
    stroke: 0, strokeColor: '#000000', shadow: '0 0.04em 0.18em rgba(0,0,0,0.55)', anim: 'highlighter', box: '#5A12D0',
  }),
  karaoke: preset({
    name: 'karaoke', family: 'Inter SemiBold', file: 'Inter-SemiBold.ttf', weight: 600, case: 'sentence',
    size: 3.9, lineHeight: 1.3, letterSpacing: 0, maxWords: 8, maxLines: 2, maxCharsPerLine: 26,
    gapBreak: 0.5, minShow: 0.7, y: 0.70, padX: 0, padY: 0.1,
    color: '#FFFFFF', active: '#FFFFFF', emphasis: '#FFD24A', emphasisScale: 1.07,
    stroke: 0, strokeColor: '#000000', shadow: '0 0.06em 0.25em rgba(0,0,0,0.75)', anim: 'karaoke', dim: 0.5,
  }),
  bottom: preset({
    name: 'bottom', family: 'Poppins ExtraBold', file: 'Poppins-ExtraBold.ttf', weight: 800, case: 'upper',
    size: 5.6, lineHeight: 1.1, letterSpacing: -0.03, maxWords: 3, maxLines: 1, maxCharsPerLine: 14,
    gapBreak: 0.3, minShow: 0.35, y: 0.72, padX: 0, padY: 0.1,
    color: '#FFFFFF', active: '#FFFFFF', emphasis: '#FDF018', emphasisScale: 1.0,
    stroke: 0.05, strokeColor: '#000000', shadow: '0 0.07em 0.14em rgba(0,0,0,0.7)', anim: 'bottom',
  }),
  impact: preset({
    name: 'impact', family: 'Poppins Black', file: 'Poppins-Black.ttf', weight: 900, case: 'upper',
    size: 5.8, lineHeight: 1.12, letterSpacing: -0.02, maxWords: 5, maxLines: 2, maxCharsPerLine: 12,
    gapBreak: 0.3, minShow: 0.45, y: 0.64, padX: 0, padY: 0.12,
    color: '#FFFFFF', active: '#FFFFFF', emphasis: '#FEFA1D', emphasisScale: 1.0,
    stroke: 0.06, strokeColor: '#000000', shadow: '0 0.07em 0 #000000', anim: 'impact',
  }),
  glass: preset({
    name: 'glass', family: 'Inter Medium', file: 'Inter-Medium.ttf', weight: 500, case: 'lower',
    size: 3.5, lineHeight: 1.28, letterSpacing: -0.005, maxWords: 6, maxLines: 2, maxCharsPerLine: 24,
    gapBreak: 0.4, minShow: 0.6, y: 0.70, padX: 0.9, padY: 0.55,
    color: '#F4EFF1', active: '#FFFFFF', emphasis: '#FFC46B', emphasisScale: 1.0,
    stroke: 0, strokeColor: '#000000', shadow: '0 0.03em 0.1em rgba(0,0,0,0.18)', anim: 'glass',
    box: 'rgba(60,56,55,0.55)', dim: 0.42,
  }),
  ticket: preset({
    name: 'ticket', family: 'Inter Bold', file: 'Inter-Bold.ttf', weight: 700, case: 'sentence',
    size: 3.6, lineHeight: 1.45, letterSpacing: -0.01, maxWords: 4, maxLines: 2, maxCharsPerLine: 22,
    gapBreak: 0.35, minShow: 0.35, y: 0.65, padX: 0.3, padY: 0.2,
    color: '#FFFFFF', active: '#FFFFFF', emphasis: '#F60069', emphasisScale: 1.2,
    stroke: 0, strokeColor: '#000000', shadow: '', anim: 'ticket', box: '#000000',
  }),
  cinema: preset({
    name: 'cinema', family: 'Libre Baskerville', file: 'LibreBaskerville-wght.ttf', weight: 500, case: 'upper',
    size: 2.3, lineHeight: 1.9, letterSpacing: 0.26, maxWords: 6, maxLines: 2, maxCharsPerLine: 26,
    gapBreak: 0.5, minShow: 0.9, y: 0.74, padX: 0, padY: 0,
    color: '#EFE9DD', active: '#EFE9DD', emphasis: '#FFFFFF', emphasisScale: 1.0,
    stroke: 0, strokeColor: '#000000', shadow: '0 0.05em 0.6em rgba(0,0,0,0.85)', anim: 'cinema',
  }),
};
