// Run: node --test promo/scripts/media-server.test.mjs   (or `npm test` inside promo/)
import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import {serveFiles} from './media-server.mjs';

const ALLOWED = 'http://localhost:4567';

/** Raw request (lets us set Host and Origin freely). */
function get(port, urlPath, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = http.request({host: '127.0.0.1', port, path: urlPath, headers: {host: `127.0.0.1:${port}`, ...headers}}, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve({status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks)}));
    });
    req.on('error', reject);
    req.end();
  });
}

async function withServer(fn) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'media srv é '));
  const file = path.join(dir, 'clip 1 é.mp4');
  fs.writeFileSync(file, Buffer.from('0123456789abcdef'));
  const {server, port, url} = await serveFiles([file], {allowOrigins: [ALLOWED]});
  try {
    await fn({port, path: new URL(url(0)).pathname});
  } finally {
    server.close();
    fs.rmSync(dir, {recursive: true, force: true});
  }
}

test('serves the file with the token, with ranges', () =>
  withServer(async ({port, path: p}) => {
    const r = await get(port, p, {range: 'bytes=2-5', origin: ALLOWED});
    assert.equal(r.status, 206);
    assert.equal(r.body.toString(), '2345');
    assert.equal(r.headers['access-control-allow-origin'], ALLOWED);
    assert.equal((await get(port, p)).status, 200); // Node-side fallback download: no Origin
  }));

test('without the token: 404', () =>
  withServer(async ({port, path: p}) => {
    assert.equal((await get(port, '/media/0/clip.mp4')).status, 404);
    assert.equal((await get(port, p.replace(/\/media\/[0-9a-f]+\//, '/media/' + '0'.repeat(32) + '/'))).status, 404);
  }));

test('a foreign origin is refused and gets no CORS header', () =>
  withServer(async ({port, path: p}) => {
    for (const origin of ['https://evil.example', 'http://localhost:9999', 'null']) {
      const r = await get(port, p, {origin});
      assert.equal(r.status, 403, origin);
      assert.equal(r.headers['access-control-allow-origin'], undefined, origin);
      assert.equal(r.body.length, 0);
    }
    const pre = await get(port, p, {origin: 'https://evil.example', 'access-control-request-method': 'GET'});
    assert.equal(pre.headers['access-control-allow-origin'], undefined);
  }));

test('a foreign Host (DNS rebinding) is refused', () =>
  withServer(async ({port, path: p}) => {
    assert.equal((await get(port, p, {host: `evil.example:${port}`})).status, 403);
    assert.equal((await get(port, p, {host: `localhost:${port}`})).status, 403);
  }));
