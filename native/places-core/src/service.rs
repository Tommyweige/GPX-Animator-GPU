//! Profile-aware POI orchestration and explicit fallback policy.

use crate::domain::{
    FallbackAttempt, FallbackOutcome, PoiProfile, PoiSearchResponse, ProviderCredentials,
};
use crate::{
    FoursquarePlacesClient, GooglePlacesClient, LocalPoiCatalog, NearbySearchRequest,
    PlaceProvider, PlaceSummary, PlacesError, SearchCoordinate, TomTomPlacesClient,
    TomTomV3PlacesClient, sort_by_distance,
};
use serde::Deserialize;

/// A small HTTP contract for the future commercial gateway.  Keeping this
/// client in the native crate lets the UI and tests exercise the same fallback
/// behavior before a hosted gateway is deployed.
pub struct GatewayPlacesClient {
    agent: ureq::Agent,
}

impl Default for GatewayPlacesClient {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(12)))
            .http_status_as_error(false)
            .build();
        Self {
            agent: config.new_agent(),
        }
    }
}

impl GatewayPlacesClient {
    pub fn search(
        &self,
        base_url: &str,
        bearer_token: &str,
        request: NearbySearchRequest,
    ) -> Result<Vec<PlaceSummary>, PlacesError> {
        if base_url.trim().is_empty() || bearer_token.trim().is_empty() {
            return Err(PlacesError::NotConfigured("gateway URL/token".into()));
        }
        let request = request.normalized();
        let response = self
            .agent
            .get(format!(
                "{}/v1/nearby-places",
                base_url.trim_end_matches('/')
            ))
            .query("lat", request.coordinate.latitude.to_string())
            .query("lon", request.coordinate.longitude.to_string())
            .query("radius_m", request.radius_m.to_string())
            .query("limit", request.limit.to_string())
            .query("language", request.language.code())
            .header("Authorization", format!("Bearer {}", bearer_token.trim()))
            .header("Accept", "application/json")
            .call()
            .map_err(crate::map_ureq_error)?;
        let status = response.status().as_u16();
        let text = response
            .into_body()
            .read_to_string()
            .map_err(|error| PlacesError::Http(error.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(crate::provider_status(status, text));
        }
        let decoded: GatewayResponse = serde_json::from_str(&text)
            .map_err(|error| PlacesError::Parse(format!("gateway: {error}")))?;
        let mut places = decoded
            .places
            .into_iter()
            .filter_map(|place| place.into_summary(request.coordinate))
            .collect::<Vec<_>>();
        sort_by_distance(&mut places);
        places.truncate(request.limit as usize);
        if places.is_empty() {
            return Err(PlacesError::Empty);
        }
        Ok(places)
    }
}

#[derive(Debug, Deserialize)]
struct GatewayResponse {
    #[serde(default)]
    places: Vec<GatewayPlace>,
}

#[derive(Debug, Deserialize)]
struct GatewayPlace {
    id: Option<String>,
    name: Option<String>,
    category: Option<String>,
    address: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    rating: Option<f32>,
    rating_scale: Option<u8>,
    review_count: Option<u32>,
    popularity: Option<f32>,
    open_now: Option<bool>,
    phone: Option<String>,
    website: Option<String>,
    external_url: Option<String>,
}

impl GatewayPlace {
    fn into_summary(self, origin: SearchCoordinate) -> Option<PlaceSummary> {
        let id = self.id?;
        let name = self.name?.trim().to_owned();
        let latitude = self.latitude?;
        let longitude = self.longitude?;
        if name.is_empty() || !latitude.is_finite() || !longitude.is_finite() {
            return None;
        }
        Some(PlaceSummary {
            provider: PlaceProvider::Gateway,
            id,
            name,
            category: self.category,
            address: self.address,
            latitude,
            longitude,
            distance_m: crate::distance_m(
                origin,
                SearchCoordinate {
                    latitude,
                    longitude,
                },
            ),
            rating: self.rating,
            review_count: self.review_count.unwrap_or_default(),
            open_now: self.open_now,
            provider_score: None,
            phone: self.phone,
            website: self.website.and_then(crate::normalize_external_url),
            external_url: self.external_url.unwrap_or_else(|| {
                format!("https://www.google.com/maps/search/?api=1&query={latitude},{longitude}")
            }),
            rating_scale: self.rating.map(|_| self.rating_scale.unwrap_or(5)),
            popularity: self.popularity.map(|value| value.clamp(0.0, 1.0)),
            popularity_source: self.popularity.map(|_| PlaceProvider::Gateway),
            source_updated_at: None,
        })
    }
}

pub struct PoiService {
    pub local: Option<LocalPoiCatalog>,
    pub tomtom_v3: TomTomV3PlacesClient,
    pub tomtom_legacy: TomTomPlacesClient,
    pub foursquare: FoursquarePlacesClient,
    pub google: GooglePlacesClient,
    pub gateway: GatewayPlacesClient,
}

impl Default for PoiService {
    fn default() -> Self {
        Self::new(None)
    }
}

impl PoiService {
    pub fn new(local: Option<LocalPoiCatalog>) -> Self {
        Self {
            local,
            tomtom_v3: TomTomV3PlacesClient::default(),
            tomtom_legacy: TomTomPlacesClient::default(),
            foursquare: FoursquarePlacesClient::default(),
            google: GooglePlacesClient::default(),
            gateway: GatewayPlacesClient::default(),
        }
    }

    pub fn search(
        &self,
        profile: PoiProfile,
        request: NearbySearchRequest,
        credentials: &ProviderCredentials,
        online: bool,
        force_refresh: bool,
    ) -> Result<PoiSearchResponse, PlacesError> {
        let request = request.normalized();
        let mut response = PoiSearchResponse::empty(profile, request.clone());
        let local = self.local_results(&request, &mut response);
        let local_places = local.unwrap_or_default();

        match profile {
            PoiProfile::OfflineFree => {
                response.places = local_places;
                response.attribution = vec![
                    "Overture Maps Foundation data (see pack license)".into(),
                    "© OpenStreetMap contributors (ODbL)".into(),
                ];
                if response.places.is_empty() {
                    response.degraded = true;
                }
            }
            PoiProfile::TomTomLive => {
                response.attribution = vec!["TomTom Places Search (your API key)".into()];
                response.places = self.base_with_tomtom(
                    request.clone(),
                    credentials,
                    online,
                    force_refresh,
                    local_places,
                    &mut response,
                );
            }
            PoiProfile::FoursquareEnhanced => {
                response.attribution = vec![
                    "Foursquare Places (Premium fields require your plan)".into(),
                    "TomTom Places Search (your API key)".into(),
                ];
                let base = self.base_with_tomtom(
                    request.clone(),
                    credentials,
                    online,
                    force_refresh,
                    local_places,
                    &mut response,
                );
                response.places =
                    self.enrich_with_foursquare(request, credentials, online, base, &mut response);
            }
            PoiProfile::GoogleByok => {
                response.attribution = vec!["Google Places (your API key)".into()];
                if online {
                    if let Some(key) = credentials.google_api_key.as_deref() {
                        match self.google.search(key, request.clone()) {
                            Ok(places) => {
                                response.attempts.push(FallbackAttempt {
                                    provider: PlaceProvider::Google,
                                    outcome: FallbackOutcome::Succeeded {
                                        count: places.len(),
                                    },
                                });
                                response.places = places;
                            }
                            Err(error) => response.attempts.push(FallbackAttempt {
                                provider: PlaceProvider::Google,
                                outcome: FallbackOutcome::Failed(error.to_string()),
                            }),
                        }
                    } else {
                        response.attempts.push(FallbackAttempt {
                            provider: PlaceProvider::Google,
                            outcome: FallbackOutcome::Skipped("API key not configured".into()),
                        });
                    }
                }
                if response.places.is_empty() {
                    response.places = local_places;
                    response.degraded = true;
                }
            }
            PoiProfile::GatewayPro => {
                response.attribution = vec!["Gateway provider (organisation account)".into()];
                if online {
                    if let (Some(config), Some(token)) = (
                        credentials.gateway.as_ref().filter(|config| config.enabled),
                        credentials.gateway_bearer_token.as_deref(),
                    ) {
                        match self
                            .gateway
                            .search(&config.base_url, token, request.clone())
                        {
                            Ok(places) => {
                                response.attempts.push(FallbackAttempt {
                                    provider: PlaceProvider::Gateway,
                                    outcome: FallbackOutcome::Succeeded {
                                        count: places.len(),
                                    },
                                });
                                response.places = places;
                            }
                            Err(error) => response.attempts.push(FallbackAttempt {
                                provider: PlaceProvider::Gateway,
                                outcome: FallbackOutcome::Failed(error.to_string()),
                            }),
                        }
                    } else {
                        response.attempts.push(FallbackAttempt {
                            provider: PlaceProvider::Gateway,
                            outcome: FallbackOutcome::Skipped("gateway is not configured".into()),
                        });
                    }
                }
                if response.places.is_empty() {
                    response.places = self.base_with_tomtom(
                        request,
                        credentials,
                        online,
                        force_refresh,
                        local_places,
                        &mut response,
                    );
                    response.degraded = true;
                }
            }
        }
        Ok(response)
    }

    fn local_results(
        &self,
        request: &NearbySearchRequest,
        response: &mut PoiSearchResponse,
    ) -> Result<Vec<PlaceSummary>, PlacesError> {
        let Some(local) = &self.local else {
            response.attempts.push(FallbackAttempt {
                provider: PlaceProvider::Overture,
                outcome: FallbackOutcome::Skipped("no local data pack installed".into()),
            });
            return Ok(Vec::new());
        };
        let places = local.search(request.clone())?;
        response.attempts.push(FallbackAttempt {
            provider: PlaceProvider::Overture,
            outcome: if places.is_empty() {
                FallbackOutcome::Empty
            } else {
                FallbackOutcome::Succeeded {
                    count: places.len(),
                }
            },
        });
        Ok(places)
    }

    fn base_with_tomtom(
        &self,
        request: NearbySearchRequest,
        credentials: &ProviderCredentials,
        online: bool,
        force_refresh: bool,
        local_places: Vec<PlaceSummary>,
        response: &mut PoiSearchResponse,
    ) -> Vec<PlaceSummary> {
        if !online || (!force_refresh && !local_places.is_empty()) {
            response.attempts.push(FallbackAttempt {
                provider: PlaceProvider::TomTom,
                outcome: FallbackOutcome::Skipped(if !online {
                    "offline mode".into()
                } else {
                    "local snapshot satisfied query".into()
                }),
            });
            return local_places;
        }
        let Some(key) = credentials.tomtom_api_key.as_deref() else {
            response.attempts.push(FallbackAttempt {
                provider: PlaceProvider::TomTom,
                outcome: FallbackOutcome::Skipped("API key not configured".into()),
            });
            return local_places;
        };
        match self.tomtom_v3.search(key, request.clone()) {
            Ok(places) => {
                response.attempts.push(FallbackAttempt {
                    provider: PlaceProvider::TomTom,
                    outcome: FallbackOutcome::Succeeded {
                        count: places.len(),
                    },
                });
                merge_places(local_places, places, request.limit as usize)
            }
            Err(error) => {
                response.attempts.push(FallbackAttempt {
                    provider: PlaceProvider::TomTom,
                    outcome: FallbackOutcome::Failed(format!("v3: {error}")),
                });
                // Legacy Search remains a compatibility fallback for keys that
                // are not yet entitled to Orbis Places v3.
                match self.tomtom_legacy.search(key, request.clone()) {
                    Ok(places) => {
                        response.attempts.push(FallbackAttempt {
                            provider: PlaceProvider::TomTom,
                            outcome: FallbackOutcome::Succeeded {
                                count: places.len(),
                            },
                        });
                        merge_places(local_places, places, request.limit as usize)
                    }
                    Err(legacy_error) => {
                        response.attempts.push(FallbackAttempt {
                            provider: PlaceProvider::TomTom,
                            outcome: FallbackOutcome::Failed(format!("legacy: {legacy_error}")),
                        });
                        local_places
                    }
                }
            }
        }
    }

    fn enrich_with_foursquare(
        &self,
        request: NearbySearchRequest,
        credentials: &ProviderCredentials,
        online: bool,
        base: Vec<PlaceSummary>,
        response: &mut PoiSearchResponse,
    ) -> Vec<PlaceSummary> {
        if !online {
            response.degraded = true;
            response.attempts.push(FallbackAttempt {
                provider: PlaceProvider::Foursquare,
                outcome: FallbackOutcome::Skipped("offline mode".into()),
            });
            return base;
        }
        let Some(key) = credentials.foursquare_api_key.as_deref() else {
            response.degraded = true;
            response.attempts.push(FallbackAttempt {
                provider: PlaceProvider::Foursquare,
                outcome: FallbackOutcome::Skipped("Premium API key not configured".into()),
            });
            return base;
        };
        match self.foursquare.search(key, request.clone()) {
            Ok(enrichment) => {
                response.attempts.push(FallbackAttempt {
                    provider: PlaceProvider::Foursquare,
                    outcome: FallbackOutcome::Succeeded {
                        count: enrichment.len(),
                    },
                });
                merge_foursquare(base, enrichment, request.limit as usize)
            }
            Err(error) => {
                response.degraded = true;
                response.attempts.push(FallbackAttempt {
                    provider: PlaceProvider::Foursquare,
                    outcome: FallbackOutcome::Failed(error.to_string()),
                });
                base
            }
        }
    }
}

fn merge_places(
    mut local: Vec<PlaceSummary>,
    fresh: Vec<PlaceSummary>,
    limit: usize,
) -> Vec<PlaceSummary> {
    local.extend(fresh);
    dedupe_by_identity(&mut local);
    local.sort_by(|a, b| {
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
    local.truncate(limit);
    local
}

fn merge_foursquare(
    mut base: Vec<PlaceSummary>,
    enrichment: Vec<PlaceSummary>,
    limit: usize,
) -> Vec<PlaceSummary> {
    for candidate in enrichment {
        let match_index = base.iter().position(|place| {
            place.name.eq_ignore_ascii_case(&candidate.name)
                || crate::distance_m(
                    SearchCoordinate {
                        latitude: place.latitude,
                        longitude: place.longitude,
                    },
                    SearchCoordinate {
                        latitude: candidate.latitude,
                        longitude: candidate.longitude,
                    },
                ) < 120.0
        });
        if let Some(index) = match_index {
            let target = &mut base[index];
            target.rating = candidate.rating.or(target.rating);
            target.rating_scale = candidate.rating_scale.or(target.rating_scale);
            target.review_count = candidate.review_count.max(target.review_count);
            target.popularity = candidate.popularity.or(target.popularity);
            target.popularity_source = candidate.popularity_source.or(target.popularity_source);
            if target.phone.is_none() {
                target.phone = candidate.phone;
            }
            if target.website.is_none() {
                target.website = candidate.website;
            }
        } else {
            base.push(candidate);
        }
    }
    base.sort_by(|a, b| {
        b.popularity
            .unwrap_or_default()
            .partial_cmp(&a.popularity.unwrap_or_default())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.review_count.cmp(&a.review_count))
            .then_with(|| {
                a.distance_m
                    .partial_cmp(&b.distance_m)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.id.cmp(&b.id))
    });
    base.truncate(limit);
    base
}

fn dedupe_by_identity(places: &mut Vec<PlaceSummary>) {
    let mut seen = std::collections::HashSet::new();
    places.retain(|place| {
        let key = format!(
            "{}:{}:{}",
            place.name.trim().to_lowercase(),
            (place.latitude * 10_000.0).round() as i64,
            (place.longitude * 10_000.0).round() as i64
        );
        seen.insert(key)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalDataset, LocalPoiStore, PlaceLanguage};

    fn sample(provider: PlaceProvider, name: &str) -> PlaceSummary {
        PlaceSummary {
            provider,
            id: format!("{}-1", name.to_lowercase()),
            name: name.into(),
            category: Some("cafe".into()),
            address: None,
            latitude: 25.0,
            longitude: 121.0,
            distance_m: 0.0,
            rating: None,
            review_count: 0,
            open_now: None,
            provider_score: None,
            phone: None,
            website: None,
            external_url: String::new(),
            rating_scale: None,
            popularity: None,
            popularity_source: None,
            source_updated_at: None,
        }
    }

    #[test]
    fn offline_profile_never_requires_network_and_reports_local_attribution() {
        let store = LocalPoiStore::open_in_memory(LocalDataset::Overture).unwrap();
        store
            .upsert(&[sample(PlaceProvider::Overture, "Local")])
            .unwrap();
        let service = PoiService::new(Some(LocalPoiCatalog {
            overture: Some(store),
            osm: None,
        }));
        let response = service
            .search(
                PoiProfile::OfflineFree,
                NearbySearchRequest {
                    coordinate: SearchCoordinate {
                        latitude: 25.0,
                        longitude: 121.0,
                    },
                    radius_m: 500,
                    limit: 10,
                    language: PlaceLanguage::English,
                },
                &ProviderCredentials::default(),
                false,
                false,
            )
            .unwrap();
        assert_eq!(response.places.len(), 1);
        assert!(response.attribution.iter().any(|v| v.contains("Overture")));
    }

    #[test]
    fn tomtom_profile_without_key_falls_back_to_snapshot() {
        let store = LocalPoiStore::open_in_memory(LocalDataset::Overture).unwrap();
        store
            .upsert(&[sample(PlaceProvider::Overture, "Local")])
            .unwrap();
        let service = PoiService::new(Some(LocalPoiCatalog {
            overture: Some(store),
            osm: None,
        }));
        let response = service
            .search(
                PoiProfile::TomTomLive,
                NearbySearchRequest {
                    coordinate: SearchCoordinate {
                        latitude: 25.0,
                        longitude: 121.0,
                    },
                    radius_m: 500,
                    limit: 10,
                    language: PlaceLanguage::English,
                },
                &ProviderCredentials::default(),
                true,
                true,
            )
            .unwrap();
        assert_eq!(response.places.len(), 1);
        assert!(response.attempts.iter().any(|attempt| {
            attempt.provider == PlaceProvider::TomTom
                && matches!(attempt.outcome, FallbackOutcome::Skipped(_))
        }));
    }

    #[test]
    fn foursquare_merge_adds_real_popularity_without_mislabeling_relevance() {
        let mut base = vec![sample(PlaceProvider::TomTom, "Cafe")];
        base[0].provider_score = Some(0.99);
        let mut enrichment = sample(PlaceProvider::Foursquare, "Cafe");
        enrichment.rating = Some(8.5);
        enrichment.rating_scale = Some(10);
        enrichment.review_count = 100;
        enrichment.popularity = Some(0.8);
        enrichment.popularity_source = Some(PlaceProvider::Foursquare);
        let merged = merge_foursquare(base, vec![enrichment], 10);
        assert_eq!(merged[0].popularity, Some(0.8));
        assert_eq!(merged[0].provider_score, Some(0.99));
        assert_eq!(merged[0].rating_scale, Some(10));
    }
}
