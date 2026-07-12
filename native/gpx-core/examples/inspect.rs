use gpx_core::{ParseOptions, parse_gpx};

fn main() {
    let path = std::env::args_os()
        .nth(1)
        .expect("usage: inspect <track.gpx>");
    let source = std::fs::read_to_string(path).expect("read GPX");
    let track = parse_gpx(&source, ParseOptions::default()).expect("parse GPX");
    println!("name={}", track.name);
    println!("source_points={}", track.source_point_count);
    println!("points={}", track.points.len());
    println!("removed_stop_points={}", track.removed_stop_points);
    println!("distance_m={:.3}", track.distance_m);
    println!("elevation_gain_m={:.3}", track.elevation_gain_m);
}
