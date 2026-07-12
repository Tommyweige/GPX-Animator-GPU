use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub const EARTH_RADIUS_M: f64 = 6_371_008.8;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub lat: f64,
    pub lon: f64,
    pub elevation_m: Option<f64>,
    pub timestamp_ms: Option<i64>,
    pub distance_m: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    pub points: Vec<Point>,
    pub source_point_count: usize,
    pub removed_stop_points: usize,
    pub distance_m: f64,
    pub elevation_gain_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParseOptions {
    pub stationary_speed_kmh: f64,
    pub stationary_min_ms: i64,
    pub stationary_drift_m: f64,
    pub elevation_threshold_m: f64,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            stationary_speed_kmh: 2.0,
            stationary_min_ms: 6_000,
            stationary_drift_m: 30.0,
            elevation_threshold_m: 2.5,
        }
    }
}

#[derive(Debug, Error, PartialEq)]
pub enum GpxError {
    #[error("GPX 至少需要兩個有效軌跡點")]
    TooFewPoints,
    #[error("GPX XML 無法解析：{0}")]
    Xml(String),
}

pub fn haversine_m(a: &Point, b: &Point) -> f64 {
    let dlat = (b.lat - a.lat).to_radians();
    let dlon = (b.lon - a.lon).to_radians();
    let lat1 = a.lat.to_radians();
    let lat2 = b.lat.to_radians();
    let h = (dlat / 2.0).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * EARTH_RADIUS_M * h.sqrt().asin()
}

fn parse_time_ms(value: &str) -> Option<i64> {
    value.trim().parse().ok().or_else(|| {
        OffsetDateTime::parse(value.trim(), &Rfc3339)
            .ok()
            .map(|value| {
                let nanos = value.unix_timestamp_nanos();
                (nanos / 1_000_000) as i64
            })
    })
}

pub fn parse_gpx(source: &str, options: ParseOptions) -> Result<Track, GpxError> {
    let mut reader = Reader::from_str(source);
    reader.config_mut().trim_text(true);
    let mut name = String::from("未命名軌跡");
    let mut points = Vec::new();
    let mut current: Option<Point> = None;
    let mut tag = Vec::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => {
                tag.clear();
                tag.extend_from_slice(event.local_name().as_ref());
                if matches!(event.local_name().as_ref(), b"trkpt" | b"rtept") {
                    let mut lat = None;
                    let mut lon = None;
                    for attribute in event.attributes().flatten() {
                        match attribute.key.local_name().as_ref() {
                            b"lat" => {
                                lat = std::str::from_utf8(&attribute.value)
                                    .ok()
                                    .and_then(|v| v.parse().ok())
                            }
                            b"lon" => {
                                lon = std::str::from_utf8(&attribute.value)
                                    .ok()
                                    .and_then(|v| v.parse().ok())
                            }
                            _ => {}
                        }
                    }
                    if let (Some(lat), Some(lon)) = (lat, lon)
                        && (-90.0..=90.0).contains(&lat)
                        && (-180.0..=180.0).contains(&lon)
                    {
                        current = Some(Point {
                            lat,
                            lon,
                            elevation_m: None,
                            timestamp_ms: None,
                            distance_m: 0.0,
                        });
                    }
                }
            }
            Ok(Event::Empty(event))
                if matches!(event.local_name().as_ref(), b"trkpt" | b"rtept") =>
            {
                let mut lat = None;
                let mut lon = None;
                for attribute in event.attributes().flatten() {
                    match attribute.key.local_name().as_ref() {
                        b"lat" => {
                            lat = std::str::from_utf8(&attribute.value)
                                .ok()
                                .and_then(|value| value.parse().ok())
                        }
                        b"lon" => {
                            lon = std::str::from_utf8(&attribute.value)
                                .ok()
                                .and_then(|value| value.parse().ok())
                        }
                        _ => {}
                    }
                }
                if let (Some(lat), Some(lon)) = (lat, lon)
                    && (-90.0..=90.0).contains(&lat)
                    && (-180.0..=180.0).contains(&lon)
                {
                    points.push(Point {
                        lat,
                        lon,
                        elevation_m: None,
                        timestamp_ms: None,
                        distance_m: 0.0,
                    });
                }
            }
            Ok(Event::Text(text)) => {
                let value = text
                    .decode()
                    .map_err(|error| GpxError::Xml(error.to_string()))?;
                match tag.as_slice() {
                    b"name" if current.is_none() && name == "未命名軌跡" => {
                        name = value.into_owned()
                    }
                    b"ele" => {
                        if let Some(point) = current.as_mut() {
                            point.elevation_m = value.parse().ok();
                        }
                    }
                    b"time" => {
                        if let Some(point) = current.as_mut() {
                            point.timestamp_ms = parse_time_ms(&value);
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(event)) => {
                if matches!(event.local_name().as_ref(), b"trkpt" | b"rtept")
                    && let Some(point) = current.take()
                {
                    points.push(point);
                }
                tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(error) => return Err(GpxError::Xml(error.to_string())),
            _ => {}
        }
    }
    if points.len() < 2 {
        return Err(GpxError::TooFewPoints);
    }
    let source_point_count = points.len();
    let removed_stop_points = source_point_count - filter_stationary(points.clone(), options).len();
    let mut distance_m = 0.0;
    for index in 1..points.len() {
        distance_m += haversine_m(&points[index - 1], &points[index]);
        points[index].distance_m = distance_m;
    }
    let elevation_gain_m = elevation_gain(&points, options.elevation_threshold_m);
    Ok(Track {
        name,
        points,
        source_point_count,
        removed_stop_points,
        distance_m,
        elevation_gain_m,
    })
}

fn filter_stationary(points: Vec<Point>, options: ParseOptions) -> Vec<Point> {
    let mut kept = Vec::with_capacity(points.len());
    for point in points {
        let remove = kept.last().is_some_and(|previous: &Point| {
            let (Some(a), Some(b)) = (previous.timestamp_ms, point.timestamp_ms) else {
                return false;
            };
            let elapsed = b - a;
            if elapsed < options.stationary_min_ms {
                return false;
            }
            let distance = haversine_m(previous, &point);
            let speed = if elapsed > 0 {
                distance / elapsed as f64 * 3_600.0
            } else {
                f64::INFINITY
            };
            distance <= options.stationary_drift_m && speed <= options.stationary_speed_kmh
        });
        if !remove {
            kept.push(point);
        }
    }
    kept
}

fn elevation_gain(points: &[Point], threshold: f64) -> f64 {
    let Some(first_index) = points.iter().position(|point| point.elevation_m.is_some()) else {
        return 0.0;
    };
    let mut filtered = points[first_index].elevation_m.unwrap();
    let mut values = vec![filtered];
    for index in first_index + 1..points.len() {
        if let Some(elevation) = points[index].elevation_m {
            let delta_distance = (points[index].distance_m - points[index - 1].distance_m).max(0.5);
            let alpha = 1.0 - (-delta_distance / 30.0).exp();
            filtered += alpha * (elevation - filtered);
        }
        values.push(filtered);
    }
    let mut gain = 0.0;
    let mut direction = 0i8;
    let mut pivot = values[0];
    let mut extreme = pivot;
    let mut low = pivot;
    let mut high = pivot;
    let mut low_index = 0usize;
    let mut high_index = 0usize;
    for (index, value) in values.iter().copied().enumerate().skip(1) {
        if direction == 0 {
            if value < low {
                low = value;
                low_index = index
            }
            if value > high {
                high = value;
                high_index = index
            }
            if high - low >= threshold {
                if low_index < high_index {
                    direction = 1;
                    pivot = low;
                    extreme = high
                } else {
                    direction = -1;
                    pivot = high;
                    extreme = low
                }
            }
        } else if direction > 0 {
            if value > extreme {
                extreme = value
            } else if extreme - value >= threshold {
                gain += (extreme - pivot).max(0.0);
                direction = -1;
                pivot = extreme;
                extreme = value
            }
        } else if value < extreme {
            extreme = value
        } else if value - extreme >= threshold {
            direction = 1;
            pivot = extreme;
            extreme = value
        }
    }
    if direction > 0 {
        gain += (extreme - pivot).max(0.0)
    }
    gain
}

pub fn sample_distance(track: &Track, progress: f64) -> Point {
    let target = progress.clamp(0.0, 1.0) * track.distance_m;
    let upper = track
        .points
        .partition_point(|point| point.distance_m < target)
        .min(track.points.len() - 1);
    if upper == 0 {
        return track.points[0];
    }
    let a = track.points[upper - 1];
    let b = track.points[upper];
    let span = (b.distance_m - a.distance_m).max(f64::EPSILON);
    let t = (target - a.distance_m) / span;
    Point {
        lat: a.lat + (b.lat - a.lat) * t,
        lon: a.lon + (b.lon - a.lon) * t,
        elevation_m: match (a.elevation_m, b.elevation_m) {
            (Some(x), Some(y)) => Some(x + (y - x) * t),
            _ => a.elevation_m.or(b.elevation_m),
        },
        timestamp_ms: None,
        distance_m: target,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(lat: f64, lon: f64, elevation_m: Option<f64>, timestamp_ms: Option<i64>) -> Point {
        Point {
            lat,
            lon,
            elevation_m,
            timestamp_ms,
            distance_m: 0.0,
        }
    }

    #[test]
    fn parses_track_and_route_points() {
        let track = parse_gpx(r#"<gpx><trk><name>Ride</name><trkseg><trkpt lat="25" lon="121"><ele>10</ele></trkpt><trkpt lat="25.001" lon="121"><ele>14</ele></trkpt></trkseg></trk></gpx>"#, ParseOptions::default()).unwrap();
        assert_eq!(track.name, "Ride");
        assert_eq!(track.points.len(), 2);
        assert!((110.0..112.0).contains(&track.distance_m));
        assert!((track.elevation_gain_m - 3.90).abs() < 0.02);
    }

    #[test]
    fn rejects_invalid_or_short_tracks() {
        assert_eq!(
            parse_gpx("<gpx/>", ParseOptions::default()),
            Err(GpxError::TooFewPoints)
        );
    }

    #[test]
    fn removes_long_stationary_sample() {
        let points = vec![
            point(25.0, 121.0, None, Some(0)),
            point(25.00001, 121.0, None, Some(10_000)),
            point(25.001, 121.0, None, Some(20_000)),
        ];
        let filtered = filter_stationary(points, ParseOptions::default());
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn parses_rfc3339_across_day_boundary() {
        let track = parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"><time>2026-07-10T23:59:59.500Z</time></trkpt><trkpt lat="25.01" lon="121"><time>2026-07-11T00:00:01.000Z</time></trkpt></trkseg></trk></gpx>"#, ParseOptions::default()).unwrap();
        assert_eq!(
            track.points[1].timestamp_ms.unwrap() - track.points[0].timestamp_ms.unwrap(),
            1_500
        );
    }

    #[test]
    fn samples_by_distance_not_time() {
        let mut points = vec![
            point(25.0, 121.0, Some(0.0), Some(0)),
            point(25.0, 121.001, Some(10.0), Some(1)),
            point(25.0, 121.011, Some(20.0), Some(1_000_000)),
        ];
        let mut distance = 0.0;
        for index in 1..points.len() {
            distance += haversine_m(&points[index - 1], &points[index]);
            points[index].distance_m = distance;
        }
        let track = Track {
            name: "x".into(),
            points,
            source_point_count: 3,
            removed_stop_points: 0,
            distance_m: distance,
            elevation_gain_m: 20.0,
        };
        let middle = sample_distance(&track, 0.5);
        assert!((middle.lon - 121.0055).abs() < 0.0001);
    }

    #[test]
    fn ignores_elevation_jitter() {
        let mut points = vec![
            point(0.0, 0.0, Some(100.0), None),
            point(0.0, 0.1, Some(101.0), None),
            point(0.0, 0.2, Some(100.5), None),
            point(0.0, 0.3, Some(104.0), None),
        ];
        let mut distance = 0.0;
        for index in 1..points.len() {
            distance += haversine_m(&points[index - 1], &points[index]);
            points[index].distance_m = distance;
        }
        assert!((elevation_gain(&points, 2.5) - 4.0).abs() < 0.01);
    }
}
