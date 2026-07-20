use gpx_core::{ParseOptions, parse_gpx};

#[test]
fn parses_gpx_animator_ride_mobile_export() {
    let source = include_str!("fixtures/mobile-ride.gpx");
    let track = parse_gpx(source, ParseOptions::default()).unwrap();

    assert_eq!(track.name, "Taipei Night Ride");
    assert_eq!(track.source_point_count, 4);
    assert_eq!(track.points.len(), 4);
    assert_eq!(track.points[0].timestamp_ms, Some(1_784_289_600_000));
    assert_eq!(track.points[3].timestamp_ms, Some(1_784_289_603_000));
    assert_eq!(track.points[0].elevation_m, Some(12.5));
    assert_eq!(track.points[3].elevation_m, Some(14.1));
    assert!((95.0..100.0).contains(&track.distance_m));
}
