export const EARTH_RADIUS_M = 6_371_008.8;
const MAX_LAT = 85.05112878;

export function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

export function lerp(a, b, t) {
  return a + (b - a) * t;
}

export function lonLatToWorld(lon, lat) {
  const safeLat = clamp(lat, -MAX_LAT, MAX_LAT);
  const sin = Math.sin((safeLat * Math.PI) / 180);
  return {
    x: (lon + 180) / 360,
    y: 0.5 - Math.log((1 + sin) / (1 - sin)) / (4 * Math.PI),
  };
}

export function worldToLonLat(x, y) {
  const lon = x * 360 - 180;
  const n = Math.PI - 2 * Math.PI * y;
  const lat = (180 / Math.PI) * Math.atan(Math.sinh(n));
  return { lon, lat };
}

export function haversineMeters(a, b) {
  const lat1 = (a.lat * Math.PI) / 180;
  const lat2 = (b.lat * Math.PI) / 180;
  const deltaLat = lat2 - lat1;
  const deltaLon = ((b.lon - a.lon) * Math.PI) / 180;
  const h = Math.sin(deltaLat / 2) ** 2
    + Math.cos(lat1) * Math.cos(lat2) * Math.sin(deltaLon / 2) ** 2;
  return 2 * EARTH_RADIUS_M * Math.asin(Math.sqrt(h));
}

export function computeBounds(points) {
  if (!points.length) return null;
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const point of points) {
    const world = point.world ?? lonLatToWorld(point.lon, point.lat);
    minX = Math.min(minX, world.x);
    minY = Math.min(minY, world.y);
    maxX = Math.max(maxX, world.x);
    maxY = Math.max(maxY, world.y);
  }
  if (maxX - minX < 1e-7) { minX -= 5e-7; maxX += 5e-7; }
  if (maxY - minY < 1e-7) { minY -= 5e-7; maxY += 5e-7; }
  return { minX, minY, maxX, maxY, center: { x: (minX + maxX) / 2, y: (minY + maxY) / 2 } };
}

export function fitCamera(bounds, width, height, padding = 0.14) {
  if (!bounds) return { center: { x: 0.5, y: 0.5 }, scale: 256 };
  const availableWidth = Math.max(1, width * (1 - padding * 2));
  const availableHeight = Math.max(1, height * (1 - padding * 2));
  const scale = Math.min(
    availableWidth / Math.max(bounds.maxX - bounds.minX, 1e-9),
    availableHeight / Math.max(bounds.maxY - bounds.minY, 1e-9),
  );
  return { center: { ...bounds.center }, scale: clamp(scale, 256, 256 * 2 ** 18) };
}

export function sampleTrack(track, progress) {
  const points = track?.points ?? [];
  if (!points.length) return null;
  if (points.length === 1) return { ...points[0], index: 0, fraction: 0 };
  const normalizedProgress = clamp(progress, 0, 1);
  const targetDistance = normalizedProgress * track.distanceM;

  let low = 0;
  let high = points.length - 1;
  while (low < high) {
    const mid = Math.floor((low + high) / 2);
    if (points[mid].distanceM < targetDistance) low = mid + 1;
    else high = mid;
  }
  const nextIndex = clamp(low, 1, points.length - 1);
  const index = nextIndex - 1;
  const a = points[index];
  const b = points[nextIndex];
  const span = Math.max(0.001, b.distanceM - a.distanceM);
  const fraction = clamp((targetDistance - a.distanceM) / span, 0, 1);

  return {
    lat: lerp(a.lat, b.lat, fraction),
    lon: lerp(a.lon, b.lon, fraction),
    ele: Number.isFinite(a.ele) && Number.isFinite(b.ele) ? lerp(a.ele, b.ele, fraction) : (a.ele ?? b.ele),
    speedKmh: null,
    distanceM: lerp(a.distanceM, b.distanceM, fraction),
    elapsedMs: lerp(a.sourceElapsedMs, b.sourceElapsedMs, fraction),
    world: {
      x: lerp(a.world.x, b.world.x, fraction),
      y: lerp(a.world.y, b.world.y, fraction),
    },
    index,
    fraction,
    progress: normalizedProgress,
  };
}

export function hexToRgba(hex, alpha = 1) {
  const value = String(hex).replace('#', '');
  const expanded = value.length === 3 ? value.split('').map((part) => part + part).join('') : value;
  const integer = Number.parseInt(expanded, 16);
  if (!Number.isFinite(integer)) return [1, 1, 1, alpha];
  return [
    ((integer >> 16) & 255) / 255,
    ((integer >> 8) & 255) / 255,
    (integer & 255) / 255,
    alpha,
  ];
}

export function formatDuration(milliseconds, tenths = false) {
  const totalSeconds = Math.max(0, milliseconds) / 1000;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = Math.floor(totalSeconds % 60);
  const decimal = tenths ? `.${Math.floor((totalSeconds % 1) * 10)}` : '';
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}${decimal}`;
}

export function formatDistance(meters) {
  if (meters < 1000) return `${Math.round(meters)} m`;
  return `${(meters / 1000).toFixed(meters >= 100_000 ? 0 : 2)} km`;
}
