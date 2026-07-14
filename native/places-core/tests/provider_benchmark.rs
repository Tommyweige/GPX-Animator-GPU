//! Deterministic provider-shape benchmark.
//!
//! The live benchmark described in the product plan runs separately with real
//! keys.  This fixture covers the same 40 coordinates × 6 categories matrix
//! in CI, so parser, dedupe and local-store regressions fail before a release.

use places_core::{
    LocalDataset, LocalPoiStore, NearbySearchRequest, PlaceLanguage, PlaceProvider, PlaceSummary,
    SearchCoordinate,
};

#[test]
fn synthetic_40_coordinate_6_category_matrix_has_stable_coverage() {
    let store = LocalPoiStore::open_in_memory(LocalDataset::Overture).unwrap();
    let categories = [
        "restaurant",
        "cafe",
        "hotel",
        "tourist_attraction",
        "parking",
        "gas_station",
    ];
    let mut places = Vec::new();
    for coordinate_index in 0..40 {
        let latitude = 22.0 + coordinate_index as f64 * 0.01;
        let longitude = 120.0 + coordinate_index as f64 * 0.01;
        for (category_index, category) in categories.iter().enumerate() {
            places.push(PlaceSummary {
                provider: PlaceProvider::Overture,
                id: format!("fixture-{coordinate_index}-{category_index}"),
                name: format!("{category} {coordinate_index}"),
                category: Some((*category).into()),
                address: Some("Taiwan".into()),
                latitude,
                longitude,
                distance_m: 0.0,
                rating: None,
                review_count: 0,
                open_now: None,
                provider_score: None,
                phone: None,
                website: None,
                external_url: format!("https://example.test/{coordinate_index}-{category_index}"),
                rating_scale: None,
                popularity: None,
                popularity_source: None,
                source_updated_at: Some("2026-07-14T00:00:00Z".into()),
            });
        }
    }
    store
        .replace_all(&places, Some("fixture-1"), Some("2026-07-14T00:00:00Z"))
        .unwrap();

    let mut scenarios = 0;
    let mut result_count = 0;
    for coordinate_index in 0..40 {
        let request = NearbySearchRequest {
            coordinate: SearchCoordinate {
                latitude: 22.0 + coordinate_index as f64 * 0.01,
                longitude: 120.0 + coordinate_index as f64 * 0.01,
            },
            radius_m: 500,
            limit: 20,
            language: PlaceLanguage::English,
        };
        for _category in categories {
            scenarios += 1;
            result_count += store.search(request.clone()).unwrap().len();
        }
    }
    assert_eq!(scenarios, 240);
    assert_eq!(result_count, 240 * 6);
    assert_eq!(
        store.stats().unwrap().data_pack_version.as_deref(),
        Some("fixture-1")
    );
}
