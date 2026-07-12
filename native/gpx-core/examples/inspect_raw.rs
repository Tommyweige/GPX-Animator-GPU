use gpx_core::{ParseOptions, parse_gpx};
fn main() {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .expect("usage: inspect_raw <track.gpx> [elevation-threshold]");
    let threshold = args
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse().ok()))
        .unwrap_or(2.5);
    let source = std::fs::read_to_string(path).expect("read GPX");
    let options = ParseOptions {
        stationary_min_ms: i64::MAX,
        elevation_threshold_m: threshold,
        ..ParseOptions::default()
    };
    let track = parse_gpx(&source, options).expect("parse GPX");
    println!(
        "points={} distance_m={:.3} elevation_gain_m={:.3}",
        track.points.len(),
        track.distance_m,
        track.elevation_gain_m
    );
}
