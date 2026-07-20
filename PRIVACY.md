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

## GPX Animator Ride for Android

The Android companion stores precise location fixes in its private Room
database and exports GPX files under app-private storage. Recording is local
first and continues without a network connection. Location is collected only
after the user starts a visible ride-recording workflow and while the ongoing
location foreground-service notification represents that active recording.
The app does not request Android's "all the time" background-location
permission and does not use location for advertising, profiling or analytics.

Google Drive is optional. When enabled, the app requests the narrow
`drive.file` OAuth scope and uploads completed GPX files to a visible
`GPX Animator/Routes/<year>` folder owned by the selected Google account. It
does not request access to unrelated Drive files. Access tokens are supplied
by Google Play services, are never written to the route database or logs, and
can be revoked from the app. Disconnecting Drive leaves the local GPX intact.
For a large file, the resumable-upload session URL, content hash and byte length
are temporarily stored in the app-private database so upload can continue after
a network or process interruption. This metadata is excluded from backup and is
cleared after success or when Drive is disconnected.

Android ride records and GPX files are excluded from Android cloud backup and
device-transfer backup. A user must explicitly delete a local ride or its Drive
copy; deleting one does not silently delete the other.
