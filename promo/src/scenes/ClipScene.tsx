import React from 'react';
import {Easing, interpolate, spring, useCurrentFrame, useVideoConfig} from 'remotion';
import {Video} from '@remotion/media';
import type {ClipScene as ClipSceneProps, Focus, Zoom} from '../plan';
import type {PromoStyle} from '../styles/index';
import {SceneShell} from './transitions';

/** Focus x (0..1 of the source width) at `t` seconds into the scene. */
export function focusAt(t: number, focus: Focus[] | null, cropX: number): number {
  if (!focus || !focus.length) return cropX;
  if (focus.length === 1) return focus[0].x;
  return interpolate(t, focus.map((k) => k.t), focus.map((k) => k.x), {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'});
}

/** Zoom factor at `frame`: slow push-in/out over the scene plus spring punches. */
export function zoomAt(frame: number, frames: number, fps: number, zoom: Zoom[]): number {
  let z = 1;
  for (const k of zoom) {
    if (k.kind === 'in') z *= interpolate(frame, [0, frames], [1, 1.12], {extrapolateRight: 'clamp'});
    else if (k.kind === 'out') z *= interpolate(frame, [0, frames], [1.12, 1], {extrapolateRight: 'clamp'});
    else {
      const at = Math.round(k.t * fps);
      const hit = spring({frame: frame - at, fps, config: {damping: 11, stiffness: 260, mass: 0.6}});
      const settle = interpolate(frame - at, [Math.round(0.3 * fps), Math.round(0.8 * fps)], [0, 1], {extrapolateLeft: 'clamp', extrapolateRight: 'clamp', easing: Easing.inOut(Easing.sin)});
      z *= 1 + 0.2 * hit * (1 - 0.6 * settle);
    }
  }
  return z;
}

export const ClipScene: React.FC<{s: ClipSceneProps; style: PromoStyle}> = ({s, style}) => {
  const frame = useCurrentFrame();
  const {width: W, height: H, fps} = useVideoConfig();
  const sw = s.srcW || W;
  const sh = s.srcH || H;
  // cover the 9:16 frame, then slide the source so the focus point sits in the middle
  const k = Math.max(W / sw, H / sh);
  const ws = sw * k;
  const hs = sh * k;
  const fx = focusAt(frame / fps, s.focus, s.cropX);
  const left = Math.min(0, Math.max(W - ws, W / 2 - fx * ws));
  const top = (H - hs) / 2;
  const z = zoomAt(frame, s.frames, fps, s.zoom);
  const g = style.grade ?? {};
  const filter = [g.saturate ? `saturate(${g.saturate})` : '', g.contrast ? `contrast(${g.contrast})` : '', g.brightness ? `brightness(${g.brightness})` : '']
    .filter(Boolean)
    .join(' ');
  const ramp = Math.max(1, Math.round(fps / 15));
  return (
    <SceneShell frames={s.frames} transIn={s.transIn} transOut={s.transOut}>
      <div
        style={{
          position: 'absolute',
          left,
          top,
          width: ws,
          height: hs,
          transform: `scale(${z})`,
          transformOrigin: `${fx * ws}px ${hs / 2}px`,
          filter: filter || undefined,
        }}
      >
        <Video
          src={s.src}
          trimBefore={s.trimBefore}
          playbackRate={s.speed}
          objectFit="fill"
          style={{width: '100%', height: '100%'}}
          volume={(f) => s.volume * Math.min(1, f / ramp, (s.frames - f) / ramp)}
        />
      </div>
      {style.letterbox ? (
        <>
          <div style={{position: 'absolute', left: 0, top: 0, width: W, height: H * 0.085, backgroundColor: '#000'}} />
          <div style={{position: 'absolute', left: 0, bottom: 0, width: W, height: H * 0.085, backgroundColor: '#000'}} />
        </>
      ) : null}
    </SceneShell>
  );
};
