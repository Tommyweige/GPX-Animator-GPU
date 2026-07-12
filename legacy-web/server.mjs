import http from 'node:http';
import { createReadStream, createWriteStream, existsSync } from 'node:fs';
import { mkdir, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { extname, join, normalize, resolve, sep } from 'node:path';
import { createHash, randomUUID } from 'node:crypto';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = resolve(fileURLToPath(new URL('.', import.meta.url)));
const PUBLIC = join(ROOT, 'public');
const DEFAULT_HOST = 'localhost';
const DEFAULT_PORT = 4173;
const MAX_UPLOAD = 1024 * 1024 * 1024;
const renderResults = new Map();

async function removeTemporaryFile(path) {
  if (!path) return;
  for (let attempt = 0; attempt < 12; attempt += 1) {
    try {
      await rm(path, { force: true });
      if (!existsSync(path)) return;
    } catch {
      // Windows may briefly keep an FFmpeg output handle after process termination.
    }
    await new Promise((resolveWait) => setTimeout(resolveWait, 100));
  }
}

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
  { id: 'hevc_nvenc', codec: 'h265', label: 'RTX 2080 Ti · NVENC H.265', vendor: 'NVIDIA', args: ['-preset', 'p5', '-tune', 'hq', '-rc', 'vbr', '-cq', '24', '-b:v', '0', '-gpu', '0'] },
  { id: 'h264_nvenc', codec: 'h264', label: 'RTX 2080 Ti · NVENC H.264', vendor: 'NVIDIA', args: ['-preset', 'p5', '-tune', 'hq', '-rc', 'vbr', '-cq', '20', '-b:v', '0', '-gpu', '0'] },
];

function run(command, args, { timeout = 15_000, signal } = {}) {
  return new Promise((resolveRun) => {
    const child = spawn(command, args, { windowsHide: true });
    let stdout = '';
    let stderr = '';
    const timer = setTimeout(() => child.kill(), timeout);
    const abort = () => child.kill();
    signal?.addEventListener('abort', abort, { once: true });
    child.stdout?.on('data', (chunk) => { stdout += chunk; });
    child.stderr?.on('data', (chunk) => { stderr += chunk; });
    child.on('error', (error) => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      resolveRun({ ok: false, code: null, stdout, stderr: `${stderr}${error.message}` });
    });
    child.on('close', (code) => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      resolveRun({ ok: code === 0, code, stdout, stderr });
    });
  });
}

export async function detectVideoBackends() {
  const version = await run('ffmpeg', ['-hide_banner', '-version'], { timeout: 5_000 });
  if (!version.ok) {
    return {
      ffmpeg: false,
      version: null,
      encoders: [],
      selected: null,
      fallback: null,
    };
  }

  const firstLine = version.stdout.split(/\r?\n/, 1)[0] || 'FFmpeg';
  const results = [];
  for (const encoder of ENCODERS) {
    const probe = await run('ffmpeg', [
      '-hide_banner', '-loglevel', 'error',
      '-f', 'lavfi', '-i', 'color=c=black:s=1920x1080:r=60:d=0.1',
      '-frames:v', '2', '-an', ...encoderArgs(encoder.id),
      '-pix_fmt', 'yuv420p',
      '-f', 'null', '-',
    ]);
    results.push({
      id: encoder.id,
      label: encoder.label,
      vendor: encoder.vendor,
      codec: encoder.codec,
      available: probe.ok,
    });
  }

  return {
    ffmpeg: true,
    version: firstLine,
    encoders: results,
    selected: results.find((item) => item.available)?.id ?? null,
    fallback: null,
    preferredCodec: 'h265',
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

function safePublicPath(urlPath) {
  const decoded = decodeURIComponent(urlPath === '/' ? '/index.html' : urlPath);
  const relative = normalize(decoded).replace(/^([/\\])+/, '');
  const candidate = resolve(PUBLIC, relative);
  if (candidate !== PUBLIC && !candidate.startsWith(`${PUBLIC}${sep}`)) return null;
  return candidate;
}

async function serveStatic(req, res, pathname) {
  const filePath = safePublicPath(pathname);
  if (!filePath || !existsSync(filePath)) {
    json(res, 404, { error: 'Not found' });
    return;
  }
  const info = await stat(filePath);
  if (!info.isFile()) {
    json(res, 404, { error: 'Not found' });
    return;
  }

  const type = MIME[extname(filePath).toLowerCase()] ?? 'application/octet-stream';
  res.writeHead(200, {
    'Content-Type': type,
    'Content-Length': info.size,
    'Cache-Control': pathname.startsWith('/js/') ? 'no-cache' : 'no-store',
    'Cross-Origin-Resource-Policy': 'cross-origin',
  });
  createReadStream(filePath).pipe(res);
}

function encoderArgs(id) {
  const hardware = ENCODERS.find((entry) => entry.id === id);
  if (hardware) return ['-c:v', hardware.id, ...hardware.args];
  if (id === 'libx265') return ['-c:v', 'libx265', '-preset', 'medium', '-crf', '24'];
  return ['-c:v', 'libx264', '-preset', 'medium', '-crf', '18'];
}

function encoderCodec(id) {
  const hardware = ENCODERS.find((entry) => entry.id === id);
  if (hardware) return hardware.codec;
  return id === 'libx265' ? 'h265' : 'h264';
}

function normaliseFps(value) {
  const fps = Math.round(Number(value));
  return Number.isFinite(fps) && fps >= 1 && fps <= 120 ? fps : 30;
}

function encoderCandidates({ requested, codec, available }) {
  if (requested !== 'auto') return available.has(requested) ? [requested] : [];
  const preferredCodec = codec === 'h264' ? 'h264' : 'h265';
  return ENCODERS
    .filter((entry) => entry.codec === preferredCodec && available.has(entry.id))
    .map((entry) => entry.id);
}

async function receiveBody(req, target) {
  const declared = Number(req.headers['content-length'] ?? 0);
  if (declared > MAX_UPLOAD) throw new Error('影片超過 1 GB 上限');

  await new Promise((resolveWrite, rejectWrite) => {
    let received = 0;
    const output = createWriteStream(target, { flags: 'wx' });
    req.on('data', (chunk) => {
      received += chunk.length;
      if (received > MAX_UPLOAD) {
        req.destroy(new Error('影片超過 1 GB 上限'));
        return;
      }
    });
    req.on('error', rejectWrite);
    output.on('error', rejectWrite);
    output.on('finish', resolveWrite);
    req.pipe(output);
  });
}

async function transcode(req, res, url) {
  const controller = new AbortController();
  const abortJob = () => controller.abort();
  req.once('aborted', abortJob);
  res.once('close', () => { if (!res.writableEnded) abortJob(); });
  const backends = await getBackends();
  if (!backends.ffmpeg) {
    json(res, 503, { error: '找不到 FFmpeg；仍可直接匯出 WebM。' });
    return;
  }

  const requested = url.searchParams.get('encoder') ?? 'auto';
  const codec = url.searchParams.get('codec') ?? 'h265';
  const fps = normaliseFps(url.searchParams.get('fps'));
  const allowed = new Set([...ENCODERS.map((entry) => entry.id), 'libx264', 'libx265', 'auto']);
  if (!allowed.has(requested)) {
    json(res, 400, { error: '不支援的編碼器' });
    return;
  }

  const available = new Set(backends.encoders.filter((entry) => entry.available).map((entry) => entry.id));
  const candidates = [...new Set(encoderCandidates({ requested, codec, available }))];
  if (!candidates.length) {
    json(res, 503, { error: `NVIDIA NVENC ${codec === 'h264' ? 'H.264' : 'H.265'} 無法使用；已禁止 Intel QSV 與 CPU 回退。` });
    return;
  }

  const jobDir = join(tmpdir(), 'gpx-animator-gpu');
  await mkdir(jobDir, { recursive: true });
  const id = randomUUID();
  const inputPath = join(jobDir, `${id}.webm`);
  const outputPath = join(jobDir, `${id}.mp4`);

  try {
    await receiveBody(req, inputPath);
    let selected = null;
    const failures = [];
    for (const candidate of candidates) {
      await rm(outputPath, { force: true });
      const args = [
        '-hide_banner', '-loglevel', 'error', '-y',
        '-i', inputPath, '-an',
        '-vf', `fps=${fps}:round=near`, '-r', String(fps), '-fps_mode', 'cfr',
        ...encoderArgs(candidate),
        '-pix_fmt', 'yuv420p',
        ...(encoderCodec(candidate) === 'h265' ? ['-tag:v', 'hvc1'] : []),
        '-movflags', '+faststart', outputPath,
      ];
      const result = await run('ffmpeg', args, { timeout: 60 * 60 * 1000, signal: controller.signal });
      if (controller.signal.aborted) throw new DOMException('匯出已取消', 'AbortError');
      if (result.ok) {
        selected = candidate;
        break;
      }
      failures.push(`${candidate}: ${result.stderr.slice(-800)}`);
    }
    if (!selected) {
      json(res, 500, { error: 'FFmpeg 轉檔失敗', detail: failures.join('\n\n').slice(-4_000) });
      return;
    }

    const info = await stat(outputPath);
    const downloadName = String(url.searchParams.get('name') ?? 'gpx-animation')
      .replace(/[^a-zA-Z0-9._-]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'gpx-animation';
    res.writeHead(200, {
      'Content-Type': 'video/mp4',
      'Content-Length': info.size,
      'Content-Disposition': `attachment; filename="${downloadName}.mp4"`,
      'X-GPX-Encoder': selected,
      'X-GPX-Codec': encoderCodec(selected),
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

function boundedInteger(value, minimum, maximum, fallback) {
  const number = Math.round(Number(value));
  return Number.isFinite(number) && number >= minimum && number <= maximum ? number : fallback;
}

async function renderMp4(req, res, url) {
  const backends = await getBackends();
  const codec = url.searchParams.get('codec') === 'h264' ? 'h264' : 'h265';
  const encoder = codec === 'h264' ? 'h264_nvenc' : 'hevc_nvenc';
  const available = new Set(backends.encoders.filter((entry) => entry.available).map((entry) => entry.id));
  if (!backends.ffmpeg || !available.has(encoder)) {
    json(res, 503, { error: `RTX 2080 Ti NVENC ${codec === 'h264' ? 'H.264' : 'H.265'} 無法使用。` });
    return;
  }

  const fps = normaliseFps(url.searchParams.get('fps'));
  const width = boundedInteger(url.searchParams.get('width'), 320, 7680, 1920);
  const height = boundedInteger(url.searchParams.get('height'), 240, 7680, 1080);
  const frames = boundedInteger(url.searchParams.get('frames'), 2, 216_000, fps * 20);
  const jobDir = join(tmpdir(), 'gpx-animator-gpu');
  await mkdir(jobDir, { recursive: true });
  const outputPath = join(jobDir, `${randomUUID()}.mp4`);
  const controller = new AbortController();
  const abortJob = () => controller.abort();
  req.once('aborted', abortJob);
  res.once('close', () => { if (!res.writableEnded) abortJob(); });

  const args = [
    '-hide_banner', '-loglevel', 'error', '-y',
    '-f', 'image2pipe', '-framerate', String(fps), '-vcodec', 'mjpeg',
    '-i', 'pipe:0', '-frames:v', String(frames), '-an',
    ...encoderArgs(encoder),
    '-pix_fmt', 'yuv420p',
    ...(codec === 'h265' ? ['-tag:v', 'hvc1'] : []),
    '-movflags', '+faststart', outputPath,
  ];
  const child = spawn('ffmpeg', args, { windowsHide: true });
  let stderr = '';
  const abortChild = () => child.kill();
  controller.signal.addEventListener('abort', abortChild, { once: true });
  child.stderr.on('data', (chunk) => { stderr = `${stderr}${chunk}`.slice(-8_000); });
  req.pipe(child.stdin);

  try {
    const code = await new Promise((resolveCode, rejectCode) => {
      child.on('error', rejectCode);
      child.on('close', resolveCode);
    });
    if (controller.signal.aborted) return;
    if (code !== 0) {
      json(res, 500, { error: 'RTX NVENC 編碼失敗', detail: stderr.slice(-4_000) });
      return;
    }
    const info = await stat(outputPath);
    const downloadName = String(url.searchParams.get('name') ?? 'gpx-animation')
      .replace(/[^a-zA-Z0-9._-]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'gpx-animation';
    res.writeHead(200, {
      'Content-Type': 'video/mp4',
      'Content-Length': info.size,
      'Content-Disposition': `attachment; filename="${downloadName}.mp4"`,
      'X-GPX-Encoder': encoder,
      'X-GPX-Codec': codec,
      'X-GPX-GPU': 'NVIDIA GeForce RTX 2080 Ti',
      'X-GPX-Frame-Transport': 'mjpeg-stream',
      'Cache-Control': 'no-store',
    });
    const output = createReadStream(outputPath);
    output.pipe(res);
    output.on('close', () => rm(outputPath, { force: true }));
  } catch (error) {
    if (!controller.signal.aborted && !res.headersSent) json(res, 500, { error: error.message });
  } finally {
    controller.signal.removeEventListener('abort', abortChild);
    if (controller.signal.aborted) await rm(outputPath, { force: true });
  }
}

function websocketFrame(payload) {
  const data = Buffer.from(payload);
  let header;
  if (data.length < 126) {
    header = Buffer.from([0x81, data.length]);
  } else if (data.length <= 0xffff) {
    header = Buffer.alloc(4);
    header[0] = 0x81; header[1] = 126; header.writeUInt16BE(data.length, 2);
  } else {
    header = Buffer.alloc(10);
    header[0] = 0x81; header[1] = 127; header.writeBigUInt64BE(BigInt(data.length), 2);
  }
  return Buffer.concat([header, data]);
}

function sendSocketJson(socket, value) {
  if (!socket.destroyed) socket.write(websocketFrame(JSON.stringify(value)));
}

function handleRenderSocket(req, socket) {
  const url = new URL(req.url ?? '/', `http://${DEFAULT_HOST}`);
  const key = req.headers['sec-websocket-key'];
  if (url.pathname !== '/api/render-stream' || !key) { socket.destroy(); return; }
  const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
  socket.write(['HTTP/1.1 101 Switching Protocols', 'Upgrade: websocket', 'Connection: Upgrade', `Sec-WebSocket-Accept: ${accept}`, '\r\n'].join('\r\n'));

  let buffer = Buffer.alloc(0);
  let child = null;
  let outputPath = null;
  let completed = false;
  let stderr = '';
  let expectedFrames = 0;
  let receivedFrames = 0;
  let metadata = null;
  let fragmentedOpcode = null;
  let fragments = [];
  let parsing = Promise.resolve();
  const stop = async () => {
    if (!completed) child?.kill();
    if (!completed && outputPath) await removeTemporaryFile(outputPath);
  };
  socket.on('close', stop);
  socket.on('error', stop);

  const startEncoder = async (message) => {
    const backends = await getBackends();
    const codec = message.codec === 'h264' ? 'h264' : 'h265';
    const encoder = codec === 'h264' ? 'h264_nvenc' : 'hevc_nvenc';
    const available = new Set(backends.encoders.filter((entry) => entry.available).map((entry) => entry.id));
    if (!available.has(encoder)) throw new Error(`RTX 2080 Ti NVENC ${codec.toUpperCase()} 無法使用`);
    const fps = normaliseFps(message.fps);
    expectedFrames = boundedInteger(message.frames, 2, 216_000, fps * 20);
    metadata = {
      codec,
      encoder,
      name: String(message.name ?? 'gpx-animation'),
      width: boundedInteger(message.width, 320, 7680, 1920),
      height: boundedInteger(message.height, 240, 7680, 1080),
      frames: expectedFrames,
      fps,
    };
    const jobDir = join(tmpdir(), 'gpx-animator-gpu');
    await mkdir(jobDir, { recursive: true });
    outputPath = join(jobDir, `${randomUUID()}.mp4`);
    const args = [
      '-hide_banner', '-loglevel', 'error', '-y',
      '-hwaccel', 'cuda', '-hwaccel_device', '0', '-hwaccel_output_format', 'cuda',
      '-c:v', 'mjpeg_cuvid', '-f', 'image2pipe', '-framerate', String(fps), '-i', 'pipe:0',
      '-frames:v', String(expectedFrames), '-an', ...encoderArgs(encoder),
      ...(codec === 'h265' ? ['-tag:v', 'hvc1'] : []), '-movflags', '+faststart', outputPath,
    ];
    child = spawn('ffmpeg', args, { windowsHide: true });
    child.stderr.on('data', (chunk) => { stderr = `${stderr}${chunk}`.slice(-8_000); });
    child.stdin.on('error', () => {});
    child.on('error', (error) => sendSocketJson(socket, { type: 'error', error: error.message }));
    child.on('close', async (code) => {
      if (code !== 0 || receivedFrames !== expectedFrames) {
        sendSocketJson(socket, { type: 'error', error: 'RTX NVENC 編碼失敗', detail: stderr.slice(-2_000) });
        await removeTemporaryFile(outputPath);
        return;
      }
      const probe = await run('ffprobe', [
        '-v', 'error', '-select_streams', 'v:0',
        '-show_entries', 'stream=codec_name,width,height,nb_frames,avg_frame_rate',
        '-of', 'json', outputPath,
      ], { timeout: 15_000 });
      let streamInfo = null;
      try { streamInfo = JSON.parse(probe.stdout).streams?.[0] ?? null; } catch {}
      const expectedCodec = codec === 'h265' ? 'hevc' : 'h264';
      if (!probe.ok || streamInfo?.codec_name !== expectedCodec
        || Number(streamInfo.width) !== metadata.width
        || Number(streamInfo.height) !== metadata.height
        || Number(streamInfo.nb_frames) !== expectedFrames) {
        sendSocketJson(socket, { type: 'error', error: '輸出影片驗證失敗', detail: probe.stderr || JSON.stringify(streamInfo) });
        await removeTemporaryFile(outputPath);
        return;
      }
      metadata.probe = streamInfo;
      completed = true;
      const token = randomUUID();
      const cleanupTimer = setTimeout(() => {
        if (renderResults.delete(token)) removeTemporaryFile(outputPath);
      }, 10 * 60_000);
      cleanupTimer.unref();
      renderResults.set(token, { path: outputPath, metadata, cleanupTimer, expires: Date.now() + 10 * 60_000 });
      sendSocketJson(socket, { type: 'complete', token, encoder: metadata.encoder, codec: metadata.codec, probe: streamInfo });
    });
  };

  const handleMessage = async (opcode, payload) => {
    if (opcode === 0x8) { socket.end(); return; }
    if (opcode === 0x9) { socket.write(Buffer.from([0x8a, 0])); return; }
    if (opcode === 0x1) {
      const message = JSON.parse(payload.toString('utf8'));
      if (message.type === 'start' && !child) await startEncoder(message);
      else if (message.type === 'end' && child) child.stdin.end();
    } else if (opcode === 0x2 && child?.stdin.writable) {
      receivedFrames += 1;
      if (receivedFrames > expectedFrames) throw new Error('收到過多影片畫格');
      if (!child.stdin.write(payload)) { socket.pause(); child.stdin.once('drain', () => socket.resume()); }
    }
  };

  socket.on('data', (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    const parse = async () => {
      while (buffer.length >= 2) {
        const finalFragment = Boolean(buffer[0] & 0x80);
        const opcode = buffer[0] & 0x0f;
        const masked = Boolean(buffer[1] & 0x80);
        let length = buffer[1] & 0x7f;
        let offset = 2;
        if (length === 126) { if (buffer.length < 4) return; length = buffer.readUInt16BE(2); offset = 4; }
        else if (length === 127) { if (buffer.length < 10) return; const value = buffer.readBigUInt64BE(2); if (value > BigInt(MAX_UPLOAD)) throw new Error('單一畫格過大'); length = Number(value); offset = 10; }
        const maskSize = masked ? 4 : 0;
        if (buffer.length < offset + maskSize + length) return;
        const mask = masked ? buffer.subarray(offset, offset + 4) : null;
        const payload = Buffer.from(buffer.subarray(offset + maskSize, offset + maskSize + length));
        buffer = buffer.subarray(offset + maskSize + length);
        if (mask) for (let index = 0; index < payload.length; index += 1) payload[index] ^= mask[index % 4];
        if (opcode === 0x0) {
          if (fragmentedOpcode === null) throw new Error('收到無起始訊息的 WebSocket 分片');
          fragments.push(payload);
          if (finalFragment) {
            const completePayload = Buffer.concat(fragments);
            const completeOpcode = fragmentedOpcode;
            fragmentedOpcode = null;
            fragments = [];
            await handleMessage(completeOpcode, completePayload);
          }
        } else if ((opcode === 0x1 || opcode === 0x2) && !finalFragment) {
          fragmentedOpcode = opcode;
          fragments = [payload];
        } else {
          await handleMessage(opcode, payload);
        }
      }
    };
    parsing = parsing.then(parse).catch((error) => { sendSocketJson(socket, { type: 'error', error: error.message }); socket.end(); });
  });
}

async function serveRenderResult(res, url) {
  const token = url.searchParams.get('id');
  const result = token ? renderResults.get(token) : null;
  if (!result || result.expires < Date.now()) { json(res, 404, { error: '影片結果已不存在' }); return; }
  renderResults.delete(token);
  clearTimeout(result.cleanupTimer);
  const info = await stat(result.path);
  const downloadName = result.metadata.name.replace(/[^a-zA-Z0-9._-]+/g, '-').replace(/^-+|-+$/g, '') || 'gpx-animation';
  res.writeHead(200, { 'Content-Type': 'video/mp4', 'Content-Length': info.size, 'Content-Disposition': `attachment; filename="${downloadName}.mp4"`, 'X-GPX-Encoder': result.metadata.encoder, 'X-GPX-Codec': result.metadata.codec, 'X-GPX-GPU': 'NVIDIA GeForce RTX 2080 Ti', 'X-GPX-Width': String(result.metadata.width), 'X-GPX-Height': String(result.metadata.height), 'X-GPX-Frames': String(result.metadata.frames), 'Cache-Control': 'no-store' });
  const stream = createReadStream(result.path);
  stream.pipe(res);
  stream.on('close', () => removeTemporaryFile(result.path));
}

export function createServer() {
  const server = http.createServer(async (req, res) => {
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
      if (req.method === 'POST' && url.pathname === '/api/render-mp4') {
        await renderMp4(req, res, url);
        return;
      }
      if (req.method === 'GET' && url.pathname === '/api/render-result') {
        await serveRenderResult(res, url);
        return;
      }
      if (req.method === 'GET' || req.method === 'HEAD') {
        await serveStatic(req, res, url.pathname);
        return;
      }
      json(res, 405, { error: 'Method not allowed' });
    } catch (error) {
      json(res, 500, { error: error.message });
    }
  });
  server.on('upgrade', handleRenderSocket);
  return server;
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

const isEntry = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isEntry) {
  const portIndex = process.argv.indexOf('--port');
  const port = portIndex >= 0 ? Number(process.argv[portIndex + 1]) : DEFAULT_PORT;
  const hostIndex = process.argv.indexOf('--host');
  const host = hostIndex >= 0 ? String(process.argv[hostIndex + 1]) : DEFAULT_HOST;
  const shouldOpen = process.argv.includes('--open');
  const server = createServer();
  server.listen(port, host, () => {
    const displayHost = host.includes(':') ? `[${host}]` : host;
    const url = `http://${displayHost}:${port}`;
    console.log(`GPX Animator GPU: ${url}`);
    if (shouldOpen) openBrowser(url);
    getBackends().then((backends) => {
      console.log(backends.ffmpeg
        ? `MP4 encoder: ${backends.selected ?? backends.fallback}`
        : 'FFmpeg not found; WebM export remains available.');
    });
  });
}
