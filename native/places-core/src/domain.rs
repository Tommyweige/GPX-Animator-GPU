//! Provider-independent POI domain types and fallback policy.
//!
//! This module contains only data and policy.  Network clients and the local
//! SQLite store live in sibling modules so the UI can test the fallback matrix
//! without making network calls.

use super::{NearbySearchRequest, PlaceProvider, PlaceSummary};
use serde::{Deserialize, Serialize};

/// Product profiles exposed by the settings UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoiProfile {
    /// No API key and no network.  Uses the downloaded Overture/OSM snapshot.
    #[default]
    OfflineFree,
    /// Local snapshot first, then TomTom Places Search v3 when a key exists.
    TomTomLive,
    /// Local/TomTom base results enriched with Foursquare Premium metrics.
    FoursquareEnhanced,
    /// A future organisation gateway, with local/TomTom fallback.
    GatewayPro,
    /// Explicit Google Places BYOK mode.  Google is never an implicit fallback.
    GoogleByok,
}

impl PoiProfile {
    pub const fn label(self) -> &'static str {
        match self {
            Self::OfflineFree => "Offline Free (Overture + OSM)",
            Self::TomTomLive => "TomTom Live (BYOK)",
            Self::FoursquareEnhanced => "Foursquare Enhanced (BYOK)",
            Self::GatewayPro => "Gateway Pro",
            Self::GoogleByok => "Google Places (BYOK)",
        }
    }

    pub const fn uses_network(self) -> bool {
        !matches!(self, Self::OfflineFree)
    }

    pub const fn needs_tomtom(self) -> bool {
        matches!(
            self,
            Self::TomTomLive | Self::FoursquareEnhanced | Self::GatewayPro
        )
    }

    pub const fn needs_foursquare(self) -> bool {
        matches!(self, Self::FoursquareEnhanced)
    }

    pub const fn needs_google(self) -> bool {
        matches!(self, Self::GoogleByok)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderStatus {
    Available,
    MissingApiKey,
    NotConfigured,
    Disabled(String),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityState {
    pub provider: PlaceProvider,
    pub status: ProviderStatus,
    pub capabilities: ProviderCapabilities,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub search: bool,
    pub details: bool,
    pub rating: bool,
    pub review_count: bool,
    pub popularity: bool,
    pub opening_hours: bool,
    pub local_snapshot: bool,
}

impl ProviderCapabilities {
    pub const fn local_snapshot() -> Self {
        Self {
            search: true,
            details: false,
            rating: false,
            review_count: false,
            popularity: false,
            opening_hours: false,
            local_snapshot: true,
        }
    }

    pub const fn tomtom() -> Self {
        Self {
            search: true,
            details: true,
            rating: false,
            review_count: false,
            popularity: false,
            opening_hours: true,
            local_snapshot: false,
        }
    }

    pub const fn foursquare() -> Self {
        Self {
            search: true,
            details: true,
            rating: true,
            review_count: true,
            popularity: true,
            opening_hours: true,
            local_snapshot: false,
        }
    }

    pub const fn google() -> Self {
        Self {
            search: true,
            details: true,
            rating: true,
            review_count: true,
            popularity: false,
            opening_hours: true,
            local_snapshot: false,
        }
    }

    pub const fn gateway() -> Self {
        Self {
            search: true,
            details: true,
            rating: true,
            review_count: true,
            popularity: true,
            opening_hours: true,
            local_snapshot: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCredentials {
    /// Secrets are supplied at runtime by the desktop Credential Manager
    /// adapter.  They are not persisted by this struct or the settings JSON.
    #[serde(skip_serializing, skip_deserializing)]
    pub tomtom_api_key: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub foursquare_api_key: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub google_api_key: Option<String>,
    #[serde(skip_serializing, skip_deserializing)]
    pub gateway_bearer_token: Option<String>,
    pub gateway: Option<GatewayConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    pub base_url: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FallbackOutcome {
    Succeeded { count: usize },
    Empty,
    Skipped(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackAttempt {
    pub provider: PlaceProvider,
    pub outcome: FallbackOutcome,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoiSearchResponse {
    pub profile: PoiProfile,
    pub request: NearbySearchRequest,
    pub places: Vec<PlaceSummary>,
    pub attempts: Vec<FallbackAttempt>,
    pub degraded: bool,
    pub attribution: Vec<String>,
}

impl PoiSearchResponse {
    pub fn empty(profile: PoiProfile, request: NearbySearchRequest) -> Self {
        Self {
            profile,
            request,
            places: Vec::new(),
            attempts: Vec::new(),
            degraded: false,
            attribution: Vec::new(),
        }
    }
}
