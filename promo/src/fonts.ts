// Bundled fonts (public/fonts, SIL OFL) registered with the browser before any frame renders. Nothing is fetched
// from the network: staticFile() points into the bundle Remotion serves locally.
import {useEffect, useState} from 'react';
import {cancelRender, continueRender, delayRender, staticFile} from 'remotion';

export type FontRef = {family: string; file: string; weight: number};
const loaded = new Map<string, Promise<void>>();

function load(f: FontRef): Promise<void> {
  const key = `${f.family}|${f.file}`;
  let p = loaded.get(key);
  if (!p) {
    // a variable font (file name holds `wght`) registers its whole range so the CSS weight picks the instance
    const weight = /wght/.test(f.file) ? '100 900' : String(f.weight);
    const face = new FontFace(f.family, `url(${staticFile('fonts/' + f.file)})`, {weight});
    p = face.load().then((ff) => {
      document.fonts.add(ff);
    });
    loaded.set(key, p);
  }
  return p;
}

export function useFonts(fonts: FontRef[]): boolean {
  const [handle] = useState(() => delayRender('loading fonts', {timeoutInMilliseconds: 60000}));
  const [ready, setReady] = useState(false);
  useEffect(() => {
    Promise.all(fonts.map(load))
      .then(() => document.fonts.ready)
      .then(() => {
        setReady(true);
        continueRender(handle);
      })
      .catch((err) => cancelRender(err));
    // fonts are fixed for a render
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return ready;
}

export const fontCss = (f: FontRef) => `"${f.family}", "Helvetica Neue", Arial, sans-serif`;
