'use strict';

const http = require('node:http');
const { createReadStream, createWriteStream } = require('node:fs');
const { mkdir, rm, stat } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { extname, join, posix } = require('node:path');
const { randomUUID } = require('node:crypto');
const { spawn } = require('node:child_process');
const { getAsset, isSea } = require('node:sea');

const DEFAULT_HOST = '127.0.0.1';
const DEFAULT_PORT = 4173;
const MAX_UPLOAD = 1024 * 1024 * 1024;

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.gpx': 'application/gpx+xml; charset=utf-8',
  '.png': 'image/png',
  '.webp': 'image/webp',
};

const ENCODERS = [
  { id: 'h264_nvenc', label: 'NVIDIA NVENC', vendor: 'NVIDIA', args: ['-preset', 'p5', '-rc', 'vbr', '-cq', '20', '-b:v', '0'] },
  { id: 'h264_qsv', label: 'Intel Quick Sync', vendor: 'Intel', args: ['-preset', 'medium', '-global_quality', '21'] },
  { id: 'h264_amf', label: 'AMD AMF', vendor: 'AMD', args: ['-quality', 'quality', '-rc', 'cqp', '-qp_i', '20', '-qp_p', '22'] },
  { id: 'h264_videotoolbox', label: 'Apple VideoToolbox', vendor: 'Apple', args: ['-q:v', '65'] },
];

function run(command, args, { timeout = 15_000 } = {}) {
  return new Promise((resolveRun) => {
    const child = spawn(command, args, { windowsHide: true });
    let stdout = '';
    let stderr = '';
    let resolved = false;
    const finish = (result) => {
      if (resolved) return;
      resolved = true;
      clearTimeout(timer);
      resolveRun(result);
    };
    const timer = setTimeout(() => {
      child.kill();
      finish({ ok: false, code: null, stdout, stderr: `${stderr}Timed out` });
    }, timeout);
    child.stdout?.on('data', (chunk) => { stdout += chunk; });
    child.stderr?.on('data', (chunk) => { stderr += chunk; });
    child.on('error', (error) => finish({ ok: false, code: null, stdout, stderr: `${stderr}${error.message}` }));
    child.on('close', (code) => finish({ ok: code === 0, code, stdout, stderr }));
  });
}

async function detectVideoBackends() {
  const version = await run('ffmpeg', ['-hide_banner', '-version'], { timeout: 5_000 });
  if (!version.ok) {
    return { ffmpeg: false, version: null, encoders: [], selected: null, fallback: null };
  }

  const firstLine = version.stdout.split(/\r?\n/, 1)[0] || 'FFmpeg';
  const results = [];
  for (const encoder of ENCODERS) {
    const probe = await run('ffmpeg', [
      '-hide_banner', '-loglevel', 'error',
      '-f', 'lavfi', '-i', 'color=c=black:s=128x128:r=30:d=0.1',
      '-frames:v', '1', '-an', '-c:v', encoder.id,
      '-f', 'null', '-',
    ]);
    results.push({ id: encoder.id, label: encoder.label, vendor: encoder.vendor, available: probe.ok });
  }

  return {
    ffmpeg: true,
    version: firstLine,
    encoders: results,
    selected: results.find((item) => item.available)?.id ?? null,
    fallback: 'libx264',
  };
}

let backendPromise = null;
function getBackends() {
  if (!backendPromise) backendPromise = detectVideoBackends();
  return backendPromise;
}

function json(res, status, body) {
  const payload = Buffer.from(JSON.stringify(body));
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': payload.length,
    'Cache-Control': 'no-store',
  });
  res.end(payload);
}

function assetKey(urlPath) {
  let decoded;
  try { decoded = decodeURIComponent(urlPath); } catch { return null; }
  const key = posix.normalize(decoded === '/' ? 'index.html' : decoded.replace(/^\/+/, ''));
  if (!key || key === '.' || key.startsWith('../') || key.includes('/../')) return null;
  return key;
}

function serveAsset(req, res, pathname) {
  const key = assetKey(pathname);
  if (!key) {
    json(res, 404, { error: 'Not found' });
    return;
  }
  let asset;
  try {
    asset = Buffer.from(getAsset(key));
  } catch {
    json(res, 404, { error: 'Not found' });
    return;
  }

  res.writeHead(200, {
    'Content-Type': MIME[extname(key).toLowerCase()] ?? 'application/octet-stream',
    'Content-Length': asset.length,
    'Cache-Control': 'no-store',
    'Cross-Origin-Resource-Policy': 'cross-origin',
  });
  if (req.method === 'HEAD') res.end();
  else res.end(asset);
}

function encoderArgs(id) {
  const hardware = ENCODERS.find((entry) => entry.id === id);
  if (hardware) return ['-c:v', hardware.id, ...hardware.args];
  return ['-c:v', 'libx264', '-preset', 'medium', '-crf', '18'];
}

async function receiveBody(req, target) {
  const declared = Number(req.headers['content-length'] ?? 0);
  if (declared > MAX_UPLOAD) throw new Error('影片超過 1 GB 上限');
  await new Promise((resolveWrite, rejectWrite) => {
    let received = 0;
    const output = createWriteStream(target, { flags: 'wx' });
    req.on('data', (chunk) => {
      received += chunk.length;
      if (received > MAX_UPLOAD) req.destroy(new Error('影片超過 1 GB 上限'));
    });
    req.on('error', rejectWrite);
    output.on('error', rejectWrite);
    output.on('finish', resolveWrite);
    req.pipe(output);
  });
}

async function transcode(req, res, url) {
  const backends = await getBackends();
  if (!backends.ffmpeg) {
    json(res, 503, { error: '找不到 FFmpeg；仍可直接匯出 WebM。' });
    return;
  }

  const requested = url.searchParams.get('encoder') ?? 'auto';
  const allowed = new Set([...ENCODERS.map((entry) => entry.id), 'libx264', 'auto']);
  if (!allowed.has(requested)) {
    json(res, 400, { error: '不支援的編碼器' });
    return;
  }
  const available = new Set(backends.encoders.filter((entry) => entry.available).map((entry) => entry.id));
  const selected = requested === 'auto'
    ? (backends.selected ?? 'libx264')
    : (available.has(requested) || requested === 'libx264' ? requested : 'libx264');

  const jobDir = join(tmpdir(), 'gpx-animator-gpu');
  await mkdir(jobDir, { recursive: true });
  const id = randomUUID();
  const inputPath = join(jobDir, `${id}.webm`);
  const outputPath = join(jobDir, `${id}.mp4`);
  try {
    await receiveBody(req, inputPath);
    const result = await run('ffmpeg', [
      '-hide_banner', '-loglevel', 'error', '-y', '-i', inputPath, '-an',
      ...encoderArgs(selected), '-pix_fmt', 'yuv420p', '-movflags', '+faststart', outputPath,
    ], { timeout: 60 * 60 * 1000 });
    if (!result.ok) {
      json(res, 500, { error: 'FFmpeg 轉檔失敗', detail: result.stderr.slice(-2_000), encoder: selected });
      return;
    }

    const info = await stat(outputPath);
    const downloadName = String(url.searchParams.get('name') ?? 'gpx-animation')
      .replace(/[^a-zA-Z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || 'gpx-animation';
    res.writeHead(200, {
      'Content-Type': 'video/mp4',
      'Content-Length': info.size,
      'Content-Disposition': `attachment; filename="${downloadName}.mp4"`,
      'X-GPX-Encoder': selected,
      'Cache-Control': 'no-store',
    });
    const stream = createReadStream(outputPath);
    stream.pipe(res);
    stream.on('close', () => rm(inputPath, { force: true }).then(() => rm(outputPath, { force: true })));
  } catch (error) {
    json(res, 500, { error: error.message });
    await rm(inputPath, { force: true });
    await rm(outputPath, { force: true });
  }
}

function createServer() {
  return http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url ?? '/', `http://${DEFAULT_HOST}`);
      if (req.method === 'GET' && url.pathname === '/api/system') {
        json(res, 200, await getBackends());
        return;
      }
      if (req.method === 'POST' && url.pathname === '/api/transcode') {
        await transcode(req, res, url);
        return;
      }
      if (req.method === 'GET' || req.method === 'HEAD') {
        serveAsset(req, res, url.pathname);
        return;
      }
      json(res, 405, { error: 'Method not allowed' });
    } catch (error) {
      json(res, 500, { error: error.message });
    }
  });
}

function openBrowser(url) {
  if (process.platform === 'win32') {
    spawn('cmd', ['/c', 'start', '', url], { detached: true, stdio: 'ignore', windowsHide: true }).unref();
  } else if (process.platform === 'darwin') {
    spawn('open', [url], { detached: true, stdio: 'ignore' }).unref();
  } else {
    spawn('xdg-open', [url], { detached: true, stdio: 'ignore' }).unref();
  }
}

if (process.argv.includes('--self-test')) {
  let indexBytes = 0;
  try { indexBytes = getAsset('index.html').byteLength; } catch {}
  console.log(JSON.stringify({ sea: isSea(), indexBytes, version: '1.1.0' }));
  process.exit(indexBytes > 0 && isSea() ? 0 : 1);
}

const portIndex = process.argv.indexOf('--port');
const requestedPort = portIndex >= 0 ? Number(process.argv[portIndex + 1]) : DEFAULT_PORT;
const hostIndex = process.argv.indexOf('--host');
const host = hostIndex >= 0 ? String(process.argv[hostIndex + 1]) : DEFAULT_HOST;
const shouldOpen = !process.argv.includes('--no-open');
const server = createServer();

function onListening() {
  const address = server.address();
  const port = typeof address === 'object' && address ? address.port : requestedPort;
  const displayHost = host === '0.0.0.0' || host === '::' ? '127.0.0.1' : host;
  const url = `http://${displayHost}:${port}`;
  console.log('');
  console.log('  GPX Animator GPU is running');
  console.log(`  ${url}`);
  console.log('  Close this window to stop the app.');
  console.log('');
  if (shouldOpen) openBrowser(url);
  getBackends().then((backends) => {
    console.log(backends.ffmpeg
      ? `  MP4 encoder: ${backends.selected ?? backends.fallback}`
      : '  FFmpeg not found; WebM export remains available.');
  });
}

server.once('error', (error) => {
  if (error.code === 'EADDRINUSE') {
    console.log(`  Port ${requestedPort} is in use; selecting another local port.`);
    server.listen(0, host, onListening);
  } else {
    console.error(`  Failed to start: ${error.message}`);
    process.exitCode = 1;
  }
});
server.listen(requestedPort, host, onListening);
