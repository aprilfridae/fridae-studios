import React from 'react';
import {AbsoluteFill, Easing, useCurrentFrame, useVideoConfig} from 'remotion';

// Each scene plays its half of a transition: the outgoing scene runs the "out" half over its last frames and the
// incoming one the "in" half over its first frames, so scenes never overlap and the video keeps its exact length.
const HALF: Record<string, number> = {cut: 0, whip: 5, zoom: 6, flash: 4, fade: 9, slide: 7};

export const halfFrames = (kind: string, fps: number) => Math.round(((HALF[kind] ?? 0) * fps) / 30);

const easeOut = Easing.out(Easing.cubic);
const easeIn = Easing.in(Easing.cubic);

type Look = {transform: string; filter: string; overlay: string | null; overlayOpacity: number};

function look(kind: string, amount: number, dir: 1 | -1): Look {
  // amount: 0 = settled, 1 = fully in the transition; dir: -1 for the incoming side, 1 for the outgoing side
  const a = amount;
  switch (kind) {
    case 'whip':
      return {transform: `translateX(${-dir * a * 55}%) scale(${1 + a * 0.08})`, filter: `blur(${a * 26}px)`, overlay: null, overlayOpacity: 0};
    case 'zoom':
      return {transform: `scale(${1 + a * 0.6})`, filter: `blur(${a * 18}px)`, overlay: null, overlayOpacity: 0};
    case 'slide':
      return {transform: `translateY(${-dir * a * 38}%)`, filter: `blur(${a * 10}px)`, overlay: '#000', overlayOpacity: a * 0.5};
    case 'flash':
      return {transform: '', filter: '', overlay: '#FFFFFF', overlayOpacity: a};
    case 'fade':
      return {transform: '', filter: '', overlay: '#000000', overlayOpacity: a};
    default:
      return {transform: '', filter: '', overlay: null, overlayOpacity: 0};
  }
}

export const SceneShell: React.FC<{frames: number; transIn: string; transOut: string; children: React.ReactNode}> = ({frames, transIn, transOut, children}) => {
  const frame = useCurrentFrame();
  const {fps} = useVideoConfig();
  const nIn = Math.min(halfFrames(transIn, fps), Math.floor(frames / 2));
  const nOut = Math.min(halfFrames(transOut, fps), Math.floor(frames / 2));
  let l: Look = look('cut', 0, 1);
  if (nIn > 0 && frame < nIn) l = look(transIn, 1 - easeOut((frame + 1) / (nIn + 1)), -1);
  else if (nOut > 0 && frame >= frames - nOut) l = look(transOut, easeIn((frame - (frames - nOut) + 1) / (nOut + 1)), 1);
  return (
    <AbsoluteFill style={{backgroundColor: '#000', overflow: 'hidden'}}>
      <AbsoluteFill style={{transform: l.transform || undefined, filter: l.filter || undefined}}>{children}</AbsoluteFill>
      {l.overlay && l.overlayOpacity > 0 ? <AbsoluteFill style={{backgroundColor: l.overlay, opacity: l.overlayOpacity}} /> : null}
    </AbsoluteFill>
  );
};
