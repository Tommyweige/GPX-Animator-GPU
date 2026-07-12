import { drawElevationOverlay, drawHudOverlay, drawMapCredit } from './overlay.js';

function supportedMimeType() {
  const choices = [
    'video/webm;codecs=vp9',
    'video/webm;codecs=vp8',
    'video/webm',
  ];
  return choices.find((type) => MediaRecorder.isTypeSupported(type)) ?? '';
}

function abortError() {
  return new DOMException('匯出已取消', 'AbortError');
}

export async function recordAnimation({
  renderer,
  track,
  width,
  height,
  fps,
  durationSeconds,
  options,
  signal,
  onProgress = () => {},
}) {
  if (!window.MediaRecorder || !HTMLCanvasElement.prototype.captureStream) {
    throw new Error('此瀏覽器不支援 Canvas 影片錄製；請使用最新版 Chrome、Edge 或 Firefox。');
  }
  if (signal?.aborted) throw abortError();

  const output = document.createElement('canvas');
  output.width = width;
  output.height = height;
  const context = output.getContext('2d', {
    alpha: options.mapStyle === 'transparent',
    desynchronized: true,
  });
  const previousWidth = renderer.canvas.width;
  const previousHeight = renderer.canvas.height;
  let stream = null;
  let recorder = null;
  let frameHandle = 0;
  let abortHandler = null;

  try {
    renderer.resize(width, height);
    onProgress(0, '準備地圖圖磚…');
    const prefetchSteps = options.cameraMode === 'follow' ? 12 : 1;
    for (let index = 0; index < prefetchSteps; index += 1) {
      if (signal?.aborted) throw abortError();
      renderer.render(index / Math.max(1, prefetchSteps - 1), options);
    }
    await Promise.race([
      renderer.waitForTiles(5_000),
      new Promise((_, reject) => {
        if (!signal) return;
        abortHandler = () => reject(abortError());
        signal.addEventListener('abort', abortHandler, { once: true });
      }),
    ]);
    if (signal?.aborted) throw abortError();
    if (abortHandler) signal.removeEventListener('abort', abortHandler);

    const mimeType = supportedMimeType();
    const bitrate = Math.max(4_000_000, Math.min(45_000_000, Math.round(width * height * fps * 0.14)));
    stream = output.captureStream(fps);
    recorder = new MediaRecorder(stream, { mimeType, videoBitsPerSecond: bitrate });
    const chunks = [];
    recorder.ondataavailable = (event) => { if (event.data.size) chunks.push(event.data); };

    const stopped = new Promise((resolve, reject) => {
      recorder.onerror = () => reject(recorder.error ?? new Error('瀏覽器影片編碼失敗'));
      recorder.onstop = () => resolve(new Blob(chunks, { type: mimeType || 'video/webm' }));
    });
    const aborted = new Promise((_, reject) => {
      if (!signal) return;
      abortHandler = () => {
        cancelAnimationFrame(frameHandle);
        if (recorder?.state !== 'inactive') recorder.stop();
        reject(abortError());
      };
      signal.addEventListener('abort', abortHandler, { once: true });
    });

    const drawFrame = (progress) => {
      const sample = renderer.render(progress, options);
      context.clearRect(0, 0, width, height);
      context.drawImage(renderer.canvas, 0, 0, width, height);
      if (options.showElevation) drawElevationOverlay(context, track, progress, { color: options.routeColor, width, height });
      if (options.showHud) drawHudOverlay(context, sample, { width, height });
      if (options.mapStyle !== 'transparent') drawMapCredit(context, { width, height });
    };

    drawFrame(0);
    recorder.start(1_000);
    const startedAt = performance.now();
    const frames = new Promise((resolveFrames, rejectFrames) => {
      const tick = (now) => {
        if (signal?.aborted) { rejectFrames(abortError()); return; }
        const progress = Math.min(1, (now - startedAt) / (durationSeconds * 1000));
        drawFrame(progress);
        onProgress(progress, `WebGL2 繪製 ${Math.round(progress * durationSeconds * fps)} / ${Math.round(durationSeconds * fps)} 幀`);
        if (progress >= 1) {
          frameHandle = requestAnimationFrame(resolveFrames);
        } else {
          frameHandle = requestAnimationFrame(tick);
        }
      };
      frameHandle = requestAnimationFrame(tick);
    });

    await Promise.race([frames, aborted, stopped]);
    if (recorder.state !== 'inactive') recorder.stop();
    return await Promise.race([stopped, aborted]);
  } finally {
    if (abortHandler && signal) signal.removeEventListener('abort', abortHandler);
    cancelAnimationFrame(frameHandle);
    if (recorder?.state !== 'inactive') recorder.stop();
    stream?.getTracks().forEach((trackItem) => trackItem.stop());
    renderer.resize(previousWidth, previousHeight);
  }
}

export async function transcodeToMp4(webm, { name, codec = 'h265', fps = 30, signal, onStatus = () => {} } = {}) {
  onStatus('正在交給 FFmpeg 硬體編碼器…');
  const encoder = codec === 'h264' ? 'h264_nvenc' : 'hevc_nvenc';
  const params = new URLSearchParams({ encoder, codec, fps: String(fps), name: name || 'gpx-animation' });
  const response = await fetch(`/api/transcode?${params}`, {
    method: 'POST',
    headers: { 'Content-Type': webm.type || 'video/webm' },
    body: webm,
    signal,
  });
  if (!response.ok) {
    let detail;
    try { detail = await response.json(); } catch { detail = { error: response.statusText }; }
    throw new Error(detail.detail ? `${detail.error}：${detail.detail}` : detail.error);
  }
  return {
    blob: await response.blob(),
    encoder: response.headers.get('X-GPX-Encoder') ?? 'unknown',
    codec: response.headers.get('X-GPX-Codec') ?? codec,
  };
}

export function downloadBlob(blob, filename) {
  const url = URL.createObjectURL(blob);
  const anchor = document.createElement('a');
  anchor.href = url;
  anchor.download = filename;
  document.body.append(anchor);
  anchor.click();
  anchor.remove();
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}

function canvasBlob(canvas, type = 'image/jpeg', quality = 0.92) {
  return new Promise((resolve, reject) => {
    canvas.toBlob((blob) => {
      if (blob) resolve(blob);
      else reject(new Error('無法建立影片畫格'));
    }, type, quality);
  });
}

export async function renderAnimationToMp4({
  renderer,
  track,
  width,
  height,
  fps,
  durationSeconds,
  options,
  codec = 'h265',
  name = 'gpx-animation',
  signal,
  onProgress = () => {},
}) {
  if (signal?.aborted) throw abortError();
  const laneCount = Math.min(4, Math.max(2, Math.floor((navigator.hardwareConcurrency || 8) / 4)));
  const lanes = Array.from({ length: laneCount }, () => {
    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    return { canvas, context: canvas.getContext('2d', { alpha: false, desynchronized: true }) };
  });
  const previousWidth = renderer.canvas.width;
  const previousHeight = renderer.canvas.height;
  const frameCount = Math.max(2, Math.round(durationSeconds * fps));
  let frameIndex = 0;

  renderer.resize(width, height);
  try {
    onProgress(0, '準備 RTX 影片串流…');
    const prefetchSteps = options.cameraMode === 'follow' ? 12 : 1;
    for (let index = 0; index < prefetchSteps; index += 1) {
      if (signal?.aborted) throw abortError();
      renderer.render(index / Math.max(1, prefetchSteps - 1), options);
    }
    await renderer.waitForTiles(5_000);
    if (signal?.aborted) throw abortError();

    const encoder = codec === 'h264' ? 'h264_nvenc' : 'hevc_nvenc';
    const socketUrl = `${location.protocol === 'https:' ? 'wss:' : 'ws:'}//${location.host}/api/render-stream`;
    const result = await new Promise((resolve, reject) => {
      const socket = new WebSocket(socketUrl);
      socket.binaryType = 'arraybuffer';
      let settled = false;
      const finish = (callback, value) => {
        if (settled) return;
        settled = true;
        signal?.removeEventListener('abort', cancel);
        callback(value);
      };
      const cancel = () => {
        socket.close(1000, 'cancelled');
        finish(reject, abortError());
      };
      signal?.addEventListener('abort', cancel, { once: true });
      socket.onerror = () => finish(reject, new Error('無法連接本機 RTX 編碼器'));
      socket.onclose = () => {
        if (!settled) finish(reject, signal?.aborted ? abortError() : new Error('RTX 編碼連線提前結束'));
      };
      socket.onmessage = (event) => {
        const message = JSON.parse(String(event.data));
        if (message.type === 'complete') finish(resolve, message);
        else if (message.type === 'error') finish(reject, new Error(message.detail ? `${message.error}：${message.detail}` : message.error));
      };
      socket.onopen = async () => {
        try {
          socket.send(JSON.stringify({ type: 'start', codec, fps, frames: frameCount, width, height, name }));
          let producedFrames = 0;
          const pendingFrames = [];
          while (frameIndex < frameCount) {
            if (signal?.aborted) throw abortError();
            while (producedFrames < frameCount && pendingFrames.length < laneCount) {
              const lane = lanes[producedFrames % laneCount];
              const progress = producedFrames / Math.max(1, frameCount - 1);
              const sample = renderer.render(progress, options);
              lane.context.drawImage(renderer.canvas, 0, 0, width, height);
              if (options.showElevation) drawElevationOverlay(lane.context, track, progress, { color: options.routeColor, width, height });
              if (options.showHud) drawHudOverlay(lane.context, sample, { width, height });
              if (options.mapStyle !== 'transparent') drawMapCredit(lane.context, { width, height });
              pendingFrames.push(canvasBlob(lane.canvas).then((blob) => blob.arrayBuffer()));
              producedFrames += 1;
            }
            while (socket.bufferedAmount > 32 * 1024 * 1024) {
              await new Promise((resolveWait) => setTimeout(resolveWait, 4));
              if (signal?.aborted) throw abortError();
            }
            socket.send(await pendingFrames.shift());
            frameIndex += 1;
            onProgress(frameIndex / frameCount, `RTX 串流 ${frameIndex} / ${frameCount} 幀`);
          }
          socket.send(JSON.stringify({ type: 'end' }));
        } catch (error) {
          socket.close();
          finish(reject, error);
        }
      };
    });
    const response = await fetch(`/api/render-result?id=${encodeURIComponent(result.token)}`, { signal });
    if (!response.ok) {
      let detail;
      try { detail = await response.json(); } catch { detail = { error: response.statusText }; }
      throw new Error(detail.detail ? `${detail.error}：${detail.detail}` : detail.error);
    }
    return {
      blob: await response.blob(),
      encoder: response.headers.get('X-GPX-Encoder') ?? result.encoder ?? encoder,
      codec: response.headers.get('X-GPX-Codec') ?? codec,
    };
  } finally {
    renderer.resize(previousWidth, previousHeight);
  }
}
