//! Offline Overture/OSM POI stores.
//!
//! Overture and OSM are intentionally kept in separate SQLite files.  This
//! makes attribution and license handling explicit and prevents a future
//! update from accidentally replacing one source with another.  The stores
//! use SQLite RTree for spatial candidates and FTS5 for optional text search;
//! nearby lookup itself remains deterministic and has no network dependency.

use crate::{
    NearbySearchRequest, PlaceProvider, PlaceSummary, PlacesError, SearchCoordinate, distance_m,
};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalDataset {
    Overture,
    OpenStreetMap,
}

impl LocalDataset {
    pub const fn provider(self) -> PlaceProvider {
        match self {
            Self::Overture => PlaceProvider::Overture,
            Self::OpenStreetMap => PlaceProvider::OpenStreetMap,
        }
    }

    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Overture => "overture-taiwan.sqlite3",
            Self::OpenStreetMap => "osm-taiwan.sqlite3",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreStats {
    pub dataset: LocalDataset,
    pub path: PathBuf,
    pub place_count: u64,
    pub schema_version: u32,
    pub data_pack_version: Option<String>,
    pub refreshed_at: Option<String>,
}

#[derive(Clone)]
pub struct LocalPoiStore {
    connection: Arc<Mutex<Connection>>,
    dataset: LocalDataset,
    path: PathBuf,
}

impl LocalPoiStore {
    pub fn open(path: impl Into<PathBuf>, dataset: LocalDataset) -> Result<Self, PlacesError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| PlacesError::Storage(error.to_string()))?;
        }
        let connection = Connection::open(&path).map_err(storage_error)?;
        initialise(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            dataset,
            path,
        })
    }

    pub fn open_in_memory(dataset: LocalDataset) -> Result<Self, PlacesError> {
        let connection = Connection::open_in_memory().map_err(storage_error)?;
        initialise(&connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            dataset,
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn dataset(&self) -> LocalDataset {
        self.dataset
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn replace_all(
        &self,
        places: &[PlaceSummary],
        data_pack_version: Option<&str>,
        refreshed_at: Option<&str>,
    ) -> Result<(), PlacesError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| PlacesError::Storage("store mutex poisoned".into()))?;
        let transaction = connection.transaction().map_err(storage_error)?;
        transaction
            .execute("DELETE FROM places", [])
            .map_err(storage_error)?;
        transaction
            .execute("DELETE FROM places_rtree", [])
            .map_err(storage_error)?;
        transaction
            .execute("DELETE FROM places_fts", [])
            .map_err(storage_error)?;
        for place in places {
            insert_place(&transaction, place, self.dataset)?;
        }
        transaction
            .execute(
                "INSERT INTO store_meta(key,value) VALUES ('data_pack_version',?1),('refreshed_at',?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![data_pack_version, refreshed_at],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)
    }

    pub fn upsert(&self, places: &[PlaceSummary]) -> Result<(), PlacesError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| PlacesError::Storage("store mutex poisoned".into()))?;
        let transaction = connection.transaction().map_err(storage_error)?;
        for place in places {
            insert_place(&transaction, place, self.dataset)?;
        }
        transaction.commit().map_err(storage_error)
    }

    pub fn search(&self, request: NearbySearchRequest) -> Result<Vec<PlaceSummary>, PlacesError> {
        let request = request.normalized();
        let origin = request.coordinate;
        let radius = request.radius_m as f64;
        let lat_delta = radius / 111_320.0;
        let lon_delta = radius / (111_320.0 * origin.latitude.to_radians().cos().abs().max(0.1));
        let min_lat = origin.latitude - lat_delta;
        let max_lat = origin.latitude + lat_delta;
        let min_lon = origin.longitude - lon_delta;
        let max_lon = origin.longitude + lon_delta;
        let connection = self
            .connection
            .lock()
            .map_err(|_| PlacesError::Storage("store mutex poisoned".into()))?;
        let mut statement = connection
            .prepare(
                "SELECT p.provider,p.id,p.name,p.category,p.address,p.latitude,p.longitude,
                        p.rating,p.rating_scale,p.review_count,p.open_now,p.provider_score,
                        p.popularity,p.popularity_source,p.source_updated_at,p.phone,p.website,p.external_url
                   FROM places_rtree r
                   JOIN places p ON p.rowid=r.id
                  WHERE r.min_lat <= ?1 AND r.max_lat >= ?2
                    AND r.min_lon <= ?3 AND r.max_lon >= ?4",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(params![max_lat, min_lat, max_lon, min_lon], |row| {
                decode_place_row(row, origin)
            })
            .map_err(storage_error)?;
        let mut places = Vec::new();
        for row in rows {
            let place = row.map_err(storage_error)?;
            if place.distance_m <= radius {
                places.push(place);
            }
        }
        places.sort_by(|a, b| {
            a.distance_m
                .partial_cmp(&b.distance_m)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        places.truncate(request.limit as usize);
        Ok(places)
    }

    pub fn count(&self) -> Result<u64, PlacesError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| PlacesError::Storage("store mutex poisoned".into()))?;
        connection
            .query_row("SELECT COUNT(*) FROM places", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(storage_error)
    }

    /// Optional text-constrained nearby search backed by the FTS5 index.
    /// Callers can pass an empty query to use the spatial-only path.
    pub fn search_text(
        &self,
        query: &str,
        request: NearbySearchRequest,
    ) -> Result<Vec<PlaceSummary>, PlacesError> {
        if query.trim().is_empty() {
            return self.search(request);
        }
        let request = request.normalized();
        let origin = request.coordinate;
        let radius = request.radius_m as f64;
        let lat_delta = radius / 111_320.0;
        let lon_delta = radius / (111_320.0 * origin.latitude.to_radians().cos().abs().max(0.1));
        let connection = self
            .connection
            .lock()
            .map_err(|_| PlacesError::Storage("store mutex poisoned".into()))?;
        let mut statement = connection
            .prepare(
                "SELECT p.provider,p.id,p.name,p.category,p.address,p.latitude,p.longitude,
                        p.rating,p.rating_scale,p.review_count,p.open_now,p.provider_score,
                        p.popularity,p.popularity_source,p.source_updated_at,p.phone,p.website,p.external_url
                   FROM places_fts f
                   JOIN places p ON p.rowid=f.rowid
                   JOIN places_rtree r ON r.id=f.rowid
                  WHERE places_fts MATCH ?1
                    AND r.min_lat <= ?2 AND r.max_lat >= ?3
                    AND r.min_lon <= ?4 AND r.max_lon >= ?5",
            )
            .map_err(storage_error)?;
        let rows = statement
            .query_map(
                params![
                    query,
                    origin.latitude + lat_delta,
                    origin.latitude - lat_delta,
                    origin.longitude + lon_delta,
                    origin.longitude - lon_delta
                ],
                |row| decode_place_row(row, origin),
            )
            .map_err(storage_error)?;
        let mut places = Vec::new();
        for row in rows {
            let place = row.map_err(storage_error)?;
            if place.distance_m <= radius {
                places.push(place);
            }
        }
        places.sort_by(|a, b| {
            a.distance_m
                .partial_cmp(&b.distance_m)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        places.truncate(request.limit as usize);
        Ok(places)
    }

    pub fn stats(&self) -> Result<StoreStats, PlacesError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| PlacesError::Storage("store mutex poisoned".into()))?;
        let place_count = connection
            .query_row("SELECT COUNT(*) FROM places", [], |row| {
                row.get::<_, u64>(0)
            })
            .map_err(storage_error)?;
        let data_pack_version = connection
            .query_row(
                "SELECT value FROM store_meta WHERE key='data_pack_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        let refreshed_at = connection
            .query_row(
                "SELECT value FROM store_meta WHERE key='refreshed_at'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(storage_error)?;
        Ok(StoreStats {
            dataset: self.dataset,
            path: self.path.clone(),
            place_count,
            schema_version: 1,
            data_pack_version,
            refreshed_at,
        })
    }
}

/// Two separate local stores are merged at query time.  Overture wins an
/// exact duplicate because it normally carries richer place attributes; OSM
/// remains a valuable free fallback for coverage gaps.
#[derive(Clone, Default)]
pub struct LocalPoiCatalog {
    pub overture: Option<LocalPoiStore>,
    pub osm: Option<LocalPoiStore>,
}

impl LocalPoiCatalog {
    pub fn from_app_data(root: impl AsRef<Path>) -> Result<Self, PlacesError> {
        let root = root.as_ref();
        Ok(Self {
            overture: Some(LocalPoiStore::open(
                root.join(LocalDataset::Overture.file_name()),
                LocalDataset::Overture,
            )?),
            osm: Some(LocalPoiStore::open(
                root.join(LocalDataset::OpenStreetMap.file_name()),
                LocalDataset::OpenStreetMap,
            )?),
        })
    }

    pub fn search(&self, request: NearbySearchRequest) -> Result<Vec<PlaceSummary>, PlacesError> {
        let mut merged = Vec::new();
        if let Some(store) = &self.overture {
            merged.extend(store.search(request.clone())?);
        }
        if let Some(store) = &self.osm {
            merged.extend(store.search(request.clone())?);
        }
        dedupe_places(&mut merged);
        merged.sort_by(|a, b| {
            a.distance_m
                .partial_cmp(&b.distance_m)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.id.cmp(&b.id))
        });
        merged.truncate(request.limit as usize);
        Ok(merged)
    }

    pub fn has_any_data(&self) -> Result<bool, PlacesError> {
        Ok(self
            .overture
            .as_ref()
            .map(|store| store.count().unwrap_or_default() > 0)
            .unwrap_or(false)
            || self
                .osm
                .as_ref()
                .map(|store| store.count().unwrap_or_default() > 0)
                .unwrap_or(false))
    }
}

fn initialise(connection: &Connection) -> Result<(), PlacesError> {
    connection
        .execute_batch(
            "PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS store_meta (
                 key TEXT PRIMARY KEY NOT NULL,
                 value TEXT
             );
             CREATE TABLE IF NOT EXISTS places (
                 provider TEXT NOT NULL,
                 id TEXT NOT NULL,
                 name TEXT NOT NULL,
                 category TEXT,
                 address TEXT,
                 latitude REAL NOT NULL,
                 longitude REAL NOT NULL,
                 rating REAL,
                 rating_scale INTEGER,
                 review_count INTEGER NOT NULL DEFAULT 0,
                 open_now INTEGER,
                 provider_score REAL,
                 popularity REAL,
                 popularity_source TEXT,
                 source_updated_at TEXT,
                 phone TEXT,
                 website TEXT,
                 external_url TEXT NOT NULL,
                 UNIQUE(provider,id)
             );
             CREATE INDEX IF NOT EXISTS places_name_idx ON places(name);
             CREATE VIRTUAL TABLE IF NOT EXISTS places_rtree USING rtree(
                 id,
                 min_lat,max_lat,
                 min_lon,max_lon
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS places_fts USING fts5(
                 id UNINDEXED,
                 name,
                 category,
                 address
             );",
        )
        .map_err(storage_error)
}

fn insert_place(
    transaction: &rusqlite::Transaction<'_>,
    place: &PlaceSummary,
    dataset: LocalDataset,
) -> Result<(), PlacesError> {
    let provider = dataset.provider();
    transaction
        .execute(
            "INSERT INTO places(
                provider,id,name,category,address,latitude,longitude,rating,rating_scale,
                review_count,open_now,provider_score,popularity,popularity_source,
                source_updated_at,phone,website,external_url
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
             ON CONFLICT(provider,id) DO UPDATE SET
                name=excluded.name, category=excluded.category, address=excluded.address,
                latitude=excluded.latitude, longitude=excluded.longitude, rating=excluded.rating,
                rating_scale=excluded.rating_scale, review_count=excluded.review_count,
                open_now=excluded.open_now, provider_score=excluded.provider_score,
                popularity=excluded.popularity, popularity_source=excluded.popularity_source,
                source_updated_at=excluded.source_updated_at, phone=excluded.phone,
                website=excluded.website, external_url=excluded.external_url",
            params![
                provider_name(provider),
                place.id,
                place.name,
                place.category,
                place.address,
                place.latitude,
                place.longitude,
                place.rating,
                place.rating_scale,
                i64::from(place.review_count),
                place.open_now.map(bool_to_i64),
                place.provider_score,
                place.popularity,
                place.popularity_source.map(provider_name),
                place.source_updated_at,
                place.phone,
                place.website,
                place.external_url,
            ],
        )
        .map_err(storage_error)?;
    let rowid = transaction
        .query_row(
            "SELECT rowid FROM places WHERE provider=?1 AND id=?2",
            params![provider_name(provider), place.id],
            |row| row.get::<_, i64>(0),
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO places_rtree(id,min_lat,max_lat,min_lon,max_lon) VALUES(?1,?2,?2,?3,?3)",
            params![rowid, place.latitude, place.longitude],
        )
        .map_err(storage_error)?;
    transaction
        .execute(
            "INSERT OR REPLACE INTO places_fts(rowid,id,name,category,address) VALUES(?1,?2,?3,?4,?5)",
            params![rowid, place.id, place.name, place.category, place.address],
        )
        .map_err(storage_error)?;
    Ok(())
}

fn decode_place_row(
    row: &rusqlite::Row<'_>,
    origin: SearchCoordinate,
) -> rusqlite::Result<PlaceSummary> {
    let provider: String = row.get(0)?;
    let latitude: f64 = row.get(5)?;
    let longitude: f64 = row.get(6)?;
    let popularity_source = row
        .get::<_, Option<String>>(13)?
        .as_deref()
        .and_then(parse_provider);
    Ok(PlaceSummary {
        provider: parse_provider(&provider).unwrap_or(PlaceProvider::OpenStreetMap),
        id: row.get(1)?,
        name: row.get(2)?,
        category: row.get(3)?,
        address: row.get(4)?,
        latitude,
        longitude,
        distance_m: distance_m(
            origin,
            SearchCoordinate {
                latitude,
                longitude,
            },
        ),
        rating: row.get(7)?,
        rating_scale: row.get(8)?,
        review_count: row.get::<_, i64>(9)?.max(0) as u32,
        open_now: row.get::<_, Option<i64>>(10)?.map(|value| value != 0),
        provider_score: row.get(11)?,
        popularity: row.get(12)?,
        popularity_source,
        source_updated_at: row.get(14)?,
        phone: row.get(15)?,
        website: row.get(16)?,
        external_url: row.get(17)?,
    })
}

fn dedupe_places(places: &mut Vec<PlaceSummary>) {
    places.sort_by(|a, b| {
        provider_priority(a.provider)
            .cmp(&provider_priority(b.provider))
            .then_with(|| {
                a.distance_m
                    .partial_cmp(&b.distance_m)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let mut seen = std::collections::HashSet::new();
    places.retain(|place| {
        let normalized = place.name.trim().to_lowercase();
        let key = format!(
            "{}:{}:{}",
            normalized,
            (place.latitude * 10_000.0).round() as i64,
            (place.longitude * 10_000.0).round() as i64
        );
        seen.insert(key)
    });
}

fn provider_priority(provider: PlaceProvider) -> u8 {
    match provider {
        PlaceProvider::Overture => 0,
        PlaceProvider::OpenStreetMap => 1,
        _ => 2,
    }
}

fn provider_name(provider: PlaceProvider) -> &'static str {
    match provider {
        PlaceProvider::Google => "google",
        PlaceProvider::TomTom => "tomtom",
        PlaceProvider::OpenStreetMap => "osm",
        PlaceProvider::Overture => "overture",
        PlaceProvider::Foursquare => "foursquare",
        PlaceProvider::Gateway => "gateway",
    }
}

fn parse_provider(value: &str) -> Option<PlaceProvider> {
    Some(match value {
        "google" => PlaceProvider::Google,
        "tomtom" => PlaceProvider::TomTom,
        "osm" => PlaceProvider::OpenStreetMap,
        "overture" => PlaceProvider::Overture,
        "foursquare" => PlaceProvider::Foursquare,
        "gateway" => PlaceProvider::Gateway,
        _ => return None,
    })
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

fn storage_error(error: rusqlite::Error) -> PlacesError {
    PlacesError::Storage(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_place(id: &str, name: &str, latitude: f64, longitude: f64) -> PlaceSummary {
        PlaceSummary {
            provider: PlaceProvider::Overture,
            id: id.into(),
            name: name.into(),
            category: Some("cafe".into()),
            address: Some("Taipei".into()),
            latitude,
            longitude,
            distance_m: 0.0,
            rating: None,
            review_count: 0,
            open_now: None,
            provider_score: None,
            phone: None,
            website: None,
            external_url: "https://example.test".into(),
            rating_scale: None,
            popularity: None,
            popularity_source: None,
            source_updated_at: Some("2026-07-14T00:00:00Z".into()),
        }
    }

    #[test]
    fn sqlite_rtree_search_and_stats_are_deterministic() {
        let store = LocalPoiStore::open_in_memory(LocalDataset::Overture).unwrap();
        store
            .replace_all(
                &[
                    local_place("near", "Near Cafe", 25.0, 121.0),
                    local_place("far", "Far Cafe", 25.2, 121.2),
                ],
                Some("2026.07.1"),
                Some("2026-07-14T00:00:00Z"),
            )
            .unwrap();
        let places = store
            .search(NearbySearchRequest {
                coordinate: SearchCoordinate {
                    latitude: 25.0,
                    longitude: 121.0,
                },
                radius_m: 500,
                limit: 10,
                language: crate::PlaceLanguage::English,
            })
            .unwrap();
        assert_eq!(
            places.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            ["near"]
        );
        let text_places = store
            .search_text(
                "Near",
                NearbySearchRequest {
                    coordinate: SearchCoordinate {
                        latitude: 25.0,
                        longitude: 121.0,
                    },
                    radius_m: 500,
                    limit: 10,
                    language: crate::PlaceLanguage::English,
                },
            )
            .unwrap();
        assert_eq!(text_places[0].name, "Near Cafe");
        let stats = store.stats().unwrap();
        assert_eq!(stats.place_count, 2);
        assert_eq!(stats.data_pack_version.as_deref(), Some("2026.07.1"));
    }

    #[test]
    fn catalog_prefers_overture_for_duplicate_coordinates() {
        let overture = LocalPoiStore::open_in_memory(LocalDataset::Overture).unwrap();
        let osm = LocalPoiStore::open_in_memory(LocalDataset::OpenStreetMap).unwrap();
        let mut osm_place = local_place("osm-1", "Same Cafe", 25.0, 121.0);
        osm_place.provider = PlaceProvider::OpenStreetMap;
        overture
            .upsert(&[local_place("ov-1", "Same Cafe", 25.0, 121.0)])
            .unwrap();
        osm.upsert(&[osm_place]).unwrap();
        let catalog = LocalPoiCatalog {
            overture: Some(overture),
            osm: Some(osm),
        };
        let places = catalog
            .search(NearbySearchRequest {
                coordinate: SearchCoordinate {
                    latitude: 25.0,
                    longitude: 121.0,
                },
                radius_m: 500,
                limit: 10,
                language: crate::PlaceLanguage::English,
            })
            .unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].provider, PlaceProvider::Overture);
    }
}
