// The render's private media server: serves exactly the files a recipe names, to the headless browser only.
//
// - Bound to 127.0.0.1 on a random port.
// - Every URL carries a per-run random token (/media/<token>/<index>/<name>); anything else is 404, so a web page
//   open during a render cannot guess its way to the user's clips by scanning localhost ports.
// - Host must be exactly 127.0.0.1:<port> (defeats DNS rebinding).
// - CORS: a request with an Origin is answered only when that origin is in `allowOrigins` (the Remotion bundle's
//   own origin), and only then gets Access-Control-Allow-Origin; other origins get 403. Requests without an Origin
//   (Remotion's Node-side fallback download) need only the token.
// - Range requests, so the browser reads only the parts of a clip it needs; paths never appear in URLs.
import {randomBytes} from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const TYPES = {'.mp4': 'video/mp4', '.m4v': 'video/mp4', '.mov': 'video/quicktime', '.webm': 'video/webm', '.mkv': 'video/x-matroska',
  '.mp3': 'audio/mpeg', '.wav': 'audio/wav', '.m4a': 'audio/mp4', '.aac': 'audio/aac', '.ogg': 'audio/ogg', '.flac': 'audio/flac'};

export function serveFiles(files, {allowOrigins = []} = {}) {
  const token = randomBytes(16).toString('hex');
  const allowed = new Set(allowOrigins);
  let host = '';
  const server = http.createServer((req, res) => {
    const end = (code) => {
      res.statusCode = code;
      res.end();
    };
    if (req.headers.host !== host) return end(403);
    const origin = req.headers.origin;
    if (origin !== undefined) {
      if (!allowed.has(origin)) return end(403);
      res.setHeader('Access-Control-Allow-Origin', origin);
      res.setHeader('Vary', 'Origin');
      res.setHeader('Access-Control-Allow-Headers', 'Range');
      res.setHeader('Access-Control-Expose-Headers', 'Content-Length, Content-Range, Accept-Ranges');
    }
    if (req.method === 'OPTIONS') return end(204);
    const m = /^\/media\/([0-9a-f]+)\/(\d+)\//.exec(req.url || '');
    const file = m && m[1] === token ? files[Number(m[2])] : undefined;
    if (!file || (req.method !== 'GET' && req.method !== 'HEAD')) return end(404);
    let size;
    try {
      size = fs.statSync(file).size;
    } catch {
      return end(404);
    }
    res.setHeader('Accept-Ranges', 'bytes');
    let start = 0;
    let last = size - 1;
    const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || '');
    if (range) {
      if (range[1] === '' && range[2] !== '') start = Math.max(0, size - Number(range[2]));
      else {
        start = Number(range[1] || 0);
        if (range[2] !== '') last = Math.min(Number(range[2]), size - 1);
      }
      if (start > last || start >= size) {
        res.setHeader('Content-Range', `bytes */${size}`);
        return end(416);
      }
      res.statusCode = 206;
      res.setHeader('Content-Range', `bytes ${start}-${last}/${size}`);
    }
    res.setHeader('Content-Length', last - start + 1);
    res.setHeader('Content-Type', TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream');
    if (req.method === 'HEAD') return res.end();
    const stream = fs.createReadStream(file, {start, end: last});
    stream.on('error', () => res.destroy());
    res.on('close', () => stream.destroy());
    stream.pipe(res);
  });
  return new Promise((resolve, reject) => {
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const {port} = server.address();
      host = `127.0.0.1:${port}`;
      resolve({server, port, url: (i) => `http://${host}/media/${token}/${i}/${encodeURIComponent(path.basename(files[i]))}`});
    });
  });
}

/** A free TCP port on 127.0.0.1 (for Remotion's bundle server, so its origin is known in advance). */
export function freePort() {
  return new Promise((resolve, reject) => {
    const s = http.createServer();
    s.on('error', reject);
    s.listen(0, '127.0.0.1', () => {
      const {port} = s.address();
      s.close(() => resolve(port));
    });
  });
}
