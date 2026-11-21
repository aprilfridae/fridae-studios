import React from 'react';
import {AbsoluteFill, Easing, interpolate, spring, useCurrentFrame, useVideoConfig} from 'remotion';
import {TITLE_DRAW, type TextLayout} from '../captions/paginate';
import {fontCss} from '../fonts';
import type {TitleScene as TitleSceneProps} from '../plan';
import type {TitleStyle} from '../styles/index';
import {SceneShell} from './transitions';

const clamp = {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'} as const;

/** The lines of a laid-out title, animated per the style's `anim`. Also used by the end card. */
export const TitleText: React.FC<{layout: TextLayout; t: TitleStyle; frame: number; frames: number; color?: string; anim?: TitleStyle['anim']}> = ({layout, t, frame, frames, color, anim}) => {
  const {fps} = useVideoConfig();
  const a = anim ?? t.anim;
  const fs = layout.fontSize;
  const total = layout.lines.reduce((n, l) => n + l.text.length, 0);
  let typed = 0;
  return (
    <>
      {layout.lines.map((line, li) => {
        let transform = '';
        let opacity = 1;
        let filter = '';
        let ls = t.letterSpacing;
        let text = line.text;
        let caret = false;
        const f = frame - li * Math.round(fps * 0.06);
        if (a === 'slam') {
          const s = spring({frame: f, fps, config: {damping: 13, stiffness: 220, mass: 0.7}});
          const shake = f > 4 && f < 12 ? Math.sin(f * 2.7) * (12 - f) * 0.9 : 0;
          transform = `translate(${shake}px, ${-shake * 0.6}px) scale(${interpolate(s, [0, 1], [2.1, 1])})`;
          opacity = interpolate(f, [0, 3], [0, 1], clamp);
        } else if (a === 'reveal') {
          const p = interpolate(frame, [0, fps * 0.9], [0, 1], {...clamp, easing: Easing.out(Easing.cubic)});
          opacity = p;
          ls = t.letterSpacing + (1 - p) * 0.12;
          filter = `blur(${(1 - p) * 10}px)`;
        } else if (a === 'shake') {
          const s = spring({frame: f, fps, config: {damping: 9, stiffness: 240, mass: 0.6}});
          transform = `scale(${interpolate(s, [0, 1], [0.55, 1])}) rotate(${Math.sin(frame * 1.9 + li) * TITLE_DRAW.shake.rotate}deg)`;
        } else if (a === 'count') {
          const p = interpolate(frame, [0, fps * 0.35], [0, 1], {...clamp, easing: Easing.out(Easing.exp)});
          transform = `scale(${interpolate(p, [0, 1], [2.6, 1])})`;
          opacity = p;
          filter = `blur(${(1 - p) * 14}px)`;
        } else if (a === 'stamp') {
          const s = spring({frame: f, fps, config: {damping: 15, stiffness: 300, mass: 0.6}});
          transform = `rotate(${TITLE_DRAW.stamp.rotate}deg) scale(${interpolate(s, [0, 1], [1.45, 1])})`;
          opacity = interpolate(f, [0, 2], [0, 1], clamp);
        } else if (a === 'rise') {
          const p = interpolate(f, [0, fps * 0.45], [0, 1], {...clamp, easing: Easing.out(Easing.cubic)});
          transform = `translateY(${(1 - p) * fs * 0.45}px)`;
          opacity = p;
        } else if (a === 'type') {
          const shown = Math.floor(interpolate(frame, [0, Math.max(1, frames * 0.6)], [0, total + 0.999], clamp));
          text = line.text.slice(0, Math.max(0, shown - typed));
          caret = shown >= typed && (shown < typed + line.text.length || li === layout.lines.length - 1);
          typed += line.text.length;
        }
        const stamp = a === 'stamp';
        return (
          <div key={li} style={{position: 'absolute', left: line.x + line.w / 2, top: line.y, height: layout.lineHeight}}>
            <div
              style={{
                transform: `translateX(-50%) ${transform}`,
                transformOrigin: 'center center',
                opacity,
                filter: filter || undefined,
                whiteSpace: 'pre',
                fontFamily: fontCss(t),
                fontWeight: t.weight,
                fontSize: fs,
                lineHeight: `${layout.lineHeight}px`,
                letterSpacing: `${ls}em`,
                color: color ?? t.color,
                textShadow: t.shadow || undefined,
                WebkitTextStroke: t.stroke ? `${t.stroke * fs * 2}px ${t.strokeColor}` : undefined,
                paintOrder: 'stroke fill',
                backgroundColor: stamp ? t.accent : undefined,
                padding: stamp ? `0 ${fs * TITLE_DRAW.stamp.padX}px` : undefined,
              }}
            >
              {a === 'type' ? <span>{text || '​'}</span> : text}
              {caret && Math.floor(frame / (fps * 0.25)) % 2 === 0 ? <span style={{color: t.accent, marginLeft: fs * 0.04}}>|</span> : null}
            </div>
          </div>
        );
      })}
    </>
  );
};

export const TitleScene: React.FC<{s: TitleSceneProps}> = ({s}) => {
  const frame = useCurrentFrame();
  const t = s.style.title;
  const bg = t.background.length > 1 ? `linear-gradient(160deg, ${t.background.join(', ')})` : t.background[0];
  const drift = interpolate(frame, [0, s.frames], [1.0, 1.06]);
  return (
    <SceneShell frames={s.frames} transIn={s.transIn} transOut={s.transOut}>
      <AbsoluteFill style={{background: bg, transform: `scale(${drift})`}} />
      <AbsoluteFill style={{background: 'radial-gradient(ellipse at 50% 45%, rgba(255,255,255,0.10), rgba(0,0,0,0.35) 75%)'}} />
      <TitleText layout={s.layout} t={t} frame={frame} frames={s.frames} />
    </SceneShell>
  );
};
