// Promo styles: one JSON per style = edit rhythm + transition set + caption style + title/end card look.
// Adding a style is adding a JSON file here (and its import below); rbxpromo/recipe.py reads the names
// from the file names in this folder.
import cinematic from './cinematic.json';
import countdown from './countdown.json';
import hype from './hype.json';
import lyric from './lyric.json';
import meme from './meme.json';
import pov from './pov.json';
import showcase from './showcase.json';
import updateLog from './update-log.json';

export type TitleStyle = {
  family: string;
  file: string;
  weight: number;
  case: 'upper' | 'lower' | 'sentence';
  size: number; // % of frame height
  lineHeight: number;
  letterSpacing: number;
  y: number; // centre, fraction of height
  color: string;
  accent: string;
  stroke: number; // em
  strokeColor: string;
  shadow: string;
  background: string[]; // gradient stops behind title cards
  anim: 'slam' | 'reveal' | 'shake' | 'count' | 'stamp' | 'rise' | 'type';
};

export type EndStyle = {
  seconds: number;
  scrim: string;
  ctaFill: string;
  ctaColor: string;
  ctaOutline?: string;
};

export type PromoStyle = {
  name: string;
  about: string;
  rhythm: {shot: number[]; zoom: string};
  transitions: {default: string; set: string[]};
  captions: string; // caption preset name (captions/presets.ts)
  grade?: {saturate?: number; contrast?: number; brightness?: number};
  letterbox: boolean;
  title: TitleStyle;
  end: EndStyle;
};

export const PROMO_STYLES: Record<string, PromoStyle> = Object.fromEntries(
  [cinematic, countdown, hype, lyric, meme, pov, showcase, updateLog].map((s) => [s.name, s as PromoStyle]),
);

export const TRANSITIONS = ['cut', 'whip', 'flash', 'fade', 'zoom', 'slide'];
