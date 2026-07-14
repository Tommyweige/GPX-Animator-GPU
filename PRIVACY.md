# Privacy notes

GPX Animator processes GPX files locally. Map tiles use the configured map
provider and are cached under `%LOCALAPPDATA%\GPX Animator\cache`.

The nearby-place browse action is offline by default. Overture and OSM snapshot
data is stored in separate SQLite files under `%LOCALAPPDATA%\GPX Animator\poi`.
If the user explicitly enables an online profile, only the clicked coordinate,
radius, result limit, and language are sent to TomTom, Foursquare, Google, or a
configured organisation gateway. Public Overpass/Nominatim endpoints are not
used by the production fallback chain.

TomTom, Foursquare, Google, and gateway secrets are stored in the Windows
Credential Manager and are never written to the application settings JSON,
logs, Git, or exported video. Nearby results remain in process memory and are
dropped when the result window is closed. Data-pack downloads are verified by
hash and (in release configuration) an Ed25519 signature before installation.

The app does not sell, profile, or transmit GPX files. Network requests can be
disabled by using only cached map tiles and not invoking the nearby lookup.
