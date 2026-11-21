// Run (Node 22, the engine's node): node --experimental-strip-types --test 'promo/src/captions/*.test.ts'
// (a folder argument does not work on Node 22), or `npm test` inside promo/.
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {mergeNumbers, paginate, layoutTitle, titleBox, SAFE, TITLE_DRAW, type Box} from './paginate.ts';
import {CAPTION_STYLES} from './presets.ts';

const metrics = JSON.parse(readFileSync(new URL('./metrics.json', import.meta.url), 'utf8'));
const W = 1080;
const H = 1920;

const timed = (texts: string[], step = 0.3, from = 0) =>
  texts.map((text, i) => ({text, start: from + i * step, end: from + i * step + step * 0.9}));

test('spoken numbers merge into one token', () => {
  const out = mergeNumbers(timed(['five', 'thousand', 'dollars']));
  assert.equal(out.length, 1);
  assert.equal(out[0].text, '$5,000');
  assert.equal(out[0].start, 0);
  assert.ok(Math.abs(out[0].end - 0.87) < 1e-9);
  assert.equal(out[0].number, true);
});

test('number merging keeps the words around it and handles other forms', () => {
  const t = (xs: string[]) => mergeNumbers(timed(xs)).map((w) => w.text);
  assert.deepEqual(t(['win', 'twenty-five', 'percent', 'more']), ['win', '25%', 'more']);
  assert.deepEqual(t(['one', 'hundred', 'and', 'five', 'pets']), ['105', 'pets']);
  assert.deepEqual(t(['get', '5000', 'robux.']), ['get', '5,000 Robux.']);
  assert.deepEqual(t(['two', 'million', 'players!']), ['2,000,000', 'players!']);
  assert.deepEqual(t(['one', 'more', 'time']), ['one', 'more', 'time']); // small lone numbers stay words
  assert.deepEqual(t(['and', 'then']), ['and', 'then']);
});

test('no page crosses a cut at 2.0 s', () => {
  const words = timed('this pet is insane you have to see what happens next when it evolves'.split(' '), 0.25, 0.5);
  for (const style of Object.values(CAPTION_STYLES)) {
    const pages = paginate(words, {style, metrics, width: W, height: H, cuts: [2.0], highlight: 'auto'});
    assert.ok(pages.length > 1);
    for (const p of pages) {
      assert.ok(!(p.start < 2.0 - 1e-6 && p.end > 2.0 + 1e-6), `${style.name}: page ${p.start}-${p.end} crosses 2.0`);
      assert.ok(p.end > p.start);
    }
    // every word is shown exactly once, in order
    assert.equal(pages.flatMap((p) => p.words).length, words.length);
  }
});

test('pages do not overlap in time', () => {
  const words = timed('grow your pet trade with friends and become the richest player on the server'.split(' '), 0.2);
  for (const style of Object.values(CAPTION_STYLES)) {
    const pages = paginate(words, {style, metrics, width: W, height: H, cuts: [], highlight: 'auto'});
    for (let i = 1; i < pages.length; i++) assert.ok(pages[i].start >= pages[i - 1].end - 1e-9, style.name);
  }
});

test('one highlight per page, numbers win', () => {
  const words = timed('win five thousand dollars in the brand new mega update tonight with every friend'.split(' '));
  for (const style of Object.values(CAPTION_STYLES)) {
    const pages = paginate(words, {style, metrics, width: W, height: H, cuts: [], highlight: 'auto'});
    for (const p of pages) assert.ok(p.words.filter((w) => w.emphasis).length <= 1, style.name);
    const withNumber = pages.find((p) => p.words.some((w) => w.text === '$5,000'))!;
    assert.ok(withNumber.words.find((w) => w.text === '$5,000')!.emphasis, style.name);
    const off = paginate(words, {style, metrics, width: W, height: H, cuts: [], highlight: 'off'});
    assert.equal(off.flatMap((p) => p.words).filter((w) => w.emphasis).length, 0);
  }
});

test('a page never exceeds 3 lines and stays inside the safe width at 1080 px', () => {
  const long = 'supercalifragilistic extraordinarily unbelievable transformation championship legendary mythical dragon evolution'.split(' ');
  const words = timed([...long, ...'the quick brown fox jumps over the lazy dog again and again'.split(' ')], 0.3);
  for (const style of Object.values(CAPTION_STYLES)) {
    const pages = paginate(words, {style, metrics, width: W, height: H, cuts: [], highlight: 'auto'});
    for (const p of pages) {
      assert.ok(p.lines <= 3 && p.lines <= style.maxLines, `${style.name}: ${p.lines} lines`);
      assert.ok(p.box.x >= SAFE.x0 * W - 1 && p.box.x + p.box.w <= SAFE.x1 * W + 1, `${style.name}: box x ${p.box.x} w ${p.box.w}`);
      assert.ok(p.box.y >= SAFE.y0 * H - 1 && p.box.y + p.box.h <= SAFE.y1 * H + 1, `${style.name}: box y ${p.box.y} h ${p.box.h}`);
      assert.ok(p.fontSize <= (style.size / 100) * H + 1e-6);
    }
  }
});

test('titles wrap to at most 3 lines inside the safe area', () => {
  const font = {file: 'Anton-Regular.ttf', size: 12, lineHeight: 1.0, letterSpacing: 0, case: 'upper' as const};
  const t = layoutTitle('THIS PET IS ABSOLUTELY INSANE AND YOU WILL NOT BELIEVE IT', font, metrics, W, H, 0.45);
  assert.ok(t.lines.length <= 3);
  assert.ok(t.box.x >= SAFE.x0 * W - 1 && t.box.x + t.box.w <= SAFE.x1 * W + 1);
});

// ---------------------------------------------------------------- reported boxes hold what is drawn

const rotateAbout = (x: number, y: number, cx: number, cy: number, deg: number) => {
  const r = (deg * Math.PI) / 180;
  return [cx + (x - cx) * Math.cos(r) - (y - cy) * Math.sin(r), cy + (x - cx) * Math.sin(r) + (y - cy) * Math.cos(r)];
};
const assertHolds = (box: Box, rect: Box, cx: number, cy: number, deg: number, what: string) => {
  for (const [x, y] of [[rect.x, rect.y], [rect.x + rect.w, rect.y], [rect.x, rect.y + rect.h], [rect.x + rect.w, rect.y + rect.h]]) {
    const [px, py] = rotateAbout(x, y, cx, cy, deg);
    assert.ok(px >= box.x - 0.01 && px <= box.x + box.w + 0.01 && py >= box.y - 0.01 && py <= box.y + box.h + 0.01, `${what}: corner ${px},${py} outside ${JSON.stringify(box)}`);
  }
};

test('caption boxes hold the outline, the active-word scale-up, containers and tilt', () => {
  const words = timed('grow the biggest pet supercalifragilistic and trade five thousand dollars with friends today'.split(' '), 0.3);
  for (const st of Object.values(CAPTION_STYLES)) {
    const pages = paginate(words, {style: st, metrics, width: W, height: H, cuts: [], highlight: 'auto'});
    for (const p of pages) {
      const cx = p.box.x + p.box.w / 2;
      const cy = p.box.y + p.box.h / 2;
      const e = st.stroke * p.fontSize;
      const g = (st.activeScale ?? 1) - 1;
      for (const w of p.words) {
        const top = p.top + w.line * p.lineHeight;
        // the word drawn with its outline, scaled up about its centre while spoken
        const drawn = {x: w.x - e - (g * w.w) / 2, y: top - e - (g * p.lineHeight) / 2, w: w.w * (1 + g) + 2 * e, h: p.lineHeight * (1 + g) + 2 * e};
        assertHolds(p.box, drawn, cx, cy, p.tilt, `${st.name} word ${w.text}`);
      }
      // container (glass pill / ticket labels / highlighter box) padding around the text block
      const block = {x: cx - p.blockW / 2 - st.padX * p.fontSize, y: p.top - st.padY * p.fontSize, w: p.blockW + 2 * st.padX * p.fontSize, h: p.lines * p.lineHeight + 2 * st.padY * p.fontSize};
      assertHolds(p.box, block, cx, cy, p.tilt, `${st.name} container`);
      assert.ok(p.box.x >= SAFE.x0 * W - 1 && p.box.x + p.box.w <= SAFE.x1 * W + 1, `${st.name}: box leaves the safe width`);
    }
  }
  const punch = paginate(timed(['supercalifragilistic']), {style: CAPTION_STYLES.punch, metrics, width: W, height: H, cuts: [], highlight: 'off'})[0];
  assert.ok(punch.box.w >= punch.blockW * 1.08 + 2 * 0.07 * punch.fontSize - 0.01, 'punch: scale 1.08 and outline counted');
});

test('title boxes hold stamp padding and rotation, shake rotation, outline and caret', () => {
  for (const anim of ['stamp', 'shake', 'type', 'slam']) {
    const font = {file: 'Montserrat-Black.ttf', size: 7.5, lineHeight: 1.02, letterSpacing: -0.01, case: 'upper' as const, stroke: 0.05, anim};
    const l = layoutTitle('UPDATE 12 IS HERE WITH NEW PETS', font, metrics, W, H, 0.45);
    const box = titleBox(l, font.stroke, anim);
    const d = TITLE_DRAW[anim] ?? {padX: 0, rotate: 0};
    const fs = l.fontSize;
    for (const line of l.lines) {
      const e = fs * (font.stroke + 0.06);
      const drawn = {x: line.x - d.padX * fs - e, y: line.y - e, w: line.w + 2 * (d.padX * fs + e), h: l.lineHeight + 2 * e};
      const cx = line.x + line.w / 2;
      const cy = line.y + l.lineHeight / 2;
      for (const deg of [d.rotate, -d.rotate]) assertHolds(box, drawn, cx, cy, deg, `${anim} line ${line.text}`);
    }
    if (anim === 'stamp') assert.ok(box.w >= Math.max(...l.lines.map((x) => x.w)) + 2 * 0.25 * fs);
    assert.ok(box.x >= SAFE.x0 * W - 1 && box.x + box.w <= SAFE.x1 * W + 1, `${anim}: ${JSON.stringify(box)}`);
    assert.ok(box.y >= SAFE.y0 * H - 1 && box.y + box.h <= SAFE.y1 * H + 1, `${anim}: ${JSON.stringify(box)}`);
  }
});
