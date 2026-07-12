use gpx_core::{ParseOptions, parse_gpx};

#[test]
fn real_0706_route_matches_golden_statistics() {
    let source = include_str!("fixtures/0706-route.gpx");
    let track = parse_gpx(source, ParseOptions::default()).unwrap();
    assert_eq!(track.source_point_count, 4161);
    assert_eq!(track.points.len(), 4161);
    assert!((track.distance_m - 61_270.092).abs() < 0.5);
    assert!(
        (track.elevation_gain_m - 794.0).abs() < 2.0,
        "gain was {}",
        track.elevation_gain_m
    );
    assert!(track.removed_stop_points > 0);
}
