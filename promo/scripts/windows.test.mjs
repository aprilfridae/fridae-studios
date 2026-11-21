// Run: node --test promo/scripts/windows.test.mjs
import {test} from 'node:test';
import assert from 'node:assert/strict';
import {windows} from './windows.mjs';

const RATE = 16000;
const noise = (seconds) => Float32Array.from({length: Math.round(seconds * RATE)}, (_, i) => 0.3 * Math.sin(i * 0.3));

test('the cut lands inside a silent gap in the search range', () => {
  const s = noise(40);
  s.fill(0, 22.4 * RATE, 22.6 * RATE);
  const w = windows(s, RATE);
  assert.ok(w[0][1] >= 22.4 * RATE && w[0][1] <= 22.6 * RATE, `cut at ${w[0][1] / RATE}s`);
  assert.equal(w[1][0], w[0][1]);
  assert.equal(w[w.length - 1][1], s.length);
});

test('short audio is one window', () => {
  assert.deepEqual(windows(noise(10), RATE), [[0, 10 * RATE]]);
});

test('a tiny tail is merged into the previous window', () => {
  const w = windows(noise(25.3), RATE);
  assert.equal(w.length, 1);
  assert.equal(w[0][1], Math.round(25.3 * RATE));
});

test('windows cover everything without gaps and none is shorter than a second', () => {
  const w = windows(noise(100), RATE);
  assert.equal(w[0][0], 0);
  for (let i = 1; i < w.length; i++) assert.equal(w[i][0], w[i - 1][1]);
  for (const [a, b] of w) assert.ok(b - a >= RATE);
  assert.equal(w[w.length - 1][1], 100 * RATE);
});
