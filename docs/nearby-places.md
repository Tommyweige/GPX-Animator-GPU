# Nearby places lookup (English)

This feature is a browse helper, not a route editor. A right-click in the map
preview is converted to WGS84, then the app queries nearby places and displays
the list in a fixed-position native window. No place marker or place metadata is
added to an exported video.

## Providers and ranking

1. Google Places Nearby Search (New) is the primary provider when a key exists.
2. Overpass API with OpenStreetMap tags is the free fallback when no key exists
   or Google is unavailable.
3. Results are sorted locally by `userRatingCount` descending, rating
   descending, distance ascending, and stable provider id. OSM has no review
   count, so its list is distance ordered after the Google path is unavailable.

Google requires a field mask and attribution. The app shows a Google
attribution line and opens the provider URL rather than copying Google content
into the map renderer. OSM links open the corresponding OSM object.

## Configuration

Open **Settings** and paste a Google Places API key. Saving writes it to the
Windows Credential Manager target `GPX Animator/Google Places API Key`; the
settings JSON contains only the selected radius. Remove clears the credential.
The radius choices are 500 m, 1 km, 2 km (default), and 5 km.

Google Cloud billing and Places API enablement are required for Google results;
the application does not embed a shared key. The fallback does not require a
key but is subject to the public Overpass instance's fair-use and rate limits.

## Test coverage

`native/places-core` has deterministic tests for coordinate normalization,
Haversine distance, ranking tie-breakers, Google and Overpass fixture decoding,
request normalization, missing-key behavior, and local TCP mock HTTP responses
(including rate limiting). The desktop app tests that preferences migrate
without storing an API key and that the Credential Manager UI masks secrets.
The real provider tests remain network-independent; a manual run with a
user-owned key is intentionally not part of CI.

## Privacy

Only the chosen coordinate, radius, language, and provider request are sent to
the selected service. Results are held in memory and discarded when the result
window closes. See [PRIVACY.md](../PRIVACY.md) and [TERMS.md](../TERMS.md).
