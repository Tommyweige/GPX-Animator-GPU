//! Modern provider clients kept separate from the legacy compatibility clients.
//!
//! The TomTom client follows the Orbis Places Search v3 header contract and
//! the Foursquare client requests only fields needed by the nearby dialog.
//! Both clients expose pure response parsers so unit tests never require a
//! live API key or network access.

use crate::{
    NearbySearchRequest, PlaceProvider, PlaceSummary, PlacesError, SearchCoordinate, distance_m,
    map_ureq_error, provider_status,
};
use serde::Deserialize;

const DEFAULT_TOMTOM_V3_BASE_URL: &str = "https://api.tomtom.com";
const DEFAULT_FOURSQUARE_BASE_URL: &str = "https://places-api.foursquare.com";

pub struct TomTomV3PlacesClient {
    agent: ureq::Agent,
    base_url: String,
}

impl Default for TomTomV3PlacesClient {
    fn default() -> Self {
        Self::new(DEFAULT_TOMTOM_V3_BASE_URL)
    }
}

impl TomTomV3PlacesClient {
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

    /// Query the Orbis Places Search v3 discover endpoint.  The endpoint is
    /// deliberately isolated so a future service-version change does not
    /// alter the fallback policy in `PoiService`.
    pub fn search(
        &self,
        api_key: &str,
        request: NearbySearchRequest,
    ) -> Result<Vec<PlaceSummary>, PlacesError> {
        if api_key.trim().is_empty() {
            return Err(PlacesError::MissingApiKey);
        }
        let request = request.normalized();
        let origin = format!(
            "{},{}",
            request.coordinate.longitude, request.coordinate.latitude
        );
        let response = self
            .agent
            .get(format!("{}/maps/orbis/places/discover", self.base_url))
            .query("origin", origin)
            .query("radius", request.radius_m.to_string())
            .query("limit", request.limit.to_string())
            .query("language", request.language.code())
            .query("view", "TW")
            .header("TomTom-Api-Key", api_key.trim())
            .header("TomTom-Api-Version", "3")
            .header(
                "Attributes",
                "results(id,type,title,subtitles,position,address,contacts,categories,distanceInMeters,openingHours,score)",
            )
            .header("Accept", "application/json")
            .call()
            .map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(provider_status(status, text));
        }
        let mut places = parse_tomtom_v3_response(&text, request.coordinate)?;
        places.truncate(request.limit as usize);
        if places.is_empty() {
            return Err(PlacesError::Empty);
        }
        Ok(places)
    }

    pub fn details(
        &self,
        api_key: &str,
        id: &str,
        origin: Option<SearchCoordinate>,
    ) -> Result<PlaceSummary, PlacesError> {
        if api_key.trim().is_empty() {
            return Err(PlacesError::MissingApiKey);
        }
        if id.trim().is_empty() {
            return Err(PlacesError::Parse("TomTom place id is empty".into()));
        }
        let response = self
            .agent
            .get(format!(
                "{}/maps/orbis/places/details/pois/{}",
                self.base_url,
                url_encode_component(id.trim())
            ))
            .header("TomTom-Api-Key", api_key.trim())
            .header("TomTom-Api-Version", "3")
            .header(
                "Attributes",
                "id,type,title,subtitles,position,address,contacts,categories,distanceInMeters,openingHours,score",
            )
            .header("Accept", "application/json")
            .call()
            .map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(provider_status(status, text));
        }
        parse_tomtom_v3_response(
            &text,
            origin.unwrap_or(SearchCoordinate {
                latitude: 0.0,
                longitude: 0.0,
            }),
        )?
        .into_iter()
        .next()
        .ok_or(PlacesError::Empty)
    }
}

pub fn parse_tomtom_v3_response(
    text: &str,
    origin: SearchCoordinate,
) -> Result<Vec<PlaceSummary>, PlacesError> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| PlacesError::Parse(format!("TomTom v3: {error}")))?;
    let candidates = value
        .get("results")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .or_else(|| {
            value
                .get("places")
                .and_then(serde_json::Value::as_array)
                .cloned()
        })
        .or_else(|| value.is_object().then(|| vec![value.clone()]))
        .unwrap_or_default();
    let mut results = candidates
        .into_iter()
        .filter_map(|candidate| tomtom_v3_item_to_summary(&candidate, origin))
        .collect::<Vec<_>>();
    results.sort_by(|a, b| {
        b.provider_score
            .unwrap_or_default()
            .partial_cmp(&a.provider_score.unwrap_or_default())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.distance_m
                    .partial_cmp(&b.distance_m)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(results)
}

fn tomtom_v3_item_to_summary(
    item: &serde_json::Value,
    origin: SearchCoordinate,
) -> Option<PlaceSummary> {
    let place = item.get("place").unwrap_or(item);
    let id = string_field(place, &["id", "entityId", "placeId"])?;
    let name = string_field(place, &["title", "name"])?;
    if name.trim().is_empty() {
        return None;
    }
    let (latitude, longitude) = coordinates(place)?;
    if !latitude.is_finite() || !longitude.is_finite() {
        return None;
    }
    let address = place
        .get("address")
        .and_then(|v| {
            v.get("formattedAddress")
                .or_else(|| v.get("freeformAddress"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_owned)
        .or_else(|| string_field(place, &["address"]));
    let category = place
        .get("categories")
        .and_then(serde_json::Value::as_array)
        .and_then(|values| values.first())
        .and_then(|value| {
            value
                .get("name")
                .or_else(|| value.get("title"))
                .and_then(serde_json::Value::as_str)
        })
        .map(str::to_owned)
        .or_else(|| {
            place
                .get("categories")
                .and_then(serde_json::Value::as_array)
                .and_then(|values| values.first().and_then(serde_json::Value::as_str))
                .map(str::to_owned)
        });
    let distance_m = number_field(place, &["distanceInMeters", "distance"]).unwrap_or_else(|| {
        distance_m(
            origin,
            SearchCoordinate {
                latitude,
                longitude,
            },
        )
    });
    let provider_score = number_field(place, &["score", "relevance"]).map(|v| v as f32);
    let phone = contact_value(place, "phone");
    let website = contact_value(place, "url").and_then(crate::normalize_external_url);
    let open_now = place
        .get("openingHours")
        .and_then(|value| value.get("openNow"))
        .and_then(serde_json::Value::as_bool);
    Some(PlaceSummary {
        provider: PlaceProvider::TomTom,
        id: id.clone(),
        name: name.trim().to_owned(),
        category,
        address,
        latitude,
        longitude,
        distance_m,
        rating: None,
        review_count: 0,
        open_now,
        provider_score,
        phone,
        website,
        external_url: format!(
            "https://www.google.com/maps/search/?api=1&query={latitude},{longitude}"
        ),
        rating_scale: None,
        popularity: None,
        popularity_source: None,
        source_updated_at: None,
    })
}

fn coordinates(value: &serde_json::Value) -> Option<(f64, f64)> {
    if let Some(position) = value.get("position") {
        if let Some(coordinates) = position.get("coordinates").and_then(|v| v.as_array()) {
            let longitude = coordinates.first()?.as_f64()?;
            let latitude = coordinates.get(1)?.as_f64()?;
            return Some((latitude, longitude));
        }
        let latitude = position
            .get("lat")
            .or_else(|| position.get("latitude"))
            .and_then(serde_json::Value::as_f64);
        let longitude = position
            .get("lon")
            .or_else(|| position.get("longitude"))
            .and_then(serde_json::Value::as_f64);
        if let (Some(latitude), Some(longitude)) = (latitude, longitude) {
            return Some((latitude, longitude));
        }
    }
    let latitude = value.get("latitude").and_then(serde_json::Value::as_f64);
    let longitude = value.get("longitude").and_then(serde_json::Value::as_f64);
    latitude.zip(longitude)
}

fn string_field(value: &serde_json::Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        value
            .get(*name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    })
}

fn number_field(value: &serde_json::Value, names: &[&str]) -> Option<f64> {
    names
        .iter()
        .find_map(|name| value.get(*name).and_then(serde_json::Value::as_f64))
}

fn contact_value(value: &serde_json::Value, key: &str) -> Option<String> {
    let contacts = value.get("contacts")?;
    let raw = contacts.get(key)?;
    raw.as_str().map(str::to_owned).or_else(|| {
        raw.as_array()?
            .first()?
            .get("value")?
            .as_str()
            .map(str::to_owned)
    })
}

fn url_encode_component(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                vec![byte as char]
            }
            other => format!("%{other:02X}").chars().collect(),
        })
        .collect()
}

pub struct FoursquarePlacesClient {
    agent: ureq::Agent,
    base_url: String,
}

impl Default for FoursquarePlacesClient {
    fn default() -> Self {
        Self::new(DEFAULT_FOURSQUARE_BASE_URL)
    }
}

impl FoursquarePlacesClient {
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
        let ll = format!(
            "{},{}",
            request.coordinate.latitude, request.coordinate.longitude
        );
        let response = self
            .agent
            .get(format!("{}/places/search", self.base_url))
            .query("ll", ll)
            .query("radius", request.radius_m.to_string())
            .query("limit", request.limit.to_string())
            .query("sort", "POPULARITY")
            .query(
                "fields",
                "fsq_id,name,location,categories,distance,rating,stats,popularity,website,tel,link,hours",
            )
            .header("Authorization", api_key.trim())
            .header("Accept", "application/json")
            .call()
            .map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(provider_status(status, text));
        }
        let mut places = parse_foursquare_response(&text, request.coordinate)?;
        places.truncate(request.limit as usize);
        if places.is_empty() {
            return Err(PlacesError::Empty);
        }
        Ok(places)
    }
}

pub fn parse_foursquare_response(
    text: &str,
    origin: SearchCoordinate,
) -> Result<Vec<PlaceSummary>, PlacesError> {
    let response: FoursquareResponse = serde_json::from_str(text)
        .map_err(|error| PlacesError::Parse(format!("Foursquare: {error}")))?;
    let mut places = response
        .results
        .into_iter()
        .filter_map(|place| place.into_summary(origin))
        .collect::<Vec<_>>();
    places.sort_by(|a, b| {
        b.popularity
            .unwrap_or_default()
            .partial_cmp(&a.popularity.unwrap_or_default())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.rating
                    .unwrap_or_default()
                    .partial_cmp(&a.rating.unwrap_or_default())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                a.distance_m
                    .partial_cmp(&b.distance_m)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(places)
}

#[derive(Debug, Deserialize)]
struct FoursquareResponse {
    #[serde(default, alias = "places")]
    results: Vec<FoursquarePlace>,
}

#[derive(Debug, Deserialize)]
struct FoursquarePlace {
    fsq_id: Option<String>,
    name: Option<String>,
    location: Option<FoursquareLocation>,
    categories: Option<Vec<FoursquareCategory>>,
    distance: Option<f64>,
    rating: Option<f32>,
    stats: Option<FoursquareStats>,
    popularity: Option<f32>,
    website: Option<String>,
    tel: Option<String>,
    link: Option<String>,
    hours: Option<FoursquareHours>,
}

#[derive(Debug, Deserialize)]
struct FoursquareLocation {
    latitude: Option<f64>,
    longitude: Option<f64>,
    formatted_address: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FoursquareCategory {
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FoursquareStats {
    total_ratings: Option<u32>,
    #[serde(alias = "totalRatings")]
    total_ratings_camel: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct FoursquareHours {
    open_now: Option<bool>,
}

impl FoursquarePlace {
    fn into_summary(self, origin: SearchCoordinate) -> Option<PlaceSummary> {
        let id = self.fsq_id?;
        let name = self.name?.trim().to_owned();
        let location = self.location?;
        let latitude = location.latitude?;
        let longitude = location.longitude?;
        if name.is_empty() || !latitude.is_finite() || !longitude.is_finite() {
            return None;
        }
        let review_count = self
            .stats
            .and_then(|stats| stats.total_ratings.or(stats.total_ratings_camel))
            .unwrap_or_default();
        let external_url = self
            .link
            .clone()
            .unwrap_or_else(|| format!("https://foursquare.com/v/{id}"));
        Some(PlaceSummary {
            provider: PlaceProvider::Foursquare,
            id,
            name,
            category: self
                .categories
                .and_then(|mut values| values.drain(..).next())
                .and_then(|value| value.name),
            address: location.formatted_address,
            latitude,
            longitude,
            distance_m: self.distance.unwrap_or_else(|| {
                distance_m(
                    origin,
                    SearchCoordinate {
                        latitude,
                        longitude,
                    },
                )
            }),
            rating: self.rating,
            review_count,
            open_now: self.hours.and_then(|hours| hours.open_now),
            provider_score: None,
            phone: self.tel,
            website: self.website.and_then(crate::normalize_external_url),
            external_url,
            rating_scale: self.rating.map(|_| 10),
            popularity: self.popularity.map(|value| value.clamp(0.0, 1.0)),
            popularity_source: self.popularity.map(|_| PlaceProvider::Foursquare),
            source_updated_at: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tomtom_v3_parser_accepts_geojson_coordinates_and_contacts() {
        let body = r#"{"results":[{"id":"tt-v3","title":"Fresh Cafe","position":{"coordinates":[121.532,25.041]},"address":{"formattedAddress":"Taipei"},"categories":[{"name":"Cafe"}],"contacts":{"phone":[{"value":"02-1"}],"url":[{"value":"https://cafe.example"}]},"distanceInMeters":42,"score":0.8}]}"#;
        let places = parse_tomtom_v3_response(
            body,
            SearchCoordinate {
                latitude: 25.0,
                longitude: 121.0,
            },
        )
        .unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].provider, PlaceProvider::TomTom);
        assert_eq!(places[0].website.as_deref(), Some("https://cafe.example"));
        assert_eq!(places[0].distance_m, 42.0);
    }

    #[test]
    fn foursquare_parser_preserves_rating_scale_and_popularity() {
        let body = r#"{"results":[{"fsq_id":"fsq-1","name":"Popular","location":{"latitude":25.01,"longitude":121.02,"formatted_address":"Taipei"},"categories":[{"name":"Cafe"}],"rating":8.7,"stats":{"total_ratings":321},"popularity":0.92,"distance":120,"hours":{"open_now":true},"link":"https://foursquare.test/1"}]}"#;
        let places = parse_foursquare_response(
            body,
            SearchCoordinate {
                latitude: 25.0,
                longitude: 121.0,
            },
        )
        .unwrap();
        assert_eq!(places[0].provider, PlaceProvider::Foursquare);
        assert_eq!(places[0].rating_scale, Some(10));
        assert_eq!(places[0].review_count, 321);
        assert_eq!(places[0].popularity, Some(0.92));
        assert_eq!(places[0].open_now, Some(true));
    }

    #[test]
    fn empty_provider_payload_is_not_a_fake_success() {
        assert!(matches!(
            parse_foursquare_response("{\"results\":[]}", SearchCoordinate { latitude: 0.0, longitude: 0.0 }),
            Ok(values) if values.is_empty()
        ));
    }
}
