# Privacy notes

GPX Animator processes GPX files locally. Map tiles use the configured map
provider and are cached under `%LOCALAPPDATA%\GPX Animator\cache`.

The optional nearby-place browse action sends the right-click coordinate,
radius, and language to Google Places or Overpass/OpenStreetMap. Google API
keys are stored in the Windows Credential Manager and are never written to the
application settings JSON, logs, Git, or exported video. Nearby results remain
in process memory and are dropped when the result window is closed.

The app does not sell, profile, or transmit GPX files. Network requests can be
disabled by using only cached map tiles and not invoking the nearby lookup.
