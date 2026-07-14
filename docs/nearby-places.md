# Nearby places: data sources, profiles, and fallbacks

This feature is a read-only browse helper. Right-click a visible map location,
choose **Search nearby places**, and the native UI converts the screen point to
WGS84 before querying the selected profile. POI data is never written into a
GPX file or exported video.

## Profiles

| Profile | Cost/key | Primary source | Fallback | Rating/popularity |
| --- | --- | --- | --- | --- |
| **Offline Free** | Free; no key | Overture Taiwan snapshot, then OSM snapshot | No network fallback | No rating claim; distance ordering |
| **TomTom Live (BYOK)** | TomTom key; use your own quota | Local snapshot, then TomTom Places Search v3 | TomTom legacy Search, then local snapshot | TomTom relevance only, never called popularity |
| **Foursquare Enhanced (BYOK)** | Foursquare Premium plan/key | TomTom/local base plus Foursquare enrichment | Base results without Premium fields | Foursquare rating (0–10), rating count, popularity (0–1) |
| **Google Places (BYOK)** | Google Cloud project/key | Explicit Google Nearby Search (New) | Local snapshot only | Google rating (0–5) and user rating count |
| **Gateway Pro** | Organisation gateway/token | `/v1/nearby-places` gateway contract | TomTom/local chain | Gateway-supplied, provenance preserved |

Google is an explicit manual option; it is never silently called as a cost-
incurring fallback. Public Overpass/Nominatim endpoints are not production
fallbacks. The old Overpass client remains only as a compatibility parser for
fixtures and migration tests.

## What the numbers mean

- **Rating** is the provider's consumer rating. The UI keeps the source scale
  (`5` for Google, `10` for Foursquare) instead of pretending they are equal.
- **Rating count** is the number of ratings reported by the provider. It is not
  automatically labelled as a text-review count.
- **Popularity** is only shown when a provider supplies a documented popularity
  metric. Foursquare's value is normalised to `0..1`; TomTom `score` is a
  relevance value and is labelled as such.
- **Freshness/provenance** remains attached to each local or live result. A
  failed live request produces a visible degraded/fallback state.

## Local data packs

Overture and OSM are stored in separate SQLite files under:

```text
%LOCALAPPDATA%\GPX Animator\poi\overture-taiwan.sqlite3
%LOCALAPPDATA%\GPX Animator\poi\osm-taiwan.sqlite3
```

The first-use **Download / update offline POI data pack** action downloads the
release manifest and archives, verifies SHA-256, optionally verifies the
Ed25519 release signature, decompresses zstd archives, and atomically replaces
the SQLite file. Overture and OSM files stay separate for attribution and
licence compliance. Set `GPX_ANIMATOR_POI_MANIFEST_URL` to a private mirror;
release builds should also set `GPX_ANIMATOR_POI_PACK_PUBLIC_KEY_HEX` and keep
signature verification enabled.

The SQLite schema uses an RTree spatial index and FTS5 text index. Queries are
local, deterministic, and do not contact a public OSM endpoint. Data-pack
release metadata must include the source snapshot date, licence/NOTICE files,
coverage region, row count, compressed/uncompressed hashes, and signed version.

## Credentials and privacy

API keys/tokens are stored in the Windows per-user Credential Manager only:

- `GPX Animator/TomTom Search API Key`
- `GPX Animator/Foursquare Places API Key`
- `GPX Animator/Google Places API Key`
- `GPX Animator/POI Gateway Bearer Token`

The settings JSON stores profile, radius, online-refresh preference, and gateway
base URL, but never stores secrets. Live requests contain only the clicked
coordinate, radius, result limit, and language. Results are retained in memory
for the result window. Provider attribution and usage restrictions remain the
user's responsibility.

## Test coverage

`places-core` includes deterministic tests for:

- TomTom v3 and legacy response parsing, headers, details shape, and rate-limit
  mapping;
- Foursquare rating/count/popularity parsing and scale preservation;
- Google field-mask parsing and review-count ordering;
- Overture/OSM SQLite RTree/FTS5 queries, separate-store merge/dedupe, metadata,
  and upsert behavior;
- manifest hash checks, Ed25519 signatures, zstd extraction, atomic install,
  and tamper rejection;
- every profile's fallback matrix, including offline/no-key degradation;
- a 40-coordinate × 6-category synthetic benchmark (240 scenarios).

Live provider tests are opt-in and require user-owned keys; they are never run
in ordinary CI and their raw responses are not committed.
