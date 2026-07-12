import { renderAnimationToMp4, downloadBlob } from './exporter.js';
import { formatDistance, formatDuration } from './geo.js';
import { parseGpx } from './gpx.js';
import { drawElevationOverlay } from './overlay.js';
import { GpuMapRenderer } from './renderer.js';

const $ = (selector) => document.querySelector(selector);
const elements = {
  canvas: $('#mapCanvas'),
  elevationCanvas: $('#elevationCanvas'),
  stageWrap: $('#stageWrap'),
  emptyState: $('#emptyState'),
  emptyLoadButton: $('#emptyLoadButton'),
  webglStatus: $('#webglStatus'),
  encoderStatus: $('#encoderStatus'),
  fileInput: $('#fileInput'),
  dropZone: $('#dropZone'),
  sampleButton: $('#sampleButton'),
  clearButton: $('#clearButton'),
  trackSummary: $('#trackSummary'),
  trackName: $('#trackName'),
  totalDistance: $('#totalDistance'),
  totalClimb: $('#totalClimb'),
  pointCount: $('#pointCount'),
  trackDataNote: $('#trackDataNote'),
  mapStyle: $('#mapStyle'),
  cameraMode: $('#cameraMode'),
  routeColor: $('#routeColor'),
  markerColor: $('#markerColor'),
  lineWidth: $('#lineWidth'),
  lineWidthValue: $('#lineWidthValue'),
  showHud: $('#showHud'),
  showElevation: $('#showElevation'),
  duration: $('#duration'),
  fps: $('#fps'),
  videoCodec: $('#videoCodec'),
  resolution: $('#resolution'),
  exportButton: $('#exportButton'),
  exportNote: $('#exportNote'),
  playButton: $('#playButton'),
  restartButton: $('#restartButton'),
  timeline: $('#timeline'),
  currentTime: $('#currentTime'),
  totalTime: $('#totalTime'),
  hud: $('#hud'),
  hudDistance: $('#hudDistance'),
  hudElevation: $('#hudElevation'),
  hudTime: $('#hudTime'),
  mapCredit: $('#mapCredit'),
  exportOverlay: $('#exportOverlay'),
  progressRing: $('#progressRing'),
  exportPercent: $('#exportPercent'),
  exportTitle: $('#exportTitle'),
  exportDetail: $('#exportDetail'),
  cancelExport: $('#cancelExport'),
  helpButton: $('#helpButton'),
  helpDialog: $('#helpDialog'),
  zoomOutButton: $('#zoomOutButton'),
  zoomInButton: $('#zoomInButton'),
  resetViewButton: $('#resetViewButton'),
  toast: $('#toast'),
};

const state = {
  track: null,
  progress: 0,
  playing: false,
  playStartedAt: 0,
  playStartProgress: 0,
  animationFrame: 0,
  exporting: false,
  exportController: null,
  backends: null,
  toastTimer: 0,
  lastAutomaticCameraMode: 'fit',
};

let renderer;
try {
  renderer = new GpuMapRenderer(elements.canvas, {
    onInvalidate: () => { if (!state.exporting) renderPreview(); },
  });
  const gpu = renderer.getGpuInfo();
  if (!/NVIDIA|GeForce|RTX/i.test(gpu.renderer)) {
    throw new Error(`目前 WebGL2 使用 ${gpu.renderer}，已阻止 Intel／整合顯卡渲染。請將瀏覽器設為 Windows「高效能」GPU 後重新開啟。`);
  }
  elements.webglStatus.innerHTML = '<i></i>RTX 2080 Ti · WebGL2';
  elements.webglStatus.title = `${gpu.renderer}\n${gpu.version}\nMax texture: ${gpu.maxTextureSize}`;
} catch (error) {
  elements.webglStatus.textContent = 'WebGL2 無法啟用';
  elements.webglStatus.classList.add('error');
  elements.emptyState.querySelector('h1').textContent = '無法啟用 GPU 繪圖';
  elements.emptyState.querySelector('p').textContent = error.message;
  elements.emptyLoadButton.disabled = true;
}

function options() {
  return {
    mapStyle: elements.mapStyle.value,
    cameraMode: elements.cameraMode.value,
    routeColor: elements.routeColor.value,
    markerColor: elements.markerColor.value,
    lineWidth: Number(elements.lineWidth.value),
    showHud: elements.showHud.checked,
    showElevation: elements.showElevation.checked,
  };
}

function showToast(message, type = '') {
  clearTimeout(state.toastTimer);
  elements.toast.textContent = message;
  elements.toast.className = `toast show ${type}`.trim();
  state.toastTimer = setTimeout(() => { elements.toast.className = 'toast'; }, 4_000);
}

function updateTransport() {
  const durationMs = Number(elements.duration.value) * 1000;
  elements.timeline.value = Math.round(state.progress * 1000);
  elements.currentTime.textContent = formatDuration(state.progress * durationMs, true);
  elements.totalTime.textContent = formatDuration(durationMs, true);
  elements.playButton.classList.toggle('playing', state.playing);
}

function resizeElevationCanvas() {
  if (!renderer) return;
  if (elements.elevationCanvas.width !== renderer.canvas.width) elements.elevationCanvas.width = renderer.canvas.width;
  if (elements.elevationCanvas.height !== renderer.canvas.height) elements.elevationCanvas.height = renderer.canvas.height;
}

function updateHud(sample) {
  const visible = Boolean(state.track && elements.showHud.checked);
  elements.hud.classList.toggle('hidden', !visible);
  if (!sample || !visible) return;
  elements.hudDistance.textContent = `${(sample.distanceM / 1000).toFixed(2)} km`;
  elements.hudElevation.textContent = Number.isFinite(sample.ele) ? `${Math.round(sample.ele)} m` : '— m';
  elements.hudTime.textContent = `${Math.round(sample.progress * 100)}%`;
}

function renderPreview() {
  if (!renderer || state.exporting) return;
  renderer.resize();
  resizeElevationCanvas();
  const currentOptions = options();
  const sample = renderer.render(state.progress, currentOptions);
  const elevationContext = elements.elevationCanvas.getContext('2d');
  elevationContext.clearRect(0, 0, elements.elevationCanvas.width, elements.elevationCanvas.height);
  if (state.track && currentOptions.showElevation) {
    drawElevationOverlay(elevationContext, state.track, state.progress, {
      color: currentOptions.routeColor,
      width: elements.elevationCanvas.width,
      height: elements.elevationCanvas.height,
      preview: true,
    });
  }
  updateHud(sample);
  elements.mapCredit.classList.toggle('hidden', !state.track || currentOptions.mapStyle === 'transparent');
  updateTransport();
}

function setPlaying(value) {
  if (!state.track || state.exporting) return;
  state.playing = value;
  cancelAnimationFrame(state.animationFrame);
  if (value) {
    if (state.progress >= 1) state.progress = 0;
    state.playStartedAt = performance.now();
    state.playStartProgress = state.progress;
    state.animationFrame = requestAnimationFrame(animate);
  }
  updateTransport();
}

function animate(now) {
  if (!state.playing) return;
  const durationMs = Math.max(2_000, Number(elements.duration.value) * 1000);
  state.progress = state.playStartProgress + (now - state.playStartedAt) / durationMs;
  if (state.progress >= 1) {
    state.progress = 1;
    state.playing = false;
  }
  renderPreview();
  if (state.playing) state.animationFrame = requestAnimationFrame(animate);
}

function syncExportButtons() {
  const unavailable = !state.track || state.exporting || !renderer || !window.WebSocket;
  elements.exportButton.disabled = unavailable || !state.backends?.ffmpeg;
  elements.playButton.disabled = !state.track || state.exporting;
  elements.restartButton.disabled = !state.track || state.exporting;
  elements.timeline.disabled = !state.track || state.exporting;
  elements.zoomOutButton.disabled = !state.track || state.exporting;
  elements.zoomInButton.disabled = !state.track || state.exporting;
  elements.resetViewButton.disabled = !state.track || state.exporting;
}

function loadTrack(track) {
  state.track = track;
  state.progress = 0;
  state.playing = false;
  state.lastAutomaticCameraMode = 'fit';
  elements.cameraMode.value = 'fit';
  renderer.setTrack(track);
  elements.emptyState.classList.add('hidden');
  elements.trackSummary.classList.remove('hidden');
  elements.trackName.textContent = track.name;
  elements.trackName.title = track.name;
  elements.totalDistance.textContent = formatDistance(track.distanceM);
  elements.totalClimb.textContent = `${Math.round(track.elevationGainM)} m`;
  elements.pointCount.textContent = `${track.points.length.toLocaleString('zh-TW')} 個`;
  elements.trackDataNote.textContent = `原始 GPS 記錄 ${track.sourcePointCount.toLocaleString('zh-TW')} 點；動畫使用 ${track.points.length.toLocaleString('zh-TW')} 點，已略過 ${track.skippedStopCount} 段停留（${track.removedStopPointCount} 點）。播放依路線距離前進。`;
  if (track.synthesizedTime) {
    showToast('部分或全部時間戳已依距離自動補值。');
  } else if (track.skippedStopCount || track.pauseCount) {
    showToast(`已略過 ${track.skippedStopCount} 段停留，動畫會以固定路線速度播放。`);
  }
  syncExportButtons();
  renderPreview();
}

async function loadFile(file) {
  if (!file) return;
  if (file.size > 100 * 1024 * 1024) {
    showToast('GPX 檔案超過 100 MB，請先簡化軌跡。', 'error');
    return;
  }
  try {
    const track = parseGpx(await file.text());
    if (track.name === '未命名軌跡') track.name = file.name.replace(/\.(?:gpx|trk)$/i, '');
    loadTrack(track);
  } catch (error) {
    showToast(error.message, 'error');
  }
}

async function loadSample() {
  try {
    const response = await fetch('/samples/taipei-riverside.gpx');
    if (!response.ok) throw new Error('無法讀取範例軌跡。');
    loadTrack(parseGpx(await response.text()));
  } catch (error) {
    showToast(error.message, 'error');
  }
}

function clearTrack() {
  setPlaying(false);
  state.track = null;
  state.progress = 0;
  renderer.setTrack(null);
  elements.fileInput.value = '';
  elements.trackSummary.classList.add('hidden');
  elements.emptyState.classList.remove('hidden');
  elements.hud.classList.add('hidden');
  elements.mapCredit.classList.add('hidden');
  syncExportButtons();
  renderPreview();
}

function setExportProgress(progress, detail) {
  const value = Math.round(progress * 100);
  elements.exportPercent.textContent = `${value}%`;
  elements.progressRing.style.setProperty('--progress', `${value * 3.6}deg`);
  elements.exportDetail.textContent = detail;
}

async function exportVideo() {
  if (!state.track || state.exporting) return;
  setPlaying(false);
  state.exporting = true;
  state.exportController = new AbortController();
  syncExportButtons();
  elements.exportOverlay.classList.remove('hidden');
  elements.exportTitle.textContent = '正在以 GPU 繪製影片';
  setExportProgress(0, '準備 WebGL2 輸出畫面…');

  const [width, height] = elements.resolution.value.split('x').map(Number);
  const fps = Number(elements.fps.value);
  const durationSeconds = Math.max(2, Number(elements.duration.value));
  const filename = state.track.name.replace(/[^\p{L}\p{N}._-]+/gu, '-').replace(/^-+|-+$/g, '') || 'gpx-animation';

  try {
    const result = await renderAnimationToMp4({
      renderer,
      track: state.track,
      width,
      height,
      fps,
      durationSeconds,
      options: options(),
      signal: state.exportController.signal,
      onProgress: setExportProgress,
      codec: elements.videoCodec.value,
      name: filename,
    });
    downloadBlob(result.blob, `${filename}.mp4`);
    showToast(`MP4 已完成，編碼器：${result.encoder}／${result.codec.toUpperCase()}。`);
  } catch (error) {
    if (error.name !== 'AbortError') showToast(error.message, 'error');
  } finally {
    state.exporting = false;
    state.exportController = null;
    elements.exportOverlay.classList.add('hidden');
    renderer.resize();
    syncExportButtons();
    renderPreview();
  }
}

async function detectBackends() {
  try {
    const response = await fetch('/api/system');
    state.backends = await response.json();
    if (!state.backends.ffmpeg) {
      elements.encoderStatus.textContent = 'FFmpeg 未安裝';
      elements.encoderStatus.classList.add('error');
      elements.exportNote.textContent = '找不到 FFmpeg；仍可直接匯出 WebM。';
    } else if (state.backends.selected) {
      const selected = state.backends.encoders.find((entry) => entry.id === state.backends.selected);
      elements.encoderStatus.innerHTML = `<i></i>${selected?.label ?? state.backends.selected}`;
      elements.encoderStatus.classList.remove('muted');
      elements.exportNote.textContent = `已鎖定 ${selected?.label ?? state.backends.selected}；NVENC 無法使用時會停止輸出，不會改用 Intel QSV 或 CPU。`;
    } else {
      elements.encoderStatus.textContent = 'CPU · libx264';
      elements.exportNote.textContent = '未找到可用硬體 H.264 編碼器；繪圖仍走 GPU，MP4 以 CPU 回退。';
    }
  } catch {
    state.backends = { ffmpeg: false };
    elements.encoderStatus.textContent = '編碼器偵測失敗';
    elements.encoderStatus.classList.add('error');
  }
  syncExportButtons();
}

const aspectResolutions = {
  '16/9': [
    ['1280x720', 'HD · 1280 × 720'],
    ['1920x1080', 'Full HD · 1920 × 1080'],
    ['2560x1440', '2K · 2560 × 1440'],
    ['3840x2160', '4K · 3840 × 2160'],
  ],
  '1/1': [
    ['1080x1080', 'Square · 1080 × 1080'],
    ['1440x1440', 'Square 2K · 1440 × 1440'],
    ['2160x2160', 'Square 4K · 2160 × 2160'],
  ],
  '9/16': [
    ['720x1280', 'Vertical HD · 720 × 1280'],
    ['1080x1920', 'Vertical FHD · 1080 × 1920'],
    ['1440x2560', 'Vertical 2K · 1440 × 2560'],
  ],
};

document.querySelectorAll('[data-aspect]').forEach((button) => {
  button.addEventListener('click', () => {
    document.querySelectorAll('[data-aspect]').forEach((item) => item.classList.toggle('active', item === button));
    const aspect = button.dataset.aspect;
    elements.stageWrap.style.aspectRatio = aspect;
    elements.stageWrap.style.width = aspect === '9/16'
      ? 'min(calc(100% - 46px), calc((100vh - 210px) * 9 / 16))'
      : aspect === '1/1'
        ? 'min(calc(100% - 46px), calc(100vh - 210px))'
        : '';
    elements.resolution.innerHTML = aspectResolutions[aspect]
      .map(([value, label], index) => `<option value="${value}" ${index === 1 ? 'selected' : ''}>${label}</option>`)
      .join('');
    requestAnimationFrame(renderPreview);
  });
});

elements.emptyLoadButton.addEventListener('click', () => elements.fileInput.click());
elements.sampleButton.addEventListener('click', loadSample);
elements.clearButton.addEventListener('click', clearTrack);
elements.fileInput.addEventListener('change', () => loadFile(elements.fileInput.files[0]));
elements.dropZone.addEventListener('keydown', (event) => { if (event.key === 'Enter' || event.key === ' ') elements.fileInput.click(); });
['dragenter', 'dragover'].forEach((name) => elements.dropZone.addEventListener(name, (event) => {
  event.preventDefault();
  elements.dropZone.classList.add('dragover');
}));
['dragleave', 'drop'].forEach((name) => elements.dropZone.addEventListener(name, (event) => {
  event.preventDefault();
  elements.dropZone.classList.remove('dragover');
}));
elements.dropZone.addEventListener('drop', (event) => loadFile(event.dataTransfer.files[0]));

elements.playButton.addEventListener('click', () => setPlaying(!state.playing));
elements.restartButton.addEventListener('click', () => { setPlaying(false); state.progress = 0; renderPreview(); });
elements.timeline.addEventListener('input', () => { setPlaying(false); state.progress = Number(elements.timeline.value) / 1000; renderPreview(); });
elements.exportButton.addEventListener('click', exportVideo);
elements.cancelExport.addEventListener('click', () => state.exportController?.abort());
elements.helpButton.addEventListener('click', () => elements.helpDialog.showModal());

[elements.mapStyle, elements.routeColor, elements.markerColor, elements.showHud, elements.showElevation]
  .forEach((control) => control.addEventListener('input', renderPreview));
elements.cameraMode.addEventListener('input', () => {
  const selected = elements.cameraMode.value;
  if (selected === 'free') {
    renderer.beginFreeCamera(state.progress, state.lastAutomaticCameraMode);
  } else {
    state.lastAutomaticCameraMode = selected;
    renderer.clearManualCamera();
  }
  renderPreview();
});
elements.lineWidth.addEventListener('input', () => {
  elements.lineWidthValue.textContent = `${elements.lineWidth.value} px`;
  renderPreview();
});
elements.duration.addEventListener('input', updateTransport);
elements.videoCodec.addEventListener('input', () => {
  if (!state.backends?.ffmpeg) return;
  elements.exportNote.textContent = elements.videoCodec.value === 'h265'
    ? 'H.265 優先使用 NVIDIA NVENC；若不可用會自動改用其他 H.265 或相容後備編碼器。'
    : 'H.264 相容模式會優先使用可用的硬體編碼器。';
});

function activateFreeCamera() {
  if (!state.track || !renderer) return false;
  const currentMode = elements.cameraMode.value;
  if (currentMode !== 'free') {
    renderer.beginFreeCamera(state.progress, currentMode);
    state.lastAutomaticCameraMode = currentMode;
    elements.cameraMode.value = 'free';
  }
  return true;
}

function resetMapView() {
  if (!state.track || state.exporting) return;
  renderer.resetCamera();
  elements.cameraMode.value = 'fit';
  state.lastAutomaticCameraMode = 'fit';
  renderPreview();
}

elements.zoomInButton.addEventListener('click', () => {
  if (!activateFreeCamera()) return;
  renderer.zoomBy(1.35);
  renderPreview();
});
elements.zoomOutButton.addEventListener('click', () => {
  if (!activateFreeCamera()) return;
  renderer.zoomBy(1 / 1.35);
  renderPreview();
});
elements.resetViewButton.addEventListener('click', resetMapView);

const drag = { pointerId: null, x: 0, y: 0 };
elements.canvas.addEventListener('pointerdown', (event) => {
  if (event.button !== 0 || state.exporting || !activateFreeCamera()) return;
  setPlaying(false);
  drag.pointerId = event.pointerId;
  drag.x = event.clientX;
  drag.y = event.clientY;
  elements.canvas.setPointerCapture(event.pointerId);
  elements.stageWrap.classList.add('map-dragging');
  event.preventDefault();
});
elements.canvas.addEventListener('pointermove', (event) => {
  if (event.pointerId !== drag.pointerId) return;
  const deltaX = event.clientX - drag.x;
  const deltaY = event.clientY - drag.y;
  drag.x = event.clientX;
  drag.y = event.clientY;
  renderer.panByPixels(deltaX, deltaY);
  renderPreview();
});
function finishMapDrag(event) {
  if (event.pointerId !== drag.pointerId) return;
  if (elements.canvas.hasPointerCapture(event.pointerId)) elements.canvas.releasePointerCapture(event.pointerId);
  drag.pointerId = null;
  elements.stageWrap.classList.remove('map-dragging');
}
elements.canvas.addEventListener('pointerup', finishMapDrag);
elements.canvas.addEventListener('pointercancel', finishMapDrag);
elements.canvas.addEventListener('wheel', (event) => {
  if (state.exporting || !activateFreeCamera()) return;
  const exponent = Math.min(0.7, Math.max(-0.7, -event.deltaY * 0.0015));
  renderer.zoomAt(event.clientX, event.clientY, Math.exp(exponent));
  renderPreview();
  event.preventDefault();
}, { passive: false });

window.addEventListener('keydown', (event) => {
  if (event.code === 'Space' && !/INPUT|SELECT|BUTTON|TEXTAREA/.test(document.activeElement.tagName)) {
    event.preventDefault();
    setPlaying(!state.playing);
  }
});

if (renderer) {
  const resizeObserver = new ResizeObserver(() => { if (!state.exporting) renderPreview(); });
  resizeObserver.observe(elements.stageWrap);
  renderPreview();
}
syncExportButtons();
detectBackends();
