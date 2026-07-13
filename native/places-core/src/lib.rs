//! Nearby-place lookup shared by the native desktop UI.
//!
//! The module deliberately keeps provider responses in memory only.  Google
//! Places is the primary provider when a key is configured; Overpass/OpenStreetMap
//! is a free fallback for offline or unconfigured installations.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashSet;
use thiserror::Error;

pub const DEFAULT_RADIUS_M: u32 = 2_000;
pub const ALLOWED_RADII_M: [u32; 4] = [500, 1_000, 2_000, 5_000];
const DEFAULT_GOOGLE_BASE_URL: &str = "https://places.googleapis.com";
const DEFAULT_OVERPASS_BASE_URL: &str = "https://overpass-api.de/api/interpreter";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaceProvider {
    Google,
    OpenStreetMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaceLanguage {
    TraditionalChinese,
    English,
}

impl PlaceLanguage {
    pub const fn code(self) -> &'static str {
        match self {
            Self::TraditionalChinese => "zh-TW",
            Self::English => "en",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SearchCoordinate {
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NearbySearchRequest {
    pub coordinate: SearchCoordinate,
    pub radius_m: u32,
    pub limit: u8,
    pub language: PlaceLanguage,
}

impl NearbySearchRequest {
    pub fn normalized(mut self) -> Self {
        self.coordinate.latitude = self
            .coordinate
            .latitude
            .clamp(-85.051_128_78, 85.051_128_78);
        self.coordinate.longitude = normalize_longitude(self.coordinate.longitude);
        self.radius_m = normalize_radius(self.radius_m);
        self.limit = self.limit.clamp(1, 20);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceSummary {
    pub provider: PlaceProvider,
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub address: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub distance_m: f64,
    pub rating: Option<f32>,
    pub review_count: u32,
    pub open_now: Option<bool>,
    pub external_url: String,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PlacesError {
    #[error("Google Places API key is not configured")]
    MissingApiKey,
    #[error("provider rate limit exceeded")]
    RateLimited,
    #[error("provider rejected the request: {0}")]
    Unauthorized(String),
    #[error("provider request failed: {0}")]
    Http(String),
    #[error("provider response could not be decoded: {0}")]
    Parse(String),
    #[error("no nearby places were returned")]
    Empty,
}

pub fn normalize_radius(radius_m: u32) -> u32 {
    ALLOWED_RADII_M
        .iter()
        .copied()
        .min_by_key(|candidate| candidate.abs_diff(radius_m))
        .unwrap_or(DEFAULT_RADIUS_M)
}

pub fn normalize_longitude(longitude: f64) -> f64 {
    if !longitude.is_finite() {
        return 0.0;
    }
    let mut value = (longitude + 180.0).rem_euclid(360.0) - 180.0;
    if value == -180.0 && longitude > 0.0 {
        value = 180.0;
    }
    value
}

/// Great-circle distance using the WGS84 mean earth radius.  The result is
/// sufficiently accurate for a browse list and deterministic in unit tests.
pub fn distance_m(a: SearchCoordinate, b: SearchCoordinate) -> f64 {
    let radius = 6_371_008.8_f64;
    let lat1 = a.latitude.to_radians();
    let lat2 = b.latitude.to_radians();
    let dlat = (b.latitude - a.latitude).to_radians();
    let dlon = (b.longitude - a.longitude).to_radians();
    let h = (dlat * 0.5).sin().powi(2) + lat1.cos() * lat2.cos() * (dlon * 0.5).sin().powi(2);
    radius * 2.0 * h.clamp(0.0, 1.0).sqrt().asin()
}

/// Sort by review count (the product requirement), then rating and distance.
/// The final id tie-breaker keeps rendering stable across providers and runs.
pub fn sort_places(places: &mut [PlaceSummary]) {
    places.sort_by(|a, b| {
        b.review_count
            .cmp(&a.review_count)
            .then_with(|| {
                b.rating
                    .unwrap_or_default()
                    .partial_cmp(&a.rating.unwrap_or_default())
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| {
                a.distance_m
                    .partial_cmp(&b.distance_m)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
    });
}

pub struct GooglePlacesClient {
    agent: ureq::Agent,
    base_url: String,
}

impl Default for GooglePlacesClient {
    fn default() -> Self {
        Self::new(DEFAULT_GOOGLE_BASE_URL)
    }
}

impl GooglePlacesClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(12)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.new_agent(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    pub fn search(
        &self,
        api_key: &str,
        request: NearbySearchRequest,
    ) -> Result<Vec<PlaceSummary>, PlacesError> {
        if api_key.trim().is_empty() {
            return Err(PlacesError::MissingApiKey);
        }
        let request = request.normalized();
        let body = serde_json::json!({
            "maxResultCount": request.limit,
            "rankPreference": "POPULARITY",
            "languageCode": request.language.code(),
            "locationRestriction": {
                "circle": {
                    "center": {
                        "latitude": request.coordinate.latitude,
                        "longitude": request.coordinate.longitude,
                    },
                    "radius": request.radius_m as f64,
                }
            }
        });
        let response = self
            .agent
            .post(format!("{}/v1/places:searchNearby", self.base_url))
            .header("Content-Type", "application/json")
            .header("X-Goog-Api-Key", api_key.trim())
            .header(
                "X-Goog-FieldMask",
                "places.id,places.displayName,places.formattedAddress,places.location,places.primaryTypeDisplayName,places.rating,places.userRatingCount,places.currentOpeningHours,places.googleMapsUri",
            )
            .send(
                serde_json::to_vec(&body)
                    .map_err(|error| PlacesError::Parse(error.to_string()))?,
            )
            .map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(provider_status(status, text));
        }
        let decoded: GoogleResponse =
            serde_json::from_str(&text).map_err(|error| PlacesError::Parse(error.to_string()))?;
        let mut places = decoded
            .places
            .unwrap_or_default()
            .into_iter()
            .filter_map(|place| place.into_summary(request.coordinate))
            .collect::<Vec<_>>();
        sort_places(&mut places);
        if places.is_empty() {
            return Err(PlacesError::Empty);
        }
        Ok(places)
    }
}

pub struct OverpassClient {
    agent: ureq::Agent,
    base_url: String,
}

impl Default for OverpassClient {
    fn default() -> Self {
        Self::new(DEFAULT_OVERPASS_BASE_URL)
    }
}

impl OverpassClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(15)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.new_agent(),
            base_url: base_url.into(),
        }
    }

    pub fn search(&self, request: NearbySearchRequest) -> Result<Vec<PlaceSummary>, PlacesError> {
        let request = request.normalized();
        let query = format!(
            "[out:json][timeout:12];(nwr(around:{},{}, {})[\"name\"][\"amenity\"];nwr(around:{},{}, {})[\"name\"][\"tourism\"];nwr(around:{},{}, {})[\"name\"][\"shop\"];nwr(around:{},{}, {})[\"name\"][\"leisure\"];nwr(around:{},{}, {})[\"name\"][\"historic\"];);out center tags;",
            request.radius_m,
            request.coordinate.latitude,
            request.coordinate.longitude,
            request.radius_m,
            request.coordinate.latitude,
            request.coordinate.longitude,
            request.radius_m,
            request.coordinate.latitude,
            request.coordinate.longitude,
            request.radius_m,
            request.coordinate.latitude,
            request.coordinate.longitude,
            request.radius_m,
            request.coordinate.latitude,
            request.coordinate.longitude,
        );
        let response = self
            .agent
            .post(&self.base_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", "GPXAnimatorNative/2.0")
            .send_form([("data", query.as_str())])
            .map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(provider_status(status, text));
        }
        let decoded: OverpassResponse =
            serde_json::from_str(&text).map_err(|error| PlacesError::Parse(error.to_string()))?;
        let mut seen = HashSet::new();
        let mut places = Vec::new();
        for element in decoded.elements {
            let Some(summary) = element.into_summary(request.coordinate) else {
                continue;
            };
            if seen.insert(summary.id.clone()) {
                places.push(summary);
            }
        }
        sort_places(&mut places);
        if places.is_empty() {
            return Err(PlacesError::Empty);
        }
        Ok(places.into_iter().take(request.limit as usize).collect())
    }
}

fn map_ureq_error(error: ureq::Error) -> PlacesError {
    PlacesError::Http(error.to_string())
}

fn provider_status(status: u16, body: String) -> PlacesError {
    match status {
        401 | 403 => PlacesError::Unauthorized(body.chars().take(240).collect()),
        429 => PlacesError::RateLimited,
        _ => PlacesError::Http(format!(
            "HTTP {status}: {}",
            body.chars().take(240).collect::<String>()
        )),
    }
}

#[derive(Debug, Deserialize)]
struct GoogleResponse {
    places: Option<Vec<GooglePlace>>,
}

#[derive(Debug, Deserialize)]
struct GooglePlace {
    id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<LocalizedText>,
    #[serde(rename = "formattedAddress")]
    formatted_address: Option<String>,
    location: Option<GoogleLocation>,
    #[serde(rename = "primaryTypeDisplayName")]
    primary_type_display_name: Option<LocalizedText>,
    rating: Option<f32>,
    #[serde(rename = "userRatingCount")]
    user_rating_count: Option<u32>,
    #[serde(rename = "currentOpeningHours")]
    current_opening_hours: Option<OpeningHours>,
    #[serde(rename = "googleMapsUri")]
    google_maps_uri: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LocalizedText {
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GoogleLocation {
    latitude: Option<f64>,
    longitude: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct OpeningHours {
    #[serde(rename = "openNow")]
    open_now: Option<bool>,
}

impl GooglePlace {
    fn into_summary(self, origin: SearchCoordinate) -> Option<PlaceSummary> {
        let id = self.id?;
        let location = self.location?;
        let latitude = location.latitude?;
        let longitude = location.longitude?;
        let name = self.display_name?.text?.trim().to_owned();
        if name.is_empty() {
            return None;
        }
        Some(PlaceSummary {
            provider: PlaceProvider::Google,
            id: id.clone(),
            name,
            category: self.primary_type_display_name.and_then(|v| v.text),
            address: self.formatted_address,
            latitude,
            longitude,
            distance_m: distance_m(
                origin,
                SearchCoordinate {
                    latitude,
                    longitude,
                },
            ),
            rating: self.rating,
            review_count: self.user_rating_count.unwrap_or_default(),
            open_now: self.current_opening_hours.and_then(|v| v.open_now),
            external_url: self.google_maps_uri.unwrap_or_else(|| {
                format!("https://www.google.com/maps/search/?api=1&query={latitude},{longitude}")
            }),
        })
    }
}

#[derive(Debug, Deserialize)]
struct OverpassResponse {
    elements: Vec<OverpassElement>,
}

#[derive(Debug, Deserialize)]
struct OverpassElement {
    #[serde(rename = "type")]
    element_type: String,
    id: u64,
    lat: Option<f64>,
    lon: Option<f64>,
    center: Option<OverpassCenter>,
    tags: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct OverpassCenter {
    lat: f64,
    lon: f64,
}

impl OverpassElement {
    fn into_summary(self, origin: SearchCoordinate) -> Option<PlaceSummary> {
        let tags = self.tags?;
        let name = tags.get("name")?.trim().to_owned();
        if name.is_empty() {
            return None;
        }
        let (latitude, longitude) = match (self.lat, self.lon, self.center) {
            (Some(lat), Some(lon), _) => (lat, lon),
            (_, _, Some(center)) => (center.lat, center.lon),
            _ => return None,
        };
        let category = ["amenity", "tourism", "shop", "leisure", "historic"]
            .iter()
            .find_map(|key| tags.get(*key).cloned());
        let address = [
            tags.get("addr:housenumber"),
            tags.get("addr:street"),
            tags.get("addr:city"),
        ]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>();
        let address = (!address.is_empty()).then(|| address.join(" "));
        let id = format!("{}:{}", self.element_type, self.id);
        Some(PlaceSummary {
            provider: PlaceProvider::OpenStreetMap,
            id,
            name,
            category,
            address,
            latitude,
            longitude,
            distance_m: distance_m(
                origin,
                SearchCoordinate {
                    latitude,
                    longitude,
                },
            ),
            rating: None,
            review_count: 0,
            open_now: None,
            external_url: format!(
                "https://www.openstreetmap.org/?mlat={latitude}&mlon={longitude}#map=18/{latitude}/{longitude}"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn place(id: &str, reviews: u32, rating: Option<f32>, distance_m: f64) -> PlaceSummary {
        PlaceSummary {
            provider: PlaceProvider::Google,
            id: id.into(),
            name: id.into(),
            category: None,
            address: None,
            latitude: 25.0,
            longitude: 121.0,
            distance_m,
            rating,
            review_count: reviews,
            open_now: None,
            external_url: String::new(),
        }
    }

    fn mock_server(status: u16, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request);
            let response = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        format!("http://{address}")
    }

    #[test]
    fn normalize_radius_snaps_to_supported_values() {
        assert_eq!(normalize_radius(0), 500);
        assert_eq!(normalize_radius(1_600), 2_000);
        assert_eq!(normalize_radius(9_000), 5_000);
    }

    #[test]
    fn normalize_longitude_wraps_dateline_and_handles_nan() {
        assert!((normalize_longitude(181.0) + 179.0).abs() < f64::EPSILON);
        assert_eq!(normalize_longitude(f64::NAN), 0.0);
    }

    #[test]
    fn distance_is_zero_and_known_equator_distance() {
        let origin = SearchCoordinate {
            latitude: 0.0,
            longitude: 0.0,
        };
        assert!(distance_m(origin, origin).abs() < 1e-9);
        let one_degree = distance_m(
            origin,
            SearchCoordinate {
                latitude: 0.0,
                longitude: 1.0,
            },
        );
        assert!((one_degree - 111_195.08).abs() < 1.0);
    }

    #[test]
    fn sorting_is_review_count_then_rating_then_distance() {
        let mut values = vec![
            place("far", 12, Some(4.9), 100.0),
            place("near", 12, Some(4.9), 10.0),
            place("popular", 100, Some(3.0), 1_000.0),
            place("higher", 12, Some(4.8), 1.0),
        ];
        sort_places(&mut values);
        assert_eq!(
            values.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            ["popular", "near", "far", "higher"]
        );
    }

    #[test]
    fn google_fixture_maps_nested_fields_and_distance() {
        let json = r#"{"places":[{"id":"g1","displayName":{"text":"Cafe"},"formattedAddress":"1 Main","location":{"latitude":25.001,"longitude":121.002},"primaryTypeDisplayName":{"text":"Cafe"},"rating":4.7,"userRatingCount":231,"currentOpeningHours":{"openNow":true},"googleMapsUri":"https://maps.google.test/g1"}]}"#;
        let decoded: GoogleResponse = serde_json::from_str(json).unwrap();
        let summary = decoded
            .places
            .unwrap()
            .pop()
            .unwrap()
            .into_summary(SearchCoordinate {
                latitude: 25.0,
                longitude: 121.0,
            })
            .unwrap();
        assert_eq!(summary.name, "Cafe");
        assert_eq!(summary.review_count, 231);
        assert_eq!(summary.open_now, Some(true));
        assert!(summary.distance_m > 0.0);
    }

    #[test]
    fn overpass_fixture_uses_center_and_builds_address() {
        let json = r#"{"elements":[{"type":"way","id":42,"center":{"lat":25.01,"lon":121.02},"tags":{"name":"Park","leisure":"park","addr:street":"Main","addr:housenumber":"9","addr:city":"Taipei"}}]}"#;
        let decoded: OverpassResponse = serde_json::from_str(json).unwrap();
        let summary = decoded
            .elements
            .into_iter()
            .next()
            .unwrap()
            .into_summary(SearchCoordinate {
                latitude: 25.0,
                longitude: 121.0,
            })
            .unwrap();
        assert_eq!(summary.id, "way:42");
        assert_eq!(summary.address.as_deref(), Some("9 Main Taipei"));
        assert_eq!(summary.provider, PlaceProvider::OpenStreetMap);
    }

    #[test]
    fn request_normalization_clamps_coordinates_and_limit() {
        let request = NearbySearchRequest {
            coordinate: SearchCoordinate {
                latitude: 90.0,
                longitude: 540.0,
            },
            radius_m: 700,
            limit: 99,
            language: PlaceLanguage::English,
        }
        .normalized();
        assert_eq!(request.coordinate.latitude, 85.051_128_78);
        assert_eq!(request.coordinate.longitude, 180.0);
        assert_eq!(request.radius_m, 500);
        assert_eq!(request.limit, 20);
    }

    #[test]
    fn google_client_rejects_empty_key_before_network() {
        let client = GooglePlacesClient::default();
        let result = client.search(
            "  ",
            NearbySearchRequest {
                coordinate: SearchCoordinate {
                    latitude: 25.0,
                    longitude: 121.0,
                },
                radius_m: DEFAULT_RADIUS_M,
                limit: 10,
                language: PlaceLanguage::TraditionalChinese,
            },
        );
        assert_eq!(result, Err(PlacesError::MissingApiKey));
    }

    #[test]
    fn google_client_http_mock_parses_and_sorts_results() {
        let body = r#"{"places":[{"id":"g-low","displayName":{"text":"Low"},"location":{"latitude":25.001,"longitude":121.002},"rating":5.0,"userRatingCount":2},{"id":"g-high","displayName":{"text":"High"},"location":{"latitude":25.003,"longitude":121.004},"rating":4.1,"userRatingCount":99}]}"#;
        let client = GooglePlacesClient::new(mock_server(200, body));
        let places = client
            .search(
                "test-key",
                NearbySearchRequest {
                    coordinate: SearchCoordinate {
                        latitude: 25.0,
                        longitude: 121.0,
                    },
                    radius_m: DEFAULT_RADIUS_M,
                    limit: 20,
                    language: PlaceLanguage::English,
                },
            )
            .unwrap();
        assert_eq!(places[0].id, "g-high");
        assert_eq!(places[1].id, "g-low");
    }

    #[test]
    fn overpass_http_mock_parses_nodes_and_limits_results() {
        let body = r#"{"elements":[{"type":"node","id":1,"lat":25.001,"lon":121.002,"tags":{"name":"One","amenity":"cafe"}},{"type":"node","id":2,"lat":25.002,"lon":121.003,"tags":{"name":"Two","amenity":"park"}}]}"#;
        let client = OverpassClient::new(mock_server(200, body));
        let places = client
            .search(NearbySearchRequest {
                coordinate: SearchCoordinate {
                    latitude: 25.0,
                    longitude: 121.0,
                },
                radius_m: 500,
                limit: 1,
                language: PlaceLanguage::TraditionalChinese,
            })
            .unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].id, "node:1");
    }

    #[test]
    fn provider_http_mock_maps_rate_limit() {
        let body = "{\"error\":\"slow down\"}";
        let client = GooglePlacesClient::new(mock_server(429, body));
        let result = client.search(
            "test-key",
            NearbySearchRequest {
                coordinate: SearchCoordinate {
                    latitude: 25.0,
                    longitude: 121.0,
                },
                radius_m: DEFAULT_RADIUS_M,
                limit: 10,
                language: PlaceLanguage::English,
            },
        );
        assert_eq!(result, Err(PlacesError::RateLimited));
    }
}
