# Route landmarks and Relive-style moments

This document describes the route-place workflow implemented by the native
GPX Animator application. It is intentionally English-first so the same
contract can be used by the UI, export pipeline, and release/CI checks.

## User workflow

1. Load a GPX track and right-click a visible location in the aspect-correct
   preview.
2. Choose **Search nearby places**. The context menu converts the clicked
   screen point back to WGS84 and opens a fixed results window.
3. Select **Add to route** for Overture/OSM, or **Match & add** for a
   TomTom/Foursquare result. Live provider rows are accepted only when a
   same-name Overture/OSM row is found within 100 m; this prevents a paid
   provider object from being silently copied into the project.
4. If there is no open-data match, choose **Add custom route marker** from the
   map context menu, enter a name/category, and confirm. This is the supported
   fallback for Google, Gateway, and private/unmapped locations.
5. Use **Route places** in the left panel to preview a marker, edit its name
   or category, disable it, or remove it. Changes are saved immediately.

When the route reaches the same place more than once, the app projects the
place onto the GPX in route order and groups nearby segments into distinct
passes. A unique pass is added immediately. If there are multiple passes, the
current timeline position is used only as the initial selection and the user
must confirm the intended pass. Opposite-direction pairs are labelled
outbound/return; complex loops use pass numbers and headings instead of
guessing those semantics.

## Data and persistence contract

`scene-core` owns the provider-neutral `RouteLandmark` model:

- `latitude`/`longitude` remain the real POI coordinate and are never snapped;
- `anchor_distance_m` and `anchor_progress` identify the selected route pass
  and control activation timing;
- `anchor_mode` records whether the pass was explicitly selected, so saving or
  reloading a project cannot silently fall back to a spatially nearest pass;
- `distance_from_route_m` is retained for user review;
- `LandmarkSource` and `source_id` preserve provenance;
- `LandmarkStyle` stores the pin colour and label visibility.

The source GPX is immutable. A project is stored as
`<route>.gpxanimator.json` with schema version 2, SHA-256 of the GPX bytes, and
the landmark list. Writes are atomic (`.json.tmp` then rename). If the route
folder is read-only, the same payload is stored under
`%LOCALAPPDATA%\GPX Animator\projects\<sha256>.gpxanimator.json`. A corrupt
sidecar is renamed with a `.corrupt-<timestamp>.json` suffix before the app
continues with an empty list. Loading a project re-matches the saved pass
against the current track and warns when the GPX hash changed. Schema-v1
projects are migrated using their stored anchor progress; landmarks that no
longer match the route are disabled with a visible warning.

## Reveal and rendering contract

Each enabled marker is included in Fit-camera bounds so the final route view
contains every selected place. The marker's real map coordinate is projected
independently of the route marker. At its anchor time:

- the pin reveals over 0.65 s with an ease-out-back overshoot;
- a stem, soft offset shadow, and pulse ring create the raised/Relive-like
  appearance;
- the label fades/expands in after 0.10 s, stays readable for about 2.2 s,
  then fades; the pin remains visible until the end;
- the existing Follow-to-Fit transition remains a single smooth camera blend;
  no per-place pause, duplicate frame, or black transition is introduced.

The preview draws the same `LandmarkFrame` data with egui, while the official
export draws the vector geometry in Direct2D on the D3D11 render target. No
bitmap marker assets, CPU readbacks, browser canvas, or FFmpeg intermediate are
used.

## Provider and licensing policy

- Overture/OSM rows are the only provider results persisted directly.
- TomTom/Foursquare rows may enrich discovery, but must resolve to an open-data
  match or be converted to a manual marker by the user.
- Google and Gateway rows are view-only in this feature; the manual marker is
  the licence-safe export path.
- Attribution stays with the map/POI provider according to the existing
  `docs/nearby-places.md` and release-pack notices.

## Test matrix

The implementation is covered by deterministic tests in the normal workspace
suite:

- `scene-core`: route projection, repeated-pass detection for out-and-back
  routes, heading labels, dateline-safe nearest-segment anchoring, reveal
  timing, persistent pins, Fit bounds, camera blending, and all aspect ratios;
- `desktop-app::project`: sidecar round-trip, GPX hash re-anchor, atomic write,
  read-only AppData fallback, and corrupt-file backup;
- UI/export: exact frame/timestamp tests, cancellation cleanup, settings and
  state transitions, plus the existing GPU/NVENC and MP4 suites;
- D3D11: vector marker/HUD/elevation render smoke test on the RTX device.

Run the complete checks from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --tests --examples -- -D warnings
cargo test --workspace --all-targets --no-fail-fast
```

The ignored RTX gates remain required for release acceptance: warm-cache 4K60,
five-minute exact-frame output, ten-export leak stress, zero CPU readback, and
NVIDIA-only adapter verification.
