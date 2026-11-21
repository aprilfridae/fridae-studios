import React, {useMemo} from 'react';
import {AbsoluteFill, Sequence, Series, interpolate} from 'remotion';
import {Audio} from '@remotion/media';
import {Captions} from './captions/Captions';
import {EndCard} from './EndCard';
import {useFonts, type FontRef} from './fonts';
import type {MusicTrack, PromoProps} from './plan';
import {ClipScene} from './scenes/ClipScene';
import {TitleScene} from './scenes/TitleScene';

const DUCK = 0.25; // music level under the voice
const FADE_OUT = 0.5; // seconds of music fade at the end

/** Music gain at absolute frame `f`: ducked under the voice (0.2 s ramps), faded out over the last 0.5 s. */
export function musicGain(m: MusicTrack, f: number, frames: number, fps: number): number {
  const ramp = Math.max(1, Math.round(0.2 * fps));
  let duck = 1;
  for (const [a, b] of m.duck) {
    const inside = Math.min(interpolate(f, [a - ramp, a], [0, 1], {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'}), interpolate(f, [b, b + ramp], [1, 0], {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'}));
    duck = Math.min(duck, 1 - (1 - DUCK) * inside);
  }
  const fade = interpolate(f, [frames - Math.round(FADE_OUT * fps), frames - 1], [1, 0], {extrapolateLeft: 'clamp', extrapolateRight: 'clamp'});
  return m.volume * duck * fade;
}

const Music: React.FC<{m: MusicTrack; frames: number; fps: number}> = ({m, frames, fps}) => {
  // loop by repeating the segment from music_start; each copy knows its absolute start for the volume curve
  const copies = [];
  for (let from = 0, i = 0; from < frames; from += m.segment, i++) {
    const len = Math.min(m.segment, frames - from);
    copies.push(
      <Sequence key={i} from={from} durationInFrames={len} layout="none">
        <Audio src={m.src} trimBefore={m.trimBefore} volume={(f) => musicGain(m, from + f, frames, fps)} />
      </Sequence>,
    );
  }
  return <>{copies}</>;
};

export const Promo: React.FC<PromoProps> = (p) => {
  const fonts = useMemo(() => {
    const list: FontRef[] = [p.style.title];
    for (const s of p.scenes) if (s.kind === 'title') list.push(s.style.title);
    if (p.captions) list.push(p.captions.preset);
    const seen = new Set<string>();
    return list.filter((f) => !seen.has(f.file) && !!seen.add(f.file)).map((f) => ({family: f.family, file: f.file, weight: f.weight}));
  }, [p]);
  const ready = useFonts(fonts);
  return (
    <AbsoluteFill style={{backgroundColor: '#000'}}>
      <Series>
        {p.scenes.map((s, i) => (
          <Series.Sequence key={i} durationInFrames={s.frames}>
            {s.kind === 'clip' ? <ClipScene s={s} style={p.style} /> : ready ? <TitleScene s={s} /> : null}
          </Series.Sequence>
        ))}
      </Series>
      {ready && p.captions ? <Captions preset={p.captions.preset} pages={p.captions.pages} /> : null}
      {ready && p.end ? (
        <Sequence from={p.end.from} durationInFrames={p.end.frames}>
          <EndCard e={p.end} style={p.style} />
        </Sequence>
      ) : null}
      {p.music ? <Music m={p.music} frames={p.frames} fps={p.fps} /> : null}
      {p.voice ? (
        <Sequence from={0} durationInFrames={Math.min(p.frames, p.voice.frames)} layout="none">
          <Audio src={p.voice.src} volume={p.voice.volume} />
        </Sequence>
      ) : null}
    </AbsoluteFill>
  );
};
