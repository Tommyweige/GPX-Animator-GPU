# POI data-pack release

1. Build separate Taiwan Overture and OSM SQLite files. Keep source licences,
   NOTICE files, snapshot dates, and transformation scripts beside the release.
2. Compress each SQLite file with zstd (`zstd -19 --long` is acceptable for
   release; the app streams decompression).
3. Compute SHA-256 for both the archive and uncompressed SQLite payload.
4. Create `poi-manifest.json` from
   [`poi-manifest.example.json`](poi-manifest.example.json). Sign each
   manifest entry's canonical payload with the release Ed25519 signing key and
   publish only the public key in the application build configuration.
5. Upload the two archives, manifest, licences, and NOTICE files to a GitHub
   Release or private mirror. Set `GPX_ANIMATOR_POI_MANIFEST_URL` only when a
   mirror is required.

The desktop app refuses a signed release when the configured public key does
not verify the manifest. Hash-only mode is available to local development
builds, not production distribution.
