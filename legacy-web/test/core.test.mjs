import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import { formatDuration, haversineMeters, lonLatToWorld, sampleTrack } from '../js/geo.js';
import { parseGpx } from '../js/gpx.js';

test('parses the bundled multi-point GPX sample', async () => {
  const path = fileURLToPath(new URL('../samples/taipei-riverside.gpx', import.meta.url));
  const track = parseGpx(await readFile(path, 'utf8'));
  assert.equal(track.name, '台北河濱晨騎');
  assert.equal(track.points.length, 36);
  assert.ok(track.distanceM > 8_000 && track.distanceM < 15_000);
  assert.ok(track.durationMs > 20 * 60 * 1000);
  assert.equal(track.synthesizedTime, false);
  assert.ok(track.points.every((point, index) => index === 0 || point.elapsedMs > track.points[index - 1].elapsedMs));
});

test('synthesizes a monotonic timeline when timestamps are absent', () => {
  const track = parseGpx(`<?xml version="1.0"?><gpx><trk><name>No time</name><trkseg>
    <trkpt lat="25.0" lon="121.0"><ele>5</ele></trkpt>
    <trkpt lat="25.001" lon="121.001"><ele>8</ele></trkpt>
    <trkpt lat="25.002" lon="121.002"><ele>7</ele></trkpt>
  </trkseg></trk></gpx>`);
  assert.equal(track.synthesizedTime, true);
  assert.ok(track.durationMs > 0);
  assert.ok(track.points[2].elapsedMs > track.points[1].elapsedMs);
  assert.ok(track.elevationGainM > 2.5 && track.elevationGainM < 3.1);
});

test('distance and Web Mercator helpers return useful values', () => {
  const distance = haversineMeters({ lat: 25, lon: 121 }, { lat: 25.001, lon: 121 });
  assert.ok(distance > 110 && distance < 112);
  assert.deepEqual(lonLatToWorld(0, 0), { x: 0.5, y: 0.5 });
  assert.equal(formatDuration(65_900, true), '01:05.9');
});

test('samples interpolated track position by normalized route distance', () => {
  const track = parseGpx(`<gpx><trk><trkseg>
    <trkpt lat="25" lon="121"><time>2026-01-01T00:00:00Z</time></trkpt>
    <trkpt lat="25.01" lon="121.01"><time>2026-01-01T00:00:10Z</time><speed>10</speed></trkpt>
  </trkseg></trk></gpx>`);
  const middle = sampleTrack(track, 0.5);
  assert.equal(track.points[0].speedKmh, 0);
  assert.equal(track.points[1].speedKmh, 36);
  assert.ok(Math.abs(middle.lat - 25.005) < 1e-8);
  assert.ok(Math.abs(middle.lon - 121.005) < 1e-8);
  assert.equal(middle.elapsedMs, 5_000);
  assert.equal(middle.speedKmh, null);
});

test('uses route distance instead of uneven GPS timestamps for playback progress', () => {
  const track = parseGpx(`<gpx><trk><trkseg>
    <trkpt lat="25" lon="121"><time>2026-01-01T00:00:00Z</time></trkpt>
    <trkpt lat="25" lon="121.001"><time>2026-01-01T00:00:10Z</time></trkpt>
    <trkpt lat="25" lon="121.011"><time>2026-01-01T01:00:10Z</time></trkpt>
  </trkseg></trk></gpx>`);
  const middle = sampleTrack(track, 0.5);
  assert.ok(Math.abs(middle.lon - 121.0055) < 0.00001);
  assert.equal(middle.speedKmh, null);
});

test('skips stationary records and keeps uniform route motion across a long pause', () => {
  const track = parseGpx(`<gpx><trk><trkseg>
    <trkpt lat="25" lon="121"><ele>10</ele><time>2026-01-01T00:00:00Z</time></trkpt>
    <trkpt lat="25.001" lon="121"><ele>12</ele><time>2026-01-01T00:00:10Z</time></trkpt>
    <trkpt lat="25.00105" lon="121"><ele>37</ele><time>2026-01-01T02:00:10Z</time></trkpt>
    <trkpt lat="25.002" lon="121"><ele>40</ele><time>2026-01-01T02:00:20Z</time></trkpt>
  </trkseg></trk></gpx>`);
  assert.equal(track.pauseCount, 1);
  assert.ok(track.sourceDurationMs > 7_000_000);
  assert.ok(track.durationMs < 20_000);
  assert.equal(track.skippedStopCount, 1);
  assert.equal(track.removedStopPointCount, 1);

  const atFinalPoint = sampleTrack(track, 1);
  assert.equal(atFinalPoint.ele, 40);
  assert.equal(atFinalPoint.speedKmh, null);
});

test('filters small elevation jitter out of the climbing total', () => {
  const points = Array.from({ length: 60 }, (_, index) => `
    <trkpt lat="${25 + index * 0.00001}" lon="121"><ele>${100 + (index % 2)}</ele><time>2026-01-01T00:00:${String(index).padStart(2, '0')}Z</time></trkpt>`).join('');
  const track = parseGpx(`<gpx><trk><trkseg>${points}</trkseg></trk></gpx>`);
  assert.ok(track.elevationGainM < 2.5);
});
