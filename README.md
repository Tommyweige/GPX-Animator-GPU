# GPX Animator GPU Edition

Native Windows GPX route animation with a GPU-first 4K60 export pipeline.
The production path is **D3D11/Direct2D → NVIDIA RTX texture → NVENC → MP4**:
frames stay on the GPU until the encoded bitstream is handed to the MP4 muxer.
The released application does not require a browser, Node.js, WebView, or an
FFmpeg runtime.

This README is English-first. A complete Traditional Chinese version is
included below in [繁體中文](#繁體中文).

## Highlights

- Native Rust workspace with an `egui` desktop UI.
- Explicit NVIDIA adapter selection: Intel, AMD, Microsoft WARP, and CPU
  fallback are rejected for export instead of silently reducing performance.
- D3D11 texture ring and asynchronous NVENC submission; the export path has
  zero CPU frame readbacks.
- H.265/HEVC Main 8-bit (`hvc1`) or H.264 (`avc1`) in MP4, with fixed 60/1
  timestamps, BT.709 color metadata, and fast-start `moov` placement.
- GPX 1.0/1.1 parsing, multi-segment tracks, stationary/red-light filtering,
  distance-based uniform sampling, elevation smoothing, and exact route
  statistics.
- Satellite, dark, light, and transparent map styles with a persistent disk
  tile cache and offline parent-tile/placeholder fallback.
- Follow, fit, and free camera modes; the follow camera eases into the full
  route view at the end instead of cutting to a black frame.
- 16:9, 1:1, and 9:16 layouts; draggable/scrollable preview; HUD and elevation
  profile overlays; 8 px route width by default.
- English and Traditional Chinese UI, persisted preferences, resizable panels,
  cancellation, progress reporting, and a GPU diagnostics view.

## Download and run

The latest checked-in Windows build is:

[GPX-Animator-GPU-20260713-hud8.exe](dist/GPX-Animator-GPU-20260713-hud8.exe)

Download the executable from the repository, then open a GPX file by using the
file picker, dragging it onto the window, or passing it on the command line:

```powershell
.\dist\GPX-Animator-GPU-20260713-hud8.exe "D:\path\to\route.gpx"
```

The default export profile is 3840×2160 at 60 FPS, H.265, High quality
(NVENC P5/CQ19), Satellite imagery, and an 8 px route. All of these can be
changed in the left settings panel. If an older EXE is still running, close it
before replacing that file; Windows keeps an executable locked while its
process is alive.

## Requirements and GPU policy

- Windows 10 or Windows 11, x64.
- An NVIDIA RTX GPU with an installed driver that exposes NVENC HEVC/H.264.
- A writable local profile directory. Internet access is needed only when a
  requested map tile is not already cached.

The app deliberately does **not** fall back to Intel QSV, AMD, CPU encoding,
or Microsoft WARP. Open the GPU Diagnostics panel to see the selected adapter,
LUID, dedicated VRAM, driver/API information, NVENC capabilities, ring
occupancy, render/encode/mux timings, dropped/duplicated frames, and CPU
readback count. A valid zero-copy export reports `cpu_frame_readbacks == 0`.

## Map cache and offline use

Tiles are downloaded and decoded before rendering starts, then uploaded to the
RTX device once. Cached tiles are reused on later exports; the app does not
redownload a complete map for every video. The default cache lives at:

```text
%LOCALAPPDATA%\GPX Animator\cache
```

The Settings dialog lets you choose a 0.25–8 GB LRU limit or clear the cache.
The preflight status reports cached and missing tiles. Network timeouts,
permission errors, and offline runs degrade to a cached parent tile or a
deterministic placeholder so a transient tile request cannot corrupt the GPU
export pipeline.

## Export behavior

1. Parse and validate the GPX, remove stationary points according to the
   configured threshold, smooth elevation noise, and build a distance-uniform
   route timeline.
2. Build a tile manifest, load the disk cache, fetch only missing tiles, decode
   them, and upload the complete usable zoom level to the GPU.
3. Render into a six-texture D3D11 ring while NVENC encodes earlier textures
   asynchronously and the muxer writes completed packets.
4. Finalize an atomic MP4, move `moov` before `mdat` for fast start, and verify
   dimensions, frame rate, codec tag, sample count, and color metadata.

The default HEVC configuration is Main 8-bit, VBR, P5, CQ 19, two B-frames,
GOP 120, adaptive quantization enabled, and lookahead disabled. The Balanced
and Speed presets map to P4/CQ22/2 B-frames and P3/CQ25/no B-frames. H.264 is
available when a downstream player requires it. The exported video is not
speed-matched to every GPS pause: stop/red-light dwell is removed and the
remaining route is sampled at a smooth uniform pace.

## Build from source

Install Rust 1.92 or newer with the MSVC toolchain, then run these commands
from the repository root:

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --tests --examples -- -D warnings
cargo test --release --workspace --no-fail-fast
cargo build --release -p gpx-animator-native
```

The release executable is produced under Cargo's `target\release` directory.
The checked-in `dist` binary is the packaged build used for manual acceptance.
The NVIDIA Video Codec SDK headers are vendored for compilation; the runtime
loads the driver-provided NVENC library dynamically.

## Test and acceptance gates

The normal test suite is hardware-independent where possible and includes GPX
parsing/filtering, scene/camera math, tile cache and fallback behavior, D3D11
resource lifetime, NVENC state/error handling, MP4 `hvcC`/`hvc1` packaging,
fast-start relocation, UI state transitions, and preference persistence.

On an RTX Windows runner, the ignored integration gates exercise the actual
device and NVENC session:

```powershell
cargo test --release -p gpx-animator-native warm_cache_twenty_second_4k60_meets_realtime_gate -- --ignored
cargo test --release -p gpx-animator-native five_minute_4k60_has_exact_frames_and_realtime_throughput -- --ignored
cargo test --release -p gpx-animator-native ten_exports_do_not_leak_handles_or_partial_files -- --ignored
```

These gates check 4K60 dimensions and exact frame counts, real-time throughput
after warm cache, p95 render time, cancellation cleanup, zero CPU readback,
stable VRAM/handle usage, and the selected NVIDIA adapter. Run the ignored
tests only on the intended RTX machine; they are intentionally not required
for an ordinary CPU-only CI runner.

## Repository layout

```text
native/gpx-core          GPX parsing, stop filtering, elevation, sampling
native/scene-core        Scene description, camera and layout math
native/d3d11-renderer    D3D11/Direct2D renderer and tile cache
native/nvenc-engine      RTX texture ring and NVENC session
native/mp4-output        HEVC/H.264 sample conversion and MP4 muxing
native/desktop-app       egui UI, export state machine, diagnostics
legacy-web/              Previous browser prototype (not the release path)
packaging/               Windows packaging helpers
dist/                    Checked-in packaged executable
```

## Troubleshooting

- **No compatible GPU**: install/update the NVIDIA driver and confirm that an
  RTX adapter and NVENC capability appear in GPU Diagnostics. The app will not
  silently use Intel or software encoding.
- **Preload appears slow**: the first export must fetch and decode missing
  tiles. Subsequent exports use `%LOCALAPPDATA%\GPX Animator\cache`; the
  preflight counter shows whether work is network or cache related.
- **Tile access denied/offline**: verify firewall/proxy permissions or use a
  previously cached route. The renderer uses a parent tile/placeholder fallback
  rather than emitting mixed black tile blocks.
- **Cannot overwrite the EXE**: close every running GPX Animator process, then
  replace the packaged file.

## Attribution and license

Map tiles: © [OpenStreetMap contributors](https://www.openstreetmap.org/copyright).
Satellite imagery: © Esri, Maxar, Earthstar Geographics. See [LICENSE](LICENSE)
for the project license.

---

## 繁體中文

GPX Animator GPU Edition 是原生 Windows GPX 路線動畫工具，採用
**D3D11/Direct2D → NVIDIA RTX 紋理 → NVENC → MP4** 的 GPU 優先輸出管線。
影格會留在 GPU，直到編碼後的 bitstream 交給 MP4 muxer；正式 EXE 不需要
瀏覽器、Node.js、WebView 或 FFmpeg runtime。

## 功能摘要

- Rust 原生工作區與 `egui` 桌面介面。
- 明確選擇 NVIDIA 顯示卡；若只有 Intel、AMD、Microsoft WARP 或 CPU，會
  拒絕輸出，不會偷偷降級。
- D3D11 六紋理 ring 與非同步 NVENC，正式輸出不做 CPU frame readback。
- MP4 支援 H.265/HEVC Main 8-bit（`hvc1`）與 H.264（`avc1`），固定
  60/1 時戳、BT.709 色彩資訊及 fast-start `moov`。
- GPX 1.0/1.1、多 segment、停留／紅綠燈過濾、依距離勻速取樣、海拔降噪
  與路線統計。
- 衛星、深色、淺色、透明地圖；磁碟圖磚快取與離線父層／placeholder fallback。
- 跟隨、完整、自由攝影機；結尾平滑縮放到全路線，不切成黑畫面。
- 16:9、1:1、9:16；預覽可拖曳與滾輪縮放；HUD、海拔圖；預設路線寬度
  8 px。
- 繁體中文／英文 UI、設定保存、可調整面板、取消、進度及 GPU 診斷。

## 下載與啟動

最新的 Windows 版本是：

[GPX-Animator-GPU-20260713-hud8.exe](dist/GPX-Animator-GPU-20260713-hud8.exe)

可用檔案選擇器、把 GPX 拖進視窗，或從命令列開啟：

```powershell
.\dist\GPX-Animator-GPU-20260713-hud8.exe "D:\path\to\route.gpx"
```

預設輸出為 3840×2160、60 FPS、H.265、高畫質（NVENC P5/CQ19）、衛星圖
與 8 px 路線；左側設定面板可修改。若要覆蓋舊 EXE，請先關閉仍在執行的
舊版本，否則 Windows 會鎖定檔案。

## 系統需求與 GPU 政策

- Windows 10/11 x64。
- 具備 NVENC HEVC/H.264 的 NVIDIA RTX 與正確驅動。
- 可寫入使用者資料夾；只有未快取圖磚才需要網路。

程式不提供 Intel QSV、AMD、CPU 或 Microsoft WARP fallback。GPU 診斷頁會
顯示實際 adapter、LUID、專用 VRAM、驅動／API、NVENC 能力、ring 使用量、
各階段時間、丟幀／重複幀、峰值 VRAM 與 CPU readback 次數；零拷貝輸出應為
`cpu_frame_readbacks == 0`。

## 地圖快取與離線使用

圖磚會在渲染前下載、解碼並一次上傳 RTX；下一次輸出會重用本地快取，不會
每次重新下載整張地圖。預設位置：

```text
%LOCALAPPDATA%\GPX Animator\cache
```

設定視窗可調整 0.25–8 GB 的 LRU 上限或清除快取；預載狀態會顯示已快取與
缺少數量。網路逾時、權限錯誤或離線時，會使用快取父層圖磚或固定 placeholder，
避免暫時性的圖磚問題產生黑色區塊或中斷 GPU 輸出。

## 輸出流程與編碼

1. 解析並驗證 GPX，移除停留點、平滑海拔雜訊，建立依距離勻速的路線時間軸。
2. 建立圖磚 manifest，先讀磁碟快取，只下載缺少圖磚，解碼後上傳可用的完整
   zoom level。
3. 使用六張 D3D11 紋理 ring 渲染；NVENC 非同步編碼前一張紋理，muxer 寫入
   已完成的封包。
4. 產生原子 MP4，把 `moov` 移到 `mdat` 前方，並驗證解析度、FPS、codec tag、
   sample 數及色彩資訊。

HEVC 預設為 Main 8-bit、VBR、P5、CQ19、2 個 B-frame、GOP 120、開啟 AQ、
關閉 lookahead；平衡與高速分別是 P4/CQ22/2 B-frame 及 P3/CQ25/無 B-frame。
若播放器需要，可選 H.264。影片不會忠實放大 GPS 停等時間；停留／紅燈會被移除，
其餘路線以平滑勻速播放。

## 從原始碼建置與測試

安裝 Rust 1.92 以上及 MSVC toolchain，在專案根目錄執行：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --lib --bins --tests --examples -- -D warnings
cargo test --release --workspace --no-fail-fast
cargo build --release -p gpx-animator-native
```

RTX release gate 位於 `desktop-app` 的 ignored tests，會測試 20 秒／5 分鐘
4K60、十次連續輸出、精確幀數、零 CPU readback、取消及暫存檔清理。NVIDIA
Video Codec SDK headers 隨原始碼提供，執行時則動態載入驅動提供的 NVENC library。

## 專案目錄

```text
native/gpx-core          GPX 解析、停留過濾、海拔與取樣
native/scene-core        場景描述、攝影機與版面數學
native/d3d11-renderer    D3D11/Direct2D 渲染器與圖磚快取
native/nvenc-engine      RTX 紋理 ring 與 NVENC session
native/mp4-output        HEVC/H.264 sample 轉換與 MP4 muxing
native/desktop-app       egui UI、輸出狀態機、診斷
legacy-web/              舊瀏覽器原型（不屬於正式輸出）
packaging/               Windows 打包工具
dist/                    已打包的 EXE
```

## 授權與來源

地圖圖磚：© [OpenStreetMap contributors](https://www.openstreetmap.org/copyright)。
衛星圖：© Esri、Maxar、Earthstar Geographics。專案授權請見 [LICENSE](LICENSE)。
