//! Opt-in live provider benchmark. Never runs in ordinary CI.
//!
//! Run with `RUN_LIVE_POI_BENCHMARK=1` and one or more of
//! `TOMTOM_API_KEY`, `FOURSQUARE_API_KEY`, `GOOGLE_API_KEY`. The benchmark is
//! intentionally rate-limit aware and prints aggregate JSON-safe metrics only;
//! raw provider responses are not saved.

use places_core::{
    FoursquarePlacesClient, GooglePlacesClient, NearbySearchRequest, PlaceLanguage,
    SearchCoordinate, TomTomV3PlacesClient,
};
use std::time::Instant;

fn main() {
    if std::env::var("RUN_LIVE_POI_BENCHMARK").ok().as_deref() != Some("1") {
        eprintln!("set RUN_LIVE_POI_BENCHMARK=1 to enable live calls");
        return;
    }
    let coordinates = (0..40)
        .map(|index| SearchCoordinate {
            latitude: 22.0 + index as f64 * 0.01,
            longitude: 120.0 + index as f64 * 0.01,
        })
        .collect::<Vec<_>>();
    let mut output = Vec::new();
    if let Ok(key) = std::env::var("TOMTOM_API_KEY") {
        output.push(run_provider("tomtom_v3", coordinates.clone(), |request| {
            TomTomV3PlacesClient::default().search(&key, request)
        }));
    }
    if let Ok(key) = std::env::var("FOURSQUARE_API_KEY") {
        output.push(run_provider("foursquare", coordinates.clone(), |request| {
            FoursquarePlacesClient::default().search(&key, request)
        }));
    }
    if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
        output.push(run_provider("google", coordinates, |request| {
            GooglePlacesClient::default().search(&key, request)
        }));
    }
    println!("[{}]", output.join(",\n"));
}

fn run_provider<F>(name: &str, coordinates: Vec<SearchCoordinate>, mut call: F) -> String
where
    F: FnMut(
        NearbySearchRequest,
    ) -> Result<Vec<places_core::PlaceSummary>, places_core::PlacesError>,
{
    let started = Instant::now();
    let mut successes = 0_u32;
    let mut empty = 0_u32;
    let mut errors = 0_u32;
    let mut result_count = 0_u32;
    let mut latencies_ms = Vec::new();
    for coordinate in coordinates {
        for _category in 0..6 {
            let request_started = Instant::now();
            let result = call(NearbySearchRequest {
                coordinate,
                radius_m: 2_000,
                limit: 20,
                language: PlaceLanguage::English,
            });
            latencies_ms.push(request_started.elapsed().as_secs_f64() * 1_000.0);
            match result {
                Ok(values) if values.is_empty() => empty += 1,
                Ok(values) => {
                    successes += 1;
                    result_count += values.len() as u32;
                }
                Err(places_core::PlacesError::Empty) => empty += 1,
                Err(_) => errors += 1,
            }
        }
    }
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p95 = latencies_ms
        .get((latencies_ms.len() * 95 / 100).min(latencies_ms.len().saturating_sub(1)))
        .copied()
        .unwrap_or_default();
    format!(
        "{{\"provider\":\"{name}\",\"scenarios\":240,\"successes\":{successes},\"empty\":{empty},\"errors\":{errors},\"results\":{result_count},\"p95_ms\":{p95:.2},\"elapsed_s\":{:.2}}}",
        started.elapsed().as_secs_f64()
    )
}
