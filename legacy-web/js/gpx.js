import { haversineMeters, lonLatToWorld } from './geo.js';

function decodeXml(value = '') {
  return value
    .replace(/&lt;/g, '<')
    .replace(/&gt;/g, '>')
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&amp;/g, '&')
    .trim();
}

function firstTag(xml, tagName) {
  const qualified = `(?:[\\w.-]+:)?${tagName}`;
  const match = xml.match(new RegExp(`<${qualified}(?:\\s[^>]*)?>([\\s\\S]*?)<\\/${qualified}\\s*>`, 'i'));
  return match ? decodeXml(match[1].replace(/<[^>]*>/g, '')) : null;
}

function attribute(source, name) {
  const match = source.match(new RegExp(`(?:^|\\s)${name}\\s*=\\s*(["'])(.*?)\\1`, 'i'));
  return match ? match[2] : null;
}

function parsePointBlocks(xml, pointTag, segmentIndex) {
  const expression = new RegExp(`<${pointTag}\\b([^>]*)>([\\s\\S]*?)<\\/${pointTag}\\s*>`, 'gi');
  const points = [];
  let match;
  while ((match = expression.exec(xml))) {
    const lat = Number.parseFloat(attribute(match[1], 'lat'));
    const lon = Number.parseFloat(attribute(match[1], 'lon'));
    if (!Number.isFinite(lat) || !Number.isFinite(lon) || Math.abs(lat) > 90 || Math.abs(lon) > 180) continue;
    const eleText = firstTag(match[2], 'ele');
    const timeText = firstTag(match[2], 'time');
    const speedText = firstTag(match[2], 'speed');
    const epoch = timeText ? Date.parse(timeText) : Number.NaN;
    const speedMps = speedText === null ? Number.NaN : Number.parseFloat(speedText);
    points.push({
      lat,
      lon,
      ele: eleText === null ? null : Number.parseFloat(eleText),
      epochMs: Number.isFinite(epoch) ? epoch : null,
      recordedSpeedKmh: Number.isFinite(speedMps) && speedMps >= 0 ? speedMps * 3.6 : null,
      sourceSegmentIndex: segmentIndex,
      segmentIndex,
    });
  }
  return points;
}

function calculateElevationStats(points, smoothingDistanceM = 30, changeThresholdM = 2.5) {
  let elevationGainM = 0;
  let elevationLossM = 0;

  for (let index = 0; index < points.length;) {
    const segmentIndex = points[index].segmentIndex;
    let segmentEnd = index;
    while (segmentEnd + 1 < points.length && points[segmentEnd + 1].segmentIndex === segmentIndex) segmentEnd += 1;

    const firstValid = points.slice(index, segmentEnd + 1).findIndex((point) => Number.isFinite(point.ele));
    if (firstValid >= 0) {
      const firstIndex = index + firstValid;
      let filtered = points[firstIndex].ele;
      const filteredValues = [filtered];
      points[firstIndex].filteredEle = filtered;

      for (let cursor = firstIndex + 1; cursor <= segmentEnd; cursor += 1) {
        const point = points[cursor];
        if (!Number.isFinite(point.ele)) {
          point.filteredEle = filtered;
          filteredValues.push(filtered);
          continue;
        }
        const deltaDistance = Math.max(0.5, point.distanceM - points[cursor - 1].distanceM);
        const alpha = 1 - Math.exp(-deltaDistance / smoothingDistanceM);
        filtered += alpha * (point.ele - filtered);
        point.filteredEle = filtered;
        filteredValues.push(filtered);
      }

      let direction = 0;
      let pivot = filteredValues[0];
      let extreme = pivot;
      let low = pivot;
      let high = pivot;
      let lowIndex = 0;
      let highIndex = 0;
      for (let cursor = 1; cursor < filteredValues.length; cursor += 1) {
        const value = filteredValues[cursor];
        if (direction === 0) {
          if (value < low) { low = value; lowIndex = cursor; }
          if (value > high) { high = value; highIndex = cursor; }
          if (high - low >= changeThresholdM) {
            if (lowIndex < highIndex) {
              direction = 1;
              pivot = low;
              extreme = high;
            } else {
              direction = -1;
              pivot = high;
              extreme = low;
            }
          }
        } else if (direction > 0) {
          if (value > extreme) extreme = value;
          else if (extreme - value >= changeThresholdM) {
            elevationGainM += Math.max(0, extreme - pivot);
            direction = -1;
            pivot = extreme;
            extreme = value;
          }
        } else if (value < extreme) {
          extreme = value;
        } else if (value - extreme >= changeThresholdM) {
          elevationLossM += Math.max(0, pivot - extreme);
          direction = 1;
          pivot = extreme;
          extreme = value;
        }
      }
      if (direction > 0) elevationGainM += Math.max(0, extreme - pivot);
      else if (direction < 0) elevationLossM += Math.max(0, pivot - extreme);
    }
    index = segmentEnd + 1;
  }

  return { elevationGainM, elevationLossM };
}

function calculateSpeeds(points) {
  for (let index = 0; index < points.length; index += 1) {
    const point = points[index];
    if (index === 0 || point.segmentIndex !== points[index - 1].segmentIndex) {
      point.speedKmh = 0;
      continue;
    }
    if (Number.isFinite(point.recordedSpeedKmh)) {
      point.speedKmh = Math.min(250, point.recordedSpeedKmh);
      continue;
    }

    const previous = points[index - 1];
    const deltaDistance = point.distanceM - previous.distanceM;
    const deltaHours = (point.sourceElapsedMs - previous.sourceElapsedMs) / 3_600_000;
    point.speedKmh = deltaHours > 0 ? Math.min(250, Math.max(0, deltaDistance / 1000 / deltaHours)) : 0;
  }
}

function recalculateGeometry(points) {
  let distanceM = 0;
  for (let index = 0; index < points.length; index += 1) {
    const point = points[index];
    point.world = lonLatToWorld(point.lon, point.lat);
    if (index > 0 && point.sourceSegmentIndex === points[index - 1].sourceSegmentIndex) {
      distanceM += haversineMeters(points[index - 1], point);
    }
    point.distanceM = distanceM;
  }
  return distanceM;
}

function filterStationaryPoints(points, {
  stopSpeedKmh = 2,
  minStopDurationMs = 6_000,
  maxStopDriftM = 30,
} = {}) {
  const discarded = new Set();
  let skippedStopCount = 0;
  let skippedStopDurationMs = 0;

  for (let index = 1; index < points.length;) {
    const start = index;
    const isStationaryEdge = (cursor) => {
      const previous = points[cursor - 1];
      const point = points[cursor];
      if (point.sourceSegmentIndex !== previous.sourceSegmentIndex) return false;
      const durationMs = point.sourceElapsedMs - previous.sourceElapsedMs;
      if (durationMs <= 0) return false;
      const speedKmh = haversineMeters(previous, point) / 1000 / (durationMs / 3_600_000);
      return speedKmh <= stopSpeedKmh;
    };

    if (!isStationaryEdge(index)) {
      index += 1;
      continue;
    }

    while (index < points.length && isStationaryEdge(index)) index += 1;
    const end = index - 1;
    const anchor = points[start - 1];
    const last = points[end];
    const durationMs = last.sourceElapsedMs - anchor.sourceElapsedMs;
    const driftM = haversineMeters(anchor, last);

    if (durationMs >= minStopDurationMs && driftM <= maxStopDriftM) {
      for (let cursor = start; cursor <= end; cursor += 1) discarded.add(cursor);
      skippedStopCount += 1;
      skippedStopDurationMs += durationMs;
    }
  }

  return {
    points: points.filter((_point, index) => !discarded.has(index)),
    removedStopPointCount: discarded.size,
    skippedStopCount,
    skippedStopDurationMs,
  };
}

function buildAnimationTimeline(points, { pauseThresholdMs, synthesizeTime }) {
  let animationElapsedMs = 0;
  let visualSegmentIndex = 0;
  let pauseCount = 0;
  let pausedDurationMs = 0;
  points[0].animationElapsedMs = 0;
  points[0].segmentIndex = visualSegmentIndex;

  for (let index = 1; index < points.length; index += 1) {
    const previous = points[index - 1];
    const point = points[index];
    const gapMs = Math.max(0, point.sourceElapsedMs - previous.sourceElapsedMs);
    const originalBreak = point.sourceSegmentIndex !== previous.sourceSegmentIndex;
    const pauseBreak = !synthesizeTime && !originalBreak && gapMs > pauseThresholdMs;

    if (pauseBreak) {
      pauseCount += 1;
      pausedDurationMs += gapMs;
      point.pauseBefore = true;
    }

    if (originalBreak) {
      visualSegmentIndex += 1;
      animationElapsedMs += 1;
    } else if (!pauseBreak) {
      animationElapsedMs += gapMs;
    }
    point.segmentIndex = visualSegmentIndex;
    point.animationElapsedMs = animationElapsedMs;
  }

  return { pauseCount, pausedDurationMs };
}

function estimateStepMs(distanceMeters, defaultSpeedKmh) {
  const metersPerSecond = Math.max(0.3, defaultSpeedKmh / 3.6);
  return Math.max(250, (distanceMeters / metersPerSecond) * 1000);
}

function fillTimeline(points, defaultSpeedKmh) {
  const known = [];
  points.forEach((point, index) => {
    if (Number.isFinite(point.epochMs)) known.push(index);
  });

  if (!known.length) {
    points[0].epochMs = Date.UTC(2025, 0, 1);
    for (let index = 1; index < points.length; index += 1) {
      const delta = points[index].distanceM - points[index - 1].distanceM;
      points[index].epochMs = points[index - 1].epochMs + estimateStepMs(delta, defaultSpeedKmh);
    }
    return true;
  }

  const firstKnown = known[0];
  for (let index = firstKnown - 1; index >= 0; index -= 1) {
    const delta = points[index + 1].distanceM - points[index].distanceM;
    points[index].epochMs = points[index + 1].epochMs - estimateStepMs(delta, defaultSpeedKmh);
  }

  for (let knownIndex = 0; knownIndex < known.length - 1; knownIndex += 1) {
    const startIndex = known[knownIndex];
    const endIndex = known[knownIndex + 1];
    if (endIndex === startIndex + 1) continue;
    const start = points[startIndex];
    const end = points[endIndex];
    const distanceSpan = end.distanceM - start.distanceM;
    for (let index = startIndex + 1; index < endIndex; index += 1) {
      const fraction = distanceSpan > 0
        ? (points[index].distanceM - start.distanceM) / distanceSpan
        : (index - startIndex) / (endIndex - startIndex);
      points[index].epochMs = start.epochMs + (end.epochMs - start.epochMs) * fraction;
    }
  }

  const lastKnown = known.at(-1);
  for (let index = lastKnown + 1; index < points.length; index += 1) {
    const delta = points[index].distanceM - points[index - 1].distanceM;
    points[index].epochMs = points[index - 1].epochMs + estimateStepMs(delta, defaultSpeedKmh);
  }

  return known.length !== points.length;
}

export function parseGpx(source, { defaultSpeedKmh = 12, pauseThresholdMs = 60_000 } = {}) {
  if (typeof source !== 'string' || !/<(?:gpx|trk|rte)\b/i.test(source)) {
    throw new Error('這不是有效的 GPX 內容。');
  }

  const segmentExpression = /<trkseg\b[^>]*>([\s\S]*?)<\/trkseg\s*>/gi;
  const segments = [];
  let segmentMatch;
  while ((segmentMatch = segmentExpression.exec(source))) {
    const points = parsePointBlocks(segmentMatch[1], 'trkpt', segments.length);
    if (points.length) segments.push(points);
  }

  if (!segments.length) {
    const trackPoints = parsePointBlocks(source, 'trkpt', 0);
    if (trackPoints.length) segments.push(trackPoints);
  }
  if (!segments.length) {
    const routePoints = parsePointBlocks(source, 'rtept', 0);
    if (routePoints.length) segments.push(routePoints);
  }

  let points = segments.flat();
  if (points.length < 2) throw new Error('GPX 至少需要兩個有效的軌跡點。');

  recalculateGeometry(points);

  const synthesizedTime = fillTimeline(points, defaultSpeedKmh);
  const origin = points[0].epochMs;
  for (let index = 0; index < points.length; index += 1) {
    if (index > 0 && points[index].epochMs <= points[index - 1].epochMs) {
      points[index].epochMs = points[index - 1].epochMs + 250;
    }
    points[index].sourceElapsedMs = points[index].epochMs - origin;
    points[index].elapsedMs = points[index].sourceElapsedMs;
  }
  const sourcePointCount = points.length;
  const sourceDurationMs = Math.max(1, points.at(-1).sourceElapsedMs);
  const filtered = filterStationaryPoints(points);
  points = filtered.points;
  if (points.length < 2) throw new Error('移除停留點後沒有足夠的軌跡資料。');

  const distanceM = recalculateGeometry(points);
  const { elevationGainM, elevationLossM } = calculateElevationStats(points);
  const { pauseCount, pausedDurationMs } = buildAnimationTimeline(points, { pauseThresholdMs, synthesizeTime: synthesizedTime });
  calculateSpeeds(points);

  const segmentRanges = [];
  let start = 0;
  for (let index = 1; index <= points.length; index += 1) {
    if (index === points.length || points[index].segmentIndex !== points[index - 1].segmentIndex) {
      segmentRanges.push({ start, end: index - 1 });
      start = index;
    }
  }

  const metadata = source.match(/<metadata\b[^>]*>([\s\S]*?)<\/metadata\s*>/i)?.[1] ?? '';
  const trackBlock = source.match(/<trk\b[^>]*>([\s\S]*?)<\/trk\s*>/i)?.[1] ?? '';
  const name = firstTag(trackBlock, 'name') || firstTag(metadata, 'name') || firstTag(source, 'name') || '未命名軌跡';

  return {
    name,
    points,
    segments: segmentRanges,
    distanceM,
    elevationGainM,
    elevationLossM,
    durationMs: Math.max(1, points.at(-1).animationElapsedMs),
    sourceDurationMs,
    synthesizedTime,
    pauseCount,
    pausedDurationMs,
    sourcePointCount,
    removedStopPointCount: filtered.removedStopPointCount,
    skippedStopCount: filtered.skippedStopCount,
    skippedStopDurationMs: filtered.skippedStopDurationMs,
    playbackMode: 'uniform-distance',
  };
}
