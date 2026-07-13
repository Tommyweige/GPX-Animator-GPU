# GPX Animator GPU Edition

原生 Windows GPX 動畫輸出工具。正式輸出管線為 Direct2D/D3D11 → NVIDIA RTX texture → NVENC H.265/H.264 → MP4，不使用瀏覽器、Node.js、WebView 或 FFmpeg runtime。

## 使用方式

執行 [GPX-Animator-GPU.exe](dist/GPX-Animator-GPU.exe)，選擇 GPX 後設定畫面與輸出。也可以把 `.gpx` 拖入視窗，或用命令列直接開啟：

```powershell
.\dist\GPX-Animator-GPU.exe "D:\path\route.gpx"
```

預設為 3840×2160、60 FPS、H.265、NVENC P5/CQ19 高畫質、衛星圖、6 px 路線。支援 16:9、1:1、9:16，完整、跟隨（結尾平滑縮放至全景）與自由拖曳／滾輪縮放視角，以及 HUD、海拔圖、H.264 和取消輸出。MP4 會在完成後自動將 `moov` 移到檔案前端（fast-start），方便播放器立即開始播放。

## 系統需求

- Windows 10/11 x64
- NVIDIA RTX 顯示卡與支援 NVENC 的 NVIDIA 驅動
- 網路連線只用於首次下載地圖圖磚；圖磚會保存於 `%LOCALAPPDATA%\GPX Animator\cache`，之後可離線重用

沒有 RTX 或 NVENC 時會拒絕輸出，不會退回 Intel QSV、CPU、AMD 或 WARP。圖磚快取與設定位於 `%LOCALAPPDATA%\GPX Animator`。

## 建置與測試

```powershell
cargo test --workspace --offline
cargo fmt --all -- --check
cargo build --release -p gpx-animator-native --offline
```

RTX release Gate 位於 `desktop-app` 的 ignored tests，涵蓋 20 秒與 5 分鐘 4K60、十次連續輸出、精確幀數、零 CPU frame readback、取消與暫存檔清理。

舊瀏覽器版本保存在 `legacy-web`，不屬於正式 EXE。

OpenStreetMap 圖磚資料 © OpenStreetMap contributors；衛星圖資料 © Esri、Maxar、Earthstar Geographics。
