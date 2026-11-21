// Cuts audio into windows Whisper can take (it reads 30 s at a time), each cut at the quietest
// moment in the last SEARCH_S seconds of a window so that a word is rarely split between two passes.
export const WINDOW_S = 25;
export const SEARCH_S = 5;
const MIN_TAIL_S = 1;

export function windows(samples, rate) {
  const out = [], win = WINDOW_S * rate, frame = Math.round(rate * 0.1);
  let from = 0;
  while (from < samples.length) {
    let to = Math.min(samples.length, from + win);
    if (to < samples.length) {
      let best = Infinity, bestAt = to;
      for (let f = to - SEARCH_S * rate; f + frame <= to; f += frame) {
        let e = 0;
        for (let i = f; i < f + frame; i++) e += samples[i] * samples[i];
        if (e < best) { best = e; bestAt = f + (frame >> 1); }
      }
      to = bestAt;
      if (samples.length - to < MIN_TAIL_S * rate) to = samples.length; // no tiny last window
    }
    out.push([from, to]);
    from = to;
  }
  return out;
}
