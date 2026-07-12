import { clamp, sampleTrack } from './geo.js';

function roundedRect(ctx, x, y, width, height, radius) {
  const r = Math.min(radius, width / 2, height / 2);
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + width, y, x + width, y + height, r);
  ctx.arcTo(x + width, y + height, x, y + height, r);
  ctx.arcTo(x, y + height, x, y, r);
  ctx.arcTo(x, y, x + width, y, r);
  ctx.closePath();
}

export function drawElevationOverlay(ctx, track, progress, {
  color = '#ff5d3b',
  width = ctx.canvas.width,
  height = ctx.canvas.height,
  preview = false,
} = {}) {
  if (!track?.points?.length) return;
  const elevations = track.points.map((point) => point.ele).filter(Number.isFinite);
  if (elevations.length < 2) return;

  const scale = Math.max(0.65, width / 1920);
  const left = width * 0.055;
  const right = width * 0.055;
  const bottom = height * 0.055;
  const graphHeight = height * (preview ? 0.17 : 0.18);
  const graphWidth = width - left - right;
  const top = height - bottom - graphHeight;
  const min = Math.min(...elevations);
  const max = Math.max(...elevations);
  const span = Math.max(10, max - min);

  ctx.save();
  const gradient = ctx.createLinearGradient(0, top, 0, height - bottom);
  gradient.addColorStop(0, 'rgba(5, 8, 11, 0)');
  gradient.addColorStop(1, 'rgba(5, 8, 11, 0.58)');
  ctx.fillStyle = gradient;
  ctx.fillRect(0, top - graphHeight * .2, width, graphHeight * 1.35);

  const pointAt = (point) => ({
    x: left + (point.distanceM / Math.max(1, track.distanceM)) * graphWidth,
    y: top + (1 - ((Number.isFinite(point.ele) ? point.ele : min) - min) / span) * graphHeight,
  });

  ctx.beginPath();
  track.points.forEach((point, index) => {
    const p = pointAt(point);
    if (index === 0) ctx.moveTo(p.x, p.y);
    else ctx.lineTo(p.x, p.y);
  });
  ctx.strokeStyle = 'rgba(232, 239, 243, .25)';
  ctx.lineWidth = Math.max(1, 2 * scale);
  ctx.stroke();

  const sample = sampleTrack(track, progress);
  const targetDistance = sample?.distanceM ?? 0;
  ctx.beginPath();
  let started = false;
  for (const point of track.points) {
    if (point.distanceM > targetDistance) break;
    const p = pointAt(point);
    if (!started) { ctx.moveTo(p.x, p.y); started = true; }
    else ctx.lineTo(p.x, p.y);
  }
  ctx.strokeStyle = color;
  ctx.lineWidth = Math.max(2, 3 * scale);
  ctx.shadowColor = color;
  ctx.shadowBlur = 9 * scale;
  ctx.stroke();

  const active = sample ? pointAt(sample) : pointAt(track.points[0]);
  ctx.shadowBlur = 12 * scale;
  ctx.fillStyle = '#fff';
  ctx.beginPath();
  ctx.arc(active.x, active.y, Math.max(2.5, 4 * scale), 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

export function drawHudOverlay(ctx, sample, {
  width = ctx.canvas.width,
  height = ctx.canvas.height,
} = {}) {
  if (!sample) return;
  const scale = Math.max(0.58, width / 1920);
  const x = 44 * scale;
  const y = 42 * scale;
  const boxWidth = 475 * scale;
  const boxHeight = 108 * scale;
  const radius = 14 * scale;

  ctx.save();
  roundedRect(ctx, x, y, boxWidth, boxHeight, radius);
  ctx.fillStyle = 'rgba(6, 10, 14, .76)';
  ctx.fill();
  ctx.strokeStyle = 'rgba(255, 255, 255, .15)';
  ctx.lineWidth = Math.max(1, scale);
  ctx.stroke();

  ctx.textBaseline = 'top';
  ctx.fillStyle = '#8b99a5';
  ctx.font = `700 ${13 * scale}px Inter, "Microsoft JhengHei", sans-serif`;
  ctx.fillText('已完成', x + 25 * scale, y + 20 * scale);
  ctx.fillText('海拔', x + 180 * scale, y + 20 * scale);
  ctx.fillText('路線進度', x + 305 * scale, y + 20 * scale);

  ctx.fillStyle = '#f4f7f9';
  ctx.font = `750 ${22 * scale}px ui-monospace, Consolas, monospace`;
  ctx.fillText(`${(sample.distanceM / 1000).toFixed(2)} km`, x + 25 * scale, y + 55 * scale);
  ctx.fillText(Number.isFinite(sample.ele) ? `${Math.round(sample.ele)} m` : '— m', x + 180 * scale, y + 55 * scale);
  ctx.fillText(`${Math.round(sample.progress * 100)}%`, x + 305 * scale, y + 55 * scale);
  ctx.restore();
}

export function drawMapCredit(ctx, { width = ctx.canvas.width, height = ctx.canvas.height } = {}) {
  const scale = Math.max(.6, width / 1920);
  ctx.save();
  ctx.fillStyle = 'rgba(255,255,255,.72)';
  ctx.textAlign = 'right';
  ctx.textBaseline = 'bottom';
  ctx.font = `${10 * scale}px Inter, sans-serif`;
  ctx.shadowColor = '#000';
  ctx.shadowBlur = 3 * scale;
  ctx.fillText('© OpenStreetMap contributors', width - 12 * scale, height - 9 * scale);
  ctx.restore();
}
