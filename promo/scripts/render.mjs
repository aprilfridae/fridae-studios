// Render a promo recipe (rbxpromo/recipe.py, version 1) to a 1080x1920 H.264/AAC MP4.
//
//   node render.mjs <recipe.json> <out.mp4|-> [--layout <layout.json>] [--still <sec> <out.jpg>]
//
// stdout carries only JSON lines: {"progress":0.42} while rendering, then {"ok":true,"duration":…} — or
// {"ok":false,"error":"…"} with a non-zero exit. Pass "-" as out.mp4 to render only the still.
//
// Clips are never copied. A tiny HTTP server on 127.0.0.1 serves exactly the files the recipe names (by index, so
// paths with spaces, accents or drive letters never travel in URLs), with range requests; @remotion/media's
// <Video>/<Audio> decode them in the headless browser and read only the byte ranges they need.
import {createHash} from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const out = (o) => process.stdout.write(JSON.stringify(o) + '\n');
// keep stdout for JSON: library logging goes to stderr
console.log = console.info = console.warn = (...a) => console.error(...a);

function fail(message) {
  out({ok: false, error: String(message)});
  process.exit(1);
}
process.on('uncaughtException', (e) => fail(e?.message ?? e));
process.on('unhandledRejection', (e) => fail(e?.message ?? e));

// ---------------------------------------------------------------- arguments

function parseArgs(argv) {
  const pos = [];
  const o = {layout: null, still: null};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--layout') o.layout = argv[++i] && path.resolve(argv[i]);
    else if (a === '--still') {
      o.still = {sec: Number(argv[++i]), file: argv[++i]};
      if (!Number.isFinite(o.still.sec) || !o.still.file) fail('--still needs a time in seconds and an output .jpg path.');
      o.still.file = path.resolve(o.still.file);
    } else pos.push(a);
  }
  if (pos.length !== 2) fail('usage: node render.mjs <recipe.json> <out.mp4|-> [--layout <layout.json>] [--still <sec> <out.jpg>]');
  if (pos[1] === '-' && !o.still) fail('Nothing to render: out.mp4 is "-" and no --still was given.');
  return {recipePath: path.resolve(pos[0]), video: pos[1] === '-' ? null : path.resolve(pos[1]), ...o};
}

// ---------------------------------------------------------------- helpers

function hashDir(dirs) {
  const h = createHash('sha256');
  const walk = (d) => {
    for (const name of fs.readdirSync(d).sort()) {
      const p = path.join(d, name);
      const st = fs.statSync(p);
      if (st.isDirectory()) walk(p);
      else h.update(path.relative(ROOT, p) + '\0').update(fs.readFileSync(p)).update('\0');
    }
  };
  dirs.forEach(walk);
  return h.digest('hex').slice(0, 16);
}

/** plan.ts (TypeScript) built with the esbuild that ships with Remotion's bundler, cached by source hash. */
async function loadPlan(hash) {
  const file = path.join(os.tmpdir(), `fridae-promo-plan-${hash}.mjs`);
  if (!fs.existsSync(file)) {
    const esbuild = await import('esbuild');
    await esbuild.build({entryPoints: [path.join(ROOT, 'src', 'plan.ts')], bundle: true, platform: 'node', format: 'esm', outfile: file + '.tmp', logLevel: 'silent'});
    fs.renameSync(file + '.tmp', file);
  }
  return import(pathToFileURL(file).href);
}

async function getBundle(hash, onProgress) {
  const dir = path.join(os.tmpdir(), `fridae-promo-bundle-${hash}`);
  if (fs.existsSync(path.join(dir, 'index.html'))) return dir;
  const {bundle} = await import('@remotion/bundler');
  const tmp = dir + '.' + process.pid;
  const serveUrl = await bundle({
    entryPoint: path.join(ROOT, 'src', 'index.ts'),
    publicDir: path.join(ROOT, 'public'),
    outDir: tmp,
    onProgress: (p) => onProgress(p / 100),
  });
  try {
    fs.renameSync(serveUrl, dir);
    return dir;
  } catch {
    return serveUrl; // another render finished the same bundle first
  }
}

async function probeMedia(file) {
  const {Input, ALL_FORMATS, FilePathSource} = await import('mediabunny');
  const input = new Input({source: new FilePathSource(file), formats: ALL_FORMATS});
  try {
    const v = await input.getPrimaryVideoTrack();
    const a = await input.getPrimaryAudioTrack();
    const duration = await input.computeDuration();
    return {width: v ? v.displayWidth : 0, height: v ? v.displayHeight : 0, hasVideo: !!v, hasAudio: !!a, duration};
  } catch (e) {
    throw new Error(`Could not read ${file}: ${e?.message ?? e}. Is it a video or audio file?`);
  } finally {
    input.dispose();
  }
}

const TYPES = {'.mp4': 'video/mp4', '.m4v': 'video/mp4', '.mov': 'video/quicktime', '.webm': 'video/webm', '.mkv': 'video/x-matroska',
  '.mp3': 'audio/mpeg', '.wav': 'audio/wav', '.m4a': 'audio/mp4', '.aac': 'audio/aac', '.ogg': 'audio/ogg', '.flac': 'audio/flac'};

/** Serve only `files` (by index) from 127.0.0.1 with range requests and CORS. */
function serveFiles(files) {
  const server = http.createServer((req, res) => {
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Range');
    res.setHeader('Access-Control-Expose-Headers', 'Content-Length, Content-Range, Accept-Ranges');
    res.setHeader('Accept-Ranges', 'bytes');
    if (req.method === 'OPTIONS') {
      res.end();
      return;
    }
    const m = /^\/media\/(\d+)\//.exec(req.url || '');
    const file = m ? files[Number(m[1])] : undefined;
    if (!file || (req.method !== 'GET' && req.method !== 'HEAD')) {
      res.statusCode = 404;
      res.end();
      return;
    }
    let size;
    try {
      size = fs.statSync(file).size;
    } catch {
      res.statusCode = 404;
      res.end();
      return;
    }
    let start = 0;
    let end = size - 1;
    const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range || '');
    if (range) {
      if (range[1] === '' && range[2] !== '') start = Math.max(0, size - Number(range[2]));
      else {
        start = Number(range[1] || 0);
        if (range[2] !== '') end = Math.min(Number(range[2]), size - 1);
      }
      if (start > end || start >= size) {
        res.statusCode = 416;
        res.setHeader('Content-Range', `bytes */${size}`);
        res.end();
        return;
      }
      res.statusCode = 206;
      res.setHeader('Content-Range', `bytes ${start}-${end}/${size}`);
    }
    res.setHeader('Content-Length', end - start + 1);
    res.setHeader('Content-Type', TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream');
    if (req.method === 'HEAD') {
      res.end();
      return;
    }
    const stream = fs.createReadStream(file, {start, end});
    stream.on('error', () => res.destroy());
    res.on('close', () => stream.destroy());
    stream.pipe(res);
  });
  return new Promise((resolve, reject) => {
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const {port} = server.address();
      resolve({server, url: (i) => `http://127.0.0.1:${port}/media/${i}/${encodeURIComponent(path.basename(files[i]))}`});
    });
  });
}

function loadWords(caps, resolve) {
  if (!caps || caps.source === 'none' || caps.words == null) return null;
  let data = caps.words;
  if (typeof data === 'string') {
    const p = resolve(data);
    try {
      data = JSON.parse(fs.readFileSync(p, 'utf8'));
    } catch (e) {
      throw new Error(`Could not read the captions words file ${data}: ${e.message}`);
    }
  }
  if (data && !Array.isArray(data) && Array.isArray(data.words)) data = data.words;
  if (!Array.isArray(data)) throw new Error('captions words must be a list of {text, start, end} (seconds).');
  return data
    .filter((w) => w && typeof w.text === 'string' && Number.isFinite(Number(w.start)) && Number.isFinite(Number(w.end)))
    .map((w) => ({text: w.text, start: Number(w.start), end: Number(w.end)}));
}

// ---------------------------------------------------------------- main

async function main() {
  const args = parseArgs(process.argv.slice(2)); // every path is absolute from here on
  // Remotion finds its headless browser from the working directory (nearest package.json -> node_modules/.remotion);
  // run from the project so it uses the engine's copy instead of downloading one into wherever we were started.
  process.chdir(ROOT);
  let recipe;
  try {
    recipe = JSON.parse(fs.readFileSync(args.recipePath, 'utf8'));
  } catch (e) {
    fail(`Could not read recipe ${args.recipePath}: ${e.message}`);
  }
  if (!recipe || typeof recipe !== 'object' || !Array.isArray(recipe.scenes) || !recipe.scenes.length) fail("The recipe needs a non-empty 'scenes' list.");
  if (recipe.fps !== 30 && recipe.fps !== 60) fail("'fps' must be 30 or 60.");
  const base = path.dirname(args.recipePath);
  const resolve = (p) => (path.isAbsolute(p) || path.win32.isAbsolute(p) ? path.resolve(p) : path.resolve(base, p));

  // every file the recipe names must exist; collect them for the media server
  const files = [];
  const need = (p, what) => {
    if (typeof p !== 'string' || !p) fail(`${what} must be a file path.`);
    const abs = resolve(p);
    if (!fs.existsSync(abs) || !fs.statSync(abs).isFile()) fail(`${what} file not found: ${p}`);
    if (!files.includes(abs)) files.push(abs);
  };
  recipe.scenes.forEach((s, i) => s && 'clip' in s && need(s.clip, `scene ${i}: clip`));
  for (const key of ['music', 'voice']) if (recipe.audio?.[key] != null) need(recipe.audio[key], `audio ${key}`);
  const words = loadWords(recipe.captions, resolve);

  const hash = hashDir([path.join(ROOT, 'src'), path.join(ROOT, 'public')]);
  const plan = await loadPlan(hash);
  const {server, url} = await serveFiles(files);
  try {
    const media = {};
    for (const [i, f] of files.entries()) media[f] = {...(await probeMedia(f)), url: url(i)};
    recipe.scenes.forEach((s, i) => {
      if (s && 'clip' in s) {
        const m = media[resolve(s.clip)];
        if (Number(s.out) > m.duration + 0.05)
          fail(`scene ${i}: 'out' is ${s.out} s but ${s.clip} is only ${m.duration.toFixed(2)} s long.`);
      }
    });
    let built;
    try {
      built = plan.buildPlan(recipe, media, resolve, words);
    } catch (e) {
      fail(e.message);
    }
    const {props, layout, duration} = built;
    if (args.layout) {
      fs.mkdirSync(path.dirname(args.layout), {recursive: true});
      fs.writeFileSync(args.layout, JSON.stringify(layout, null, 1));
    }

    let last = -1;
    const progress = (v) => {
      const p = Math.min(1, Math.max(0, Math.round(v * 100) / 100));
      if (p > last) {
        last = p;
        out({progress: p});
      }
    };
    progress(0);
    const serveUrl = await getBundle(hash, (p) => progress(p * 0.1));
    const {selectComposition, renderMedia, renderStill} = await import('@remotion/renderer');
    const composition = await selectComposition({serveUrl, id: 'Promo', inputProps: props, logLevel: 'error'});
    if (args.video) {
      fs.mkdirSync(path.dirname(args.video), {recursive: true});
      await renderMedia({
        serveUrl,
        composition,
        inputProps: props,
        codec: 'h264',
        audioCodec: 'aac',
        crf: 18,
        pixelFormat: 'yuv420p',
        enforceAudioTrack: true, // silent recipes still get an AAC track
        outputLocation: args.video,
        overwrite: true,
        logLevel: 'error',
        onProgress: ({progress: p}) => progress(0.1 + p * (args.still ? 0.85 : 0.9)),
      });
    }
    if (args.still) {
      const file = args.still.file;
      fs.mkdirSync(path.dirname(file), {recursive: true});
      const frame = Math.min(composition.durationInFrames - 1, Math.max(0, Math.round(args.still.sec * props.fps)));
      await renderStill({serveUrl, composition, inputProps: props, frame, output: file, imageFormat: 'jpeg', jpegQuality: 90, overwrite: true, logLevel: 'error'});
    }
    progress(1);
    out({ok: true, duration: Math.round(duration * 1000) / 1000, video: args.video, still: args.still ? args.still.file : null, layout: args.layout});
  } finally {
    server.close();
  }
}

main().then(
  () => process.exit(0),
  (e) => fail(e?.message ?? e),
);
