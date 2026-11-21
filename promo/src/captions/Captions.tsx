// Animated caption layer. The eight looks (punch, highlighter, karaoke, bottom, impact, glass, ticket, cinema) are
// adapted from Subtle's Remotion styles (template/src/styles/*.tsx, recipe.tsx):
// https://github.com/0xpratzyy/subtle
// Copyright (c) 2026 Refs (refs.video) — MIT License (see licenses/subtle-LICENSE.txt).
// Changes: one component drawing pages pre-laid-out by paginate.ts (so layout.json matches the pixels), word
// boxes positioned absolutely from bundled-font metrics, no behind-the-subject or beat-grid modes.
import React from 'react';
import {AbsoluteFill, Easing, interpolate, useCurrentFrame, useVideoConfig} from 'remotion';
import {fontCss} from '../fonts';
import type {Page, PageWord} from './paginate';
import type {CaptionPreset} from './presets';

const clamp = {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'} as const;
const f30 = (n: number, fps: number) => Math.max(1, Math.round((n * fps) / 30));
const outBack = Easing.out(Easing.back(1.7));
const outCubic = Easing.out(Easing.cubic);

type WordState = 'upcoming' | 'active' | 'spoken';
const stateOf = (w: PageWord, t: number): WordState => (t < w.start ? 'upcoming' : t >= w.end ? 'spoken' : 'active');

export const Captions: React.FC<{preset: CaptionPreset; pages: Page[]}> = ({preset: st, pages}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = frame / fps;
  const page = pages.find((p) => t >= p.start && t < p.end);
  if (!page) return null;
  const since = frame - Math.round(page.start * fps); // frames since the page appeared
  const left = Math.round(page.end * fps) - frame; // frames until it goes
  return (
    <AbsoluteFill style={{pointerEvents: 'none'}}>
      <PageView st={st} page={page} t={t} frame={frame} fps={fps} since={since} left={left} />
    </AbsoluteFill>
  );
};

const PageView: React.FC<{st: CaptionPreset; page: Page; t: number; frame: number; fps: number; since: number; left: number}> = ({st, page, t, frame, fps, since, left}) => {
  const lh = page.lineHeight;
  const lineTop = (w: PageWord) => page.top + w.line * lh;
  const base: React.CSSProperties = {
    position: 'absolute',
    whiteSpace: 'pre',
    fontFamily: fontCss(st),
    fontWeight: st.weight,
    letterSpacing: `${st.letterSpacing}em`,
    lineHeight: `${lh}px`,
    height: lh,
    textAlign: 'center',
    color: st.color,
    textShadow: st.shadow || undefined,
    WebkitTextStroke: st.stroke ? `${st.stroke * page.fontSize * 2}px ${st.strokeColor}` : undefined,
    paintOrder: 'stroke fill',
  };
  const wordBox = (w: PageWord, extra: React.CSSProperties = {}, children?: React.ReactNode) => (
    <div key={`${w.start}-${w.text}`} style={{...base, left: w.x, top: lineTop(w), width: w.w, fontSize: w.fs, ...extra}}>
      {children ?? w.text}
    </div>
  );
  // the active word, or the last spoken one in a pause
  const activeIdx = (() => {
    let idx = -1;
    page.words.forEach((w, i) => {
      if (t >= w.start) idx = i;
    });
    return idx;
  })();

  switch (st.anim) {
    case 'punch': {
      // words pop in one by one with overshoot; the spoken word swells and turns lime
      return (
        <>
          {page.words.map((w) => {
            const s = stateOf(w, t);
            if (s === 'upcoming') return null;
            const f = frame - Math.round(w.start * fps);
            const pop = interpolate(f, [0, f30(6, fps)], [0, 1], {...clamp, easing: outBack});
            const scale = interpolate(pop, [0, 1], [0.55, 1]) * (s === 'active' ? 1.08 : 1);
            const color = w.emphasis ? st.emphasis : s === 'active' ? st.active : st.color;
            return wordBox(w, {color, transform: `scale(${scale})`, opacity: interpolate(f, [0, 2], [0.2, 1], clamp)});
          })}
        </>
      );
    }
    case 'highlighter': {
      // one flat box behind the word being spoken, hopping word to word
      const cur = page.words[Math.max(0, activeIdx)];
      const prev = page.words[Math.max(0, activeIdx - 1)];
      const hop = activeIdx > 0 ? interpolate(frame - Math.round(cur.start * fps), [0, f30(3, fps)], [0, 1], {...clamp, easing: outCubic}) : 1;
      const padX = st.padX * page.fontSize * 0.6;
      const bx = interpolate(hop, [0, 1], [prev.x, cur.x]) - padX;
      const bw = interpolate(hop, [0, 1], [prev.w, cur.w]) + 2 * padX;
      const by = interpolate(hop, [0, 1], [lineTop(prev), lineTop(cur)]) + lh * 0.06;
      const appear = interpolate(since, [0, f30(3, fps)], [0, 1], clamp);
      return (
        <>
          {activeIdx >= 0 ? (
            <div style={{position: 'absolute', left: bx, top: by, width: bw, height: lh * 0.88, backgroundColor: st.box, borderRadius: page.fontSize * 0.12, transform: `rotate(-1.2deg)`}} />
          ) : null}
          {page.words.map((w) => wordBox(w, {color: w.emphasis && stateOf(w, t) !== 'upcoming' ? st.emphasis : st.color, opacity: appear}))}
        </>
      );
    }
    case 'karaoke': {
      // the whole page shows dim; each word turns solid as it is spoken
      const pageIn = interpolate(since, [0, f30(2, fps)], [0, 1], clamp) * interpolate(left, [0, f30(2, fps)], [0, 1], clamp);
      return (
        <>
          {page.words.map((w) => {
            const s = stateOf(w, t);
            const on = interpolate(frame - Math.round(w.start * fps), [0, f30(3, fps)], [0, 1], clamp);
            const color = w.emphasis && s !== 'upcoming' ? st.emphasis : st.color;
            return wordBox(w, {color, opacity: pageIn * interpolate(on, [0, 1], [st.dim ?? 0.5, 1])});
          })}
        </>
      );
    }
    case 'bottom': {
      // big caps that build word by word, no animation; the key word is yellow
      return <>{page.words.map((w) => (t < w.start ? null : wordBox(w, {color: w.emphasis ? st.emphasis : st.color})))}</>;
    }
    case 'impact': {
      // every letter bounces in along an arc with a little tilt, on a hard offset shadow
      return (
        <>
          {page.words.map((w) => {
            if (t < w.start) return null;
            const f0 = frame - Math.round(w.start * fps);
            const chars = [...w.text];
            return wordBox(
              w,
              {color: w.emphasis ? st.emphasis : st.color},
              chars.map((ch, ci) => {
                const p = interpolate(f0 - ci * f30(1, fps), [0, f30(5, fps)], [0, 1], clamp);
                const e = outBack(p);
                return (
                  <span
                    key={ci}
                    style={{
                      display: 'inline-block',
                      opacity: p > 0 ? 1 : 0,
                      transform: `translate(${(1 - e) * -10}px, ${(1 - e) * page.fontSize * 0.45}px) rotate(${(1 - e) * -10}deg) scale(${interpolate(e, [0, 1], [0.4, 1])})`,
                    }}
                  >
                    {ch === ' ' ? ' ' : ch}
                  </span>
                );
              }),
            );
          })}
        </>
      );
    }
    case 'glass': {
      // one frosted pill per page; upcoming words are dim and brighten as they are spoken
      const p = interpolate(since, [0, f30(3, fps)], [0, 1], {...clamp, easing: outCubic});
      const out = interpolate(left, [0, f30(3, fps)], [0, 1], clamp);
      const b = page.box;
      return (
        <div style={{position: 'absolute', inset: 0, opacity: Math.min(p, out), transform: `scale(${interpolate(p, [0, 1], [0.96, 1])})`, transformOrigin: `${b.x + b.w / 2}px ${b.y + b.h / 2}px`}}>
          <div
            style={{
              position: 'absolute',
              left: b.x,
              top: b.y,
              width: b.w,
              height: b.h,
              borderRadius: Math.min(b.h / 2, page.fontSize * 0.9),
              backgroundColor: st.box,
              backdropFilter: 'blur(20px)',
              boxShadow: 'inset 0 1px 0 rgba(255,255,255,0.45), 0 0.5em 1.5em rgba(0,0,0,0.25)',
            }}
          />
          {page.words.map((w) => {
            const s = stateOf(w, t);
            const on = interpolate(frame - Math.round(w.start * fps), [0, f30(3, fps)], [0, 1], clamp);
            return wordBox(w, {color: w.emphasis && s !== 'upcoming' ? st.emphasis : st.active, opacity: interpolate(on, [0, 1], [st.dim ?? 0.4, 1])});
          })}
        </div>
      );
    }
    case 'ticket': {
      // hard-edged labels tight around each line, tilted per page; the hot word gets its own coloured label
      const cx = page.box.x + page.box.w / 2;
      const cy = page.box.y + page.box.h / 2;
      const padX = st.padX * page.fontSize;
      const lines = [...new Set(page.words.map((w) => w.line))];
      return (
        <div style={{position: 'absolute', inset: 0, transform: `rotate(${page.tilt}deg)`, transformOrigin: `${cx}px ${cy}px`}}>
          {lines.map((li) => {
            const ws = page.words.filter((w) => w.line === li);
            const x0 = ws[0].x - padX;
            const x1 = ws[ws.length - 1].x + ws[ws.length - 1].w + padX;
            return <div key={li} style={{position: 'absolute', left: x0, top: page.top + li * lh + lh * 0.04, width: x1 - x0, height: lh * 0.92, backgroundColor: st.box}} />;
          })}
          {page.words.map((w) =>
            w.emphasis ? (
              <div key={`e${w.start}`} style={{position: 'absolute', left: w.x - padX * 0.5, top: lineTop(w) - lh * 0.02, width: w.w + padX, height: lh * 1.04, backgroundColor: st.emphasis}} />
            ) : null,
          )}
          {page.words.map((w) => wordBox(w, {color: st.color}))}
        </div>
      );
    }
    case 'cinema':
    default: {
      // the page fades in and out slowly with a soft blur, like arthouse trailer subtitles
      const inP = interpolate(since, [0, f30(16, fps)], [0, 1], {...clamp, easing: outCubic});
      const outP = interpolate(left, [0, f30(12, fps)], [0, 1], clamp);
      const v = Math.min(inP, outP);
      return (
        <div style={{position: 'absolute', inset: 0, opacity: v, filter: `blur(${(1 - v) * 6}px)`}}>
          {page.words.map((w) => wordBox(w, {color: w.emphasis ? st.emphasis : st.color}))}
        </div>
      );
    }
  }
};
