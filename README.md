# GPX Animator GPU Edition

Native Windows GPX route animation with a GPU-first 4K60 export pipeline. The
production path is **D3D11/Direct2D → NVIDIA RTX texture → NVENC HEVC/H.264 →
MP4**. Frames remain on the GPU until the encoded bitstream is handed to the
muxer; the release executable does not require a browser, Node.js, WebView or
an FFmpeg runtime.

This README is English-first. A Traditional Chinese guide is provided in
[繁體中文](#繁體中文).

## Features

- Native Rust workspace with an `egui` desktop UI.
- Explicit NVIDIA RTX adapter selection. Intel, AMD, Microsoft WARP and CPU
  fallbacks are rejected for export instead of silently reducing performance.
- Six-texture D3D11 ring with asynchronous NVENC submission and zero CPU frame
  readbacks (`cpu_frame_readbacks == 0`).
- H.265/HEVC Main 8-bit (`hvc1`) or H.264 (`avc1`) in MP4, fixed timestamps,
  fast-start `moov`, and BT.709 metadata.
- GPX 1.0/1.1, multiple segments, malformed-point validation, stationary/red-
  light filtering, distance-uniform sampling, elevation smoothing and route
  statistics.
- Satellite, light, dark and transparent map styles. Dark uses a real OSM
  basemap with a deterministic dark color transform; Transparent uses the same
  real map at 35% opacity over a neutral background (MP4 has no alpha channel).
  Tiles are stored in a persistent disk LRU cache and missing network tiles use
  a parent/placeholder fallback instead of black blocks.
- Follow and Fit export cameras. Follow uses a real Web Mercator zoom level;
  preview wheel zoom keeps Follow mode and preview dragging is temporary.
  Export never uses the old Free camera mode.
- 16:9, 1:1 and 9:16 output, aspect-correct camera spans, drag/scroll preview,
  HUD, elevation profile and 8 px route width by default.
- English and Traditional Chinese UI with persisted preferences. Static HUD
  labels in the exported MP4 follow the selected UI language; provider names
  and addresses remain in the source language.
- Right-docked nearby places inspector. Right-click the map, preview a result
  as a large blue candidate pin, then add it as a selected coral pin without
  changing timeline progress. Added pins use the same renderer in preview and
  export. Settings is a fixed, bilingual window with General, Places, API keys,
  Storage and Advanced sections.
- Cancellation, progress reporting, GPU diagnostics and deterministic offline
  test fixtures.

## Download and run

The packaged Windows build is:

[GPX-Animator-GPU-20260713.exe](dist/GPX-Animator-GPU-20260713.exe)

```powershell
.\dist\GPX-Animator-GPU-20260713.exe "D:\path\to\route.gpx"
```

Close an existing GPX Animator process before replacing the executable. The
default profile is 3840×2160 at 60 FPS, HEVC High quality, satellite imagery
and an 8 px route; every setting is editable in the left panel.

## Requirements and GPU policy

- Windows 10 or Windows 11 x64.
- NVIDIA RTX GPU with a driver exposing NVENC HEVC/H.264.
- Writable local profile directory. Internet is needed only for tiles that are
  not already cached.

The app deliberately does not fall back to Intel QSV, AMD, software encoding
or Microsoft WARP. The GPU Diagnostics panel reports the adapter name/LUID,
dedicated VRAM, driver/API versions, NVENC capabilities, ring occupancy,
render/encode/mux timings, dropped/duplicated frames and CPU readbacks.

## Map cache and offline use

Tiles are fetched and decoded before rendering, uploaded once to the RTX
device, and reused on later exports:

```text
%LOCALAPPDATA%\GPX Animator\cache
```

Settings exposes the cache limit and a clear-cache action. Permission errors,
timeouts and offline runs degrade to a cached parent tile or deterministic
placeholder so a transient network failure cannot corrupt the export.

## Nearby places and route pins

1. Right-click inside the aspect-correct preview and choose **Search nearby
   places**.
2. The fixed right-hand inspector lists nearby results with provider, distance,
   address and map link. The panel never covers the map and is resizable.
3. **Preview** draws a temporary blue candidate pin. **Add pin** writes the
   selected WGS84 coordinate to the route sidecar and does not alter the
   animation timeline or current progress. If the route passes the place more
   than once, the current timeline position preselects a pass and the app asks
   you to confirm the outbound/return (or numbered) occurrence.
4. **Add custom route marker** opens the same right-hand inspector with a name,
   category, and the shared 500 m–5 km nearby-search radius. A compatible
   nearby result can be used with its real provider data; otherwise the
   clicked coordinate can be saved as a manual pin.
5. Added pins are revealed when the route reaches the selected pass and remain
   visible in the final Fit view. They use a cream/coral teardrop body, stem,
   soft pulse and localized category label in both preview and export.

Offline Free (Overture + OSM) is the default profile. Optional BYOK profiles
support TomTom Places Search, Foursquare, Google Places and a Gateway contract.
Keys live in Windows Credential Manager and never enter settings, logs, Git or
MP4 metadata. See [docs/nearby-places.md](docs/nearby-places.md) and
[docs/route-landmarks.md](docs/route-landmarks.md).

## Export pipeline

1. Parse GPX, remove stationary dwell, smooth elevation and build a uniform
   distance timeline.
2. Build a complete tile manifest for every route/follow/fit frame, load the
   local cache, fetch missing tiles and upload a usable zoom level.
3. Render into the D3D11 texture ring while NVENC encodes earlier textures and
   the MP4 muxer writes completed packets.
4. Finalize atomically, move `moov` before `mdat`, and verify dimensions, frame
   rate, codec tag, sample count and color metadata.

The default HEVC contract is Main 8-bit, VBR, NVENC P5/CQ19, two B-frames,
GOP 120, adaptive quantization and no lookahead. Balanced and Speed presets map
to P4/CQ22 and P3/CQ25 respectively. Stop/red-light dwell is intentionally
removed; the remaining route is sampled at a smooth uniform pace.

## Build from source

Install Rust 1.92 or newer with the MSVC toolchain:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --no-fail-fast
cargo build --release -p gpx-animator-native
```

The release executable is written to `target\release\gpx-animator-native.exe`.
The NVIDIA Video Codec SDK headers are used at build time; the runtime loads
the driver-provided NVENC library dynamically.

## Test and acceptance gates

The workspace tests cover GPX parsing/filtering, the 0706 route golden result,
camera math, aspect-correct tile queries, D3D11 resource lifetime, NVENC state
and cancellation, HEVC `hvcC`/`hvc1` packaging, UI state transitions, POI
provider parsing and preferences. On an RTX Windows runner, run every ignored
hardware gate as well:

```powershell
cargo test --release -p gpx-animator-native warm_cache_twenty_second_4k60_meets_realtime_gate -- --ignored --nocapture
cargo test --release -p gpx-animator-native five_minute_4k60_has_exact_frames_and_realtime_throughput -- --ignored --nocapture
cargo test --release -p gpx-animator-native ten_exports_do_not_leak_handles_or_partial_files -- --ignored --nocapture
```

These verify exact 4K60 frame counts, HEVC `hvc1`, real-time throughput after a
warm cache, p95 render time, zero CPU readback, cancellation cleanup and stable
handle/VRAM usage. A release build should only be published after all three
gates pass.

## Repository layout

```text
native/gpx-core          GPX parsing, filtering, elevation and sampling
native/scene-core        Scene description, camera and layout math
native/d3d11-renderer    D3D11/Direct2D renderer, tiles and route pins
native/nvenc-engine      RTX texture ring and NVENC session
native/mp4-output        HEVC/H.264 sample conversion and MP4 muxing
native/desktop-app       egui UI, inspector and export state machine
native/places-core       Offline Overture/OSM and optional provider clients
legacy-web/              Previous browser prototype (not the release path)
packaging/               Windows packaging helpers
dist/                    Packaged executable
```

## Troubleshooting

- **No compatible GPU:** update the NVIDIA driver and inspect GPU Diagnostics.
  Intel/WARP/CPU fallback is intentionally disabled.
- **Preload is slow:** the first export downloads missing tiles; later exports
  use the AppData cache. Light, Dark and Transparent share the OSM cache, while
  Satellite has its own cache. The preflight counter separates cache and network
  work.
- **Tile access denied/offline:** check firewall/proxy permissions or use a
  warmed cache. Parent-tile/placeholder fallback prevents black tile blocks.
- **Cannot overwrite the EXE:** close every GPX Animator process first.

## Attribution and licence

Map data: [OpenStreetMap contributors](https://www.openstreetmap.org/copyright).
Satellite imagery: Esri, Maxar and Earthstar Geographics. See [LICENSE](LICENSE).

---

## 繁體中文

GPX Animator GPU Edition 是 Windows 原生 Rust 路線動畫工具。正式輸出流程為
**D3D11/Direct2D → NVIDIA RTX 紋理 → NVENC HEVC/H.264 → MP4**，影格在 GPU
上完成直到交給封裝器，不需要瀏覽器、Node.js、WebView 或 FFmpeg runtime。

### 主要功能

- 僅選擇 NVIDIA RTX；Intel、AMD、WARP 與 CPU fallback 會被拒絕。
- 六張 D3D11 紋理環非同步編碼，正式流程 `cpu_frame_readbacks == 0`。
- HEVC `hvc1`／H.264 `avc1` MP4、固定時間戳、快速啟動 `moov`。
- GPX 多格式、多段、停留／紅綠燈過濾、距離勻速取樣與海拔降噪。
- 衛星、明亮、深色、淡化地圖；深色是真實 OSM 地圖的深色轉換，淡化地圖仍
  有真實 OSM 圖磚並以 35% 透明度疊在中性背景上（MP4 不支援 Alpha）。地圖
  磚會保存到本機 LRU cache，失敗時使用父層磚或可重現的 placeholder，避免
  黑色方塊。
- 只保留「跟隨」與「完整」輸出視角。跟隨視角使用真正 Web Mercator zoom，
  預覽滾輪不會切成自由視角；拖曳只改暫時預覽。
- 16:9、1:1、9:16、HUD、海拔圖，以及預設 8 px 路線。
- 右鍵地圖開啟固定右側的附近地點面板；可搜尋附近地點，或在同一面板填寫
  自訂圖針並共用 500 m–5 km 搜尋半徑。先預覽大型藍色候選圖針，再加入
  橘紅色選取圖針，不會改變路線進度。圖針在預覽與輸出使用同一個水滴樣式。
- 選擇 English 後，影片 HUD 固定文字同步使用英文；Settings 以固定視窗和
  左側分類導覽整理語言、POI、API keys、快取與進階診斷。

### 使用與測試

執行 `dist\GPX-Animator-GPU-20260713.exe` 後載入 GPX。第一次輸出會預載未
快取的地圖磚，之後重用 `%LOCALAPPDATA%\GPX Animator\cache`。在「GPU 診斷」
可查看實際 adapter、NVENC、render/encode/mux 時間與 CPU readback 次數。

完整測試命令與 RTX 實機 gate 請參考上方英文章節；所有一般測試及三個 ignored
硬體 gate 都必須通過才可發布新版 EXE。
