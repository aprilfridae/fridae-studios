import React from 'react';
import {AbsoluteFill, Easing, interpolate, spring, useCurrentFrame, useVideoConfig} from 'remotion';
import {fontCss} from './fonts';
import {endAnim, type EndCard as EndCardProps} from './plan';
import {TitleText} from './scenes/TitleScene';
import type {PromoStyle} from './styles/index';

/** Game title and a call-to-action pill over the last seconds of the footage. */
export const EndCard: React.FC<{e: EndCardProps; style: PromoStyle}> = ({e, style}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const t = style.title;
  const scrim = interpolate(frame, [0, fps * 0.25], [0, 1], {extrapolateRight: 'clamp', easing: Easing.out(Easing.cubic)});
  const pop = spring({frame: frame - Math.round(fps * 0.2), fps, config: {damping: 12, stiffness: 200, mass: 0.7}});
  const cta = e.cta;
  const box = e.ctaBox;
  return (
    <AbsoluteFill>
      <AbsoluteFill style={{backgroundColor: style.end.scrim, opacity: scrim}} />
      {e.title ? <TitleText layout={e.title} t={t} frame={frame} frames={e.frames} anim={endAnim(t.anim) as typeof t.anim} /> : null}
      {cta && box ? (
        <div
          style={{
            position: 'absolute',
            left: box.x,
            top: box.y,
            width: box.w,
            height: box.h,
            borderRadius: box.h / 2,
            backgroundColor: style.end.ctaFill,
            border: style.end.ctaOutline ? `${Math.max(2, cta.fontSize * 0.08)}px solid ${style.end.ctaOutline}` : undefined,
            boxSizing: 'border-box',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            transform: `scale(${interpolate(pop, [0, 1], [0.6, 1])})`,
            opacity: interpolate(pop, [0, 0.4], [0, 1], {extrapolateRight: 'clamp'}),
            boxShadow: '0 0.4em 1.2em rgba(0,0,0,0.35)',
            fontFamily: fontCss(t),
            fontWeight: t.weight,
            fontSize: cta.fontSize,
            letterSpacing: `${Math.min(t.letterSpacing, 0.08)}em`,
            color: style.end.ctaColor,
            whiteSpace: 'pre',
          }}
        >
          {cta.lines.map((l) => l.text).join(' ')}
        </div>
      ) : null}
    </AbsoluteFill>
  );
};
