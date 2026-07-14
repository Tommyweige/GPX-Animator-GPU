use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::Hash;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const EXPORT_TEXTURE_COUNT: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    pub zoom: u8,
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone)]
pub struct DecodedTile {
    pub key: TileKey,
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// Immutable list of tiles required by one export. Keeping the manifest
/// separate from the worker queue makes preflight deterministic and lets the
/// UI report cached/missing work without treating it as video frames.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileManifest {
    pub keys: Vec<TileKey>,
    pub cached: usize,
    pub missing: usize,
}

impl TileManifest {
    pub fn new(cache: &TileDiskCache, mut keys: Vec<TileKey>) -> Self {
        keys.sort_unstable_by_key(|key| (key.zoom, key.y, key.x));
        keys.dedup();
        let cached = keys.iter().filter(|key| cache.is_cached(**key)).count();
        Self {
            missing: keys.len().saturating_sub(cached),
            keys,
            cached,
        }
    }

    pub fn total(&self) -> usize {
        self.keys.len()
    }
}

pub fn tile_zoom(view_span: f64, output_width: u32) -> u8 {
    ((output_width as f64 / (256.0 * view_span.max(1e-9)))
        .log2()
        .ceil() as i32)
        .clamp(2, 18) as u8
}

pub fn required_view_tiles(center: [f64; 2], span: f64, zoom: u8) -> Vec<TileKey> {
    let n = 1u32 << zoom;
    let half = span * 0.5;
    let x0 = ((center[0] - half) * n as f64)
        .floor()
        .clamp(0.0, (n - 1) as f64) as u32;
    let x1 = ((center[0] + half) * n as f64)
        .floor()
        .clamp(0.0, (n - 1) as f64) as u32;
    let y0 = ((center[1] - half) * n as f64)
        .floor()
        .clamp(0.0, (n - 1) as f64) as u32;
    let y1 = ((center[1] + half) * n as f64)
        .floor()
        .clamp(0.0, (n - 1) as f64) as u32;
    (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| TileKey { zoom, x, y }))
        .collect()
}

/// Select one complete tile level for a frame. Mixing parent and child tiles
/// creates visible colour seams on satellite imagery because providers may use
/// different source captures and colour grading at each zoom level.
pub fn complete_tile_zoom(
    available: &HashSet<TileKey>,
    center: [f64; 2],
    span: f64,
    output_width: u32,
) -> Option<u8> {
    let preferred = tile_zoom(span, output_width);
    (2..=preferred).rev().find(|&zoom| {
        required_view_tiles(center, span, zoom)
            .iter()
            .all(|key| available.contains(key))
    })
}

/// Return a usable single zoom even when a network/cache failure leaves a
/// level incomplete. The caller still draws one level only, so it cannot
/// create the colour seams caused by mixing satellite generations.
fn fallback_tile_zoom(
    available: &HashSet<TileKey>,
    center: [f64; 2],
    span: f64,
    output_width: u32,
) -> Option<u8> {
    let preferred = tile_zoom(span, output_width);
    complete_tile_zoom(available, center, span, output_width).or_else(|| {
        (2..=preferred).rev().find(|&zoom| {
            required_view_tiles(center, span, zoom)
                .iter()
                .any(|key| available.contains(key))
        })
    })
}

pub struct TileDiskCache {
    root: PathBuf,
    max_bytes: u64,
    source: TileSource,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileSource {
    OpenStreetMap,
    EsriSatellite,
}
impl TileDiskCache {
    pub fn new(root: impl Into<PathBuf>, max_bytes: u64) -> Self {
        Self {
            root: root.into(),
            max_bytes,
            source: TileSource::OpenStreetMap,
        }
    }
    pub fn with_source(mut self, source: TileSource) -> Self {
        self.source = source;
        self
    }
    pub fn default_osm() -> Self {
        let root = std::env::var_os("GPX_ANIMATOR_TILE_CACHE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join("GPX Animator")
                    .join("cache")
                    .join("osm")
            });
        Self::new(root, 512 * 1024 * 1024)
    }
    pub fn for_map_style(style: scene_core::MapStyle) -> Self {
        Self::for_map_style_with_limit(style, None)
    }
    pub fn for_map_style_with_limit(style: scene_core::MapStyle, limit_bytes: Option<u64>) -> Self {
        let limit = limit_bytes.filter(|value| *value > 0);
        match style {
            scene_core::MapStyle::Satellite => {
                let root = std::env::var_os("GPX_ANIMATOR_TILE_CACHE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::var_os("LOCALAPPDATA")
                            .map(PathBuf::from)
                            .unwrap_or_else(std::env::temp_dir)
                            .join("GPX Animator")
                            .join("cache")
                    })
                    .join("esri-satellite");
                Self::new(root, limit.unwrap_or(1024 * 1024 * 1024))
                    .with_source(TileSource::EsriSatellite)
            }
            _ => {
                let mut cache = Self::default_osm();
                if let Some(limit) = limit {
                    cache.max_bytes = limit;
                }
                cache
            }
        }
    }
    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn is_cached(&self, key: TileKey) -> bool {
        self.path(key).is_file()
    }
    pub fn clear(&self) -> Result<(), RendererError> {
        if self.root.exists() {
            std::fs::remove_dir_all(&self.root)
                .map_err(|error| RendererError::Api(error.to_string()))?;
        }
        Ok(())
    }
    fn path(&self, key: TileKey) -> PathBuf {
        self.root
            .join(key.zoom.to_string())
            .join(key.x.to_string())
            .join(format!("{}.png", key.y))
    }
    pub fn tile_url(&self, key: TileKey) -> String {
        match self.source {
            TileSource::OpenStreetMap => format!(
                "https://tile.openstreetmap.org/{}/{}/{}.png",
                key.zoom, key.x, key.y
            ),
            TileSource::EsriSatellite => format!(
                "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{}/{}/{}",
                key.zoom, key.y, key.x
            ),
        }
    }
    pub fn load(&self, key: TileKey) -> Result<DecodedTile, RendererError> {
        self.load_with_fetch(key, |url| {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(std::time::Duration::from_secs(5)))
                .build()
                .new_agent();
            let mut response = agent
                .get(url)
                .header("User-Agent", "GPXAnimatorNative/2.0")
                .call()
                .map_err(|error| error.to_string())?;
            response
                .body_mut()
                .read_to_vec()
                .map_err(|error| error.to_string())
        })
    }
    pub fn load_with_fetch<F>(&self, key: TileKey, fetch: F) -> Result<DecodedTile, RendererError>
    where
        F: FnOnce(&str) -> Result<Vec<u8>, String>,
    {
        let path = self.path(key);
        let bytes = if path.exists() {
            std::fs::read(&path).map_err(|e| RendererError::Tile(e.to_string()))?
        } else {
            if let Some(parent) = path.parent()
                && let Err(error) = std::fs::create_dir_all(parent)
            {
                let _ = error;
                return Ok(self.offline_placeholder(key));
            }
            let url = self.tile_url(key);
            match fetch(&url) {
                Ok(bytes) => {
                    // A read-only cache must not make an export fail. The
                    // decoded tile is still useful for this run even when it
                    // cannot be persisted.
                    let _ = std::fs::write(&path, &bytes);
                    let _ = self.evict();
                    bytes
                }
                Err(_) => return Ok(self.offline_placeholder(key)),
            }
        };
        decode_tile(key, &bytes).or_else(|_| Ok(self.offline_placeholder(key)))
    }
    fn offline_placeholder(&self, key: TileKey) -> DecodedTile {
        if let Some(tile) = self.cached_parent_tile(key) {
            return tile;
        }
        DecodedTile {
            key,
            width: 0,
            height: 0,
            bgra: Vec::new(),
        }
    }
    fn cached_parent_tile(&self, key: TileKey) -> Option<DecodedTile> {
        for levels in 1..=4u8 {
            if key.zoom < levels {
                break;
            }
            let parent = TileKey {
                zoom: key.zoom - levels,
                x: key.x >> levels,
                y: key.y >> levels,
            };
            let Ok(bytes) = std::fs::read(self.path(parent)) else {
                continue;
            };
            let Ok(image) = image::load_from_memory(&bytes) else {
                continue;
            };
            let rgba = image.to_rgba8();
            let divisions = 1u32 << levels;
            let crop_width = rgba.width() / divisions;
            let crop_height = rgba.height() / divisions;
            if crop_width == 0 || crop_height == 0 {
                continue;
            }
            let local_x = key.x & (divisions - 1);
            let local_y = key.y & (divisions - 1);
            let crop = image::imageops::crop_imm(
                &rgba,
                local_x * crop_width,
                local_y * crop_height,
                crop_width,
                crop_height,
            )
            .to_image();
            let resized =
                image::imageops::resize(&crop, 256, 256, image::imageops::FilterType::Lanczos3);
            let mut bgra = resized.into_raw();
            for pixel in bgra.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            return Some(DecodedTile {
                key,
                width: 256,
                height: 256,
                bgra,
            });
        }
        None
    }
    fn evict(&self) -> Result<(), RendererError> {
        let mut files = Vec::new();
        collect_files(&self.root, &mut files)?;
        let mut total: u64 = files.iter().map(|v| v.1).sum();
        files.sort_by_key(|v| v.2);
        for (path, size, _) in files {
            if total <= self.max_bytes {
                break;
            }
            std::fs::remove_file(path).map_err(|e| RendererError::Api(e.to_string()))?;
            total -= size;
        }
        Ok(())
    }
}
fn collect_files(
    root: &Path,
    out: &mut Vec<(PathBuf, u64, std::time::SystemTime)>,
) -> Result<(), RendererError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root).map_err(|e| RendererError::Api(e.to_string()))? {
        let entry = entry.map_err(|e| RendererError::Api(e.to_string()))?;
        let meta = entry
            .metadata()
            .map_err(|e| RendererError::Api(e.to_string()))?;
        if meta.is_dir() {
            collect_files(&entry.path(), out)?
        } else {
            out.push((
                entry.path(),
                meta.len(),
                meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            ));
        }
    }
    Ok(())
}
pub fn decode_tile(key: TileKey, bytes: &[u8]) -> Result<DecodedTile, RendererError> {
    let rgba = image::load_from_memory(bytes)
        .map_err(|e| RendererError::Tile(e.to_string()))?
        .to_rgba8();
    let (width, height) = rgba.dimensions();
    let mut bgra = rgba.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2)
    }
    Ok(DecodedTile {
        key,
        width,
        height,
        bgra,
    })
}

pub struct TileLru<K, V> {
    capacity_bytes: usize,
    used_bytes: usize,
    entries: HashMap<K, (V, usize)>,
    order: VecDeque<K>,
}
impl<K: Clone + Eq + Hash, V> TileLru<K, V> {
    pub fn new(capacity_bytes: usize) -> Self {
        Self {
            capacity_bytes,
            used_bytes: 0,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }
    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }
    pub fn get(&mut self, key: &K) -> Option<&V> {
        if self.entries.contains_key(key) {
            self.order.retain(|v| v != key);
            self.order.push_back(key.clone());
        }
        self.entries.get(key).map(|v| &v.0)
    }
    pub fn insert(&mut self, key: K, value: V, bytes: usize) {
        if let Some((_, old_bytes)) = self.entries.remove(&key) {
            self.used_bytes -= old_bytes;
            self.order.retain(|v| v != &key);
        }
        if bytes > self.capacity_bytes {
            return;
        }
        while self.used_bytes + bytes > self.capacity_bytes {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some((_, old_bytes)) = self.entries.remove(&old) {
                self.used_bytes -= old_bytes;
            }
        }
        self.used_bytes += bytes;
        self.order.push_back(key.clone());
        self.entries.insert(key, (value, bytes));
    }
}

pub fn required_tiles(bounds: (f64, f64, f64, f64), zoom: u8) -> Vec<TileKey> {
    fn tile(lon: f64, lat: f64, zoom: u8) -> (u32, u32) {
        let n = 2f64.powi(zoom as i32);
        let x = ((lon + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0) as u32;
        let lat = lat.clamp(-85.05112878, 85.05112878).to_radians();
        let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n)
            .floor()
            .clamp(0.0, n - 1.0) as u32;
        (x, y)
    }
    let (min_lon, min_lat, max_lon, max_lat) = bounds;
    let (x0, y1) = tile(min_lon, min_lat, zoom);
    let (x1, y0) = tile(max_lon, max_lat, zoom);
    let mut out = Vec::new();
    for y in y0.min(y1)..=y0.max(y1) {
        for x in x0.min(x1)..=x0.max(x1) {
            out.push(TileKey { zoom, x, y });
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    pub name: String,
    pub luid: u64,
    pub dedicated_vram: u64,
    pub is_nvidia: bool,
    pub is_software: bool,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RendererError {
    #[error("找不到 NVIDIA RTX 顯示卡")]
    NoRtxAdapter,
    #[error("GPU device lost")]
    DeviceLost,
    #[error("Windows GPU API 失敗：{0}")]
    Api(String),
    #[error("地圖圖磚失敗：{0}")]
    Tile(String),
}

#[cfg(windows)]
pub struct D3d11ExportDevice {
    pub info: AdapterInfo,
    pub device: windows::Win32::Graphics::Direct3D11::ID3D11Device,
    pub context: windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext,
}

#[cfg(windows)]
impl D3d11ExportDevice {
    pub fn create_rtx() -> Result<Self, RendererError> {
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0};
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
        };
        use windows::Win32::Graphics::Dxgi::{
            CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIAdapter, IDXGIFactory1,
        };
        use windows::core::Interface;

        let factory: IDXGIFactory1 =
            unsafe { CreateDXGIFactory1().map_err(|error| RendererError::Api(error.to_string()))? };
        let mut candidates = Vec::new();
        for index in 0..32 {
            let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else {
                break;
            };
            let desc = unsafe {
                adapter
                    .GetDesc1()
                    .map_err(|error| RendererError::Api(error.to_string()))?
            };
            let name_end = desc
                .Description
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(desc.Description.len());
            let name = String::from_utf16_lossy(&desc.Description[..name_end]);
            if desc.VendorId == 0x10de
                && desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 == 0
                && name.to_ascii_uppercase().contains("RTX")
            {
                candidates.push((
                    desc.DedicatedVideoMemory,
                    adapter,
                    AdapterInfo {
                        name,
                        luid: ((desc.AdapterLuid.HighPart as u64) << 32)
                            | desc.AdapterLuid.LowPart as u64,
                        dedicated_vram: desc.DedicatedVideoMemory as u64,
                        is_nvidia: true,
                        is_software: false,
                    },
                ));
            }
        }
        let (_, adapter, info) = candidates
            .into_iter()
            .max_by_key(|candidate| candidate.0)
            .ok_or(RendererError::NoRtxAdapter)?;
        let adapter: IDXGIAdapter = adapter
            .cast()
            .map_err(|error| RendererError::Api(error.to_string()))?;
        let mut device = None;
        let mut context = None;
        unsafe {
            D3D11CreateDevice(
                &adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|error| RendererError::Api(error.to_string()))?;
        }
        Ok(Self {
            info,
            device: device.ok_or(RendererError::DeviceLost)?,
            context: context.ok_or(RendererError::DeviceLost)?,
        })
    }

    pub fn create_export_textures(
        &self,
        width: u32,
        height: u32,
    ) -> Result<Vec<windows::Win32::Graphics::Direct3D11::ID3D11Texture2D>, RendererError> {
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC,
            D3D11_USAGE_DEFAULT, ID3D11Texture2D,
        };
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut textures = Vec::with_capacity(EXPORT_TEXTURE_COUNT);
        for _ in 0..EXPORT_TEXTURE_COUNT {
            let mut texture: Option<ID3D11Texture2D> = None;
            unsafe {
                self.device
                    .CreateTexture2D(&desc, None, Some(&mut texture))
                    .map_err(|error| RendererError::Api(error.to_string()))?;
            }
            textures.push(texture.ok_or(RendererError::DeviceLost)?);
        }
        Ok(textures)
    }

    pub fn clear_texture(
        &self,
        texture: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
        color: [f32; 4],
    ) -> Result<(), RendererError> {
        use windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView;
        let mut view: Option<ID3D11RenderTargetView> = None;
        unsafe {
            self.device
                .CreateRenderTargetView(texture, None, Some(&mut view))
                .map_err(|error| RendererError::Api(error.to_string()))?;
            self.context
                .ClearRenderTargetView(&view.ok_or(RendererError::DeviceLost)?, &color);
        }
        Ok(())
    }

    pub fn flush(&self) {
        unsafe {
            self.context.Flush();
        }
    }
}

#[cfg(windows)]
pub struct D2dSceneRenderer {
    context: windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
    text_format: windows::Win32::Graphics::DirectWrite::IDWriteTextFormat,
}
#[cfg(windows)]
pub struct GpuTileSet {
    tiles: HashMap<TileKey, windows::Win32::Graphics::Direct2D::ID2D1Bitmap1>,
}

#[cfg(windows)]
impl D2dSceneRenderer {
    pub fn new(device: &D3d11ExportDevice) -> Result<Self, RendererError> {
        use windows::Win32::Graphics::Direct2D::{
            D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1CreateFactory, ID2D1Factory1,
        };
        use windows::Win32::Graphics::DirectWrite::{
            DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_WEIGHT_SEMI_BOLD, DWriteCreateFactory, IDWriteFactory,
        };
        use windows::Win32::Graphics::Dxgi::IDXGIDevice;
        use windows::core::{Interface, PCWSTR};
        let dxgi: IDXGIDevice = device
            .device
            .cast()
            .map_err(|error| RendererError::Api(error.to_string()))?;
        let factory: ID2D1Factory1 = unsafe {
            D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let d2d_device = unsafe {
            factory
                .CreateDevice(&dxgi)
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let context = unsafe {
            d2d_device
                .CreateDeviceContext(
                    windows::Win32::Graphics::Direct2D::D2D1_DEVICE_CONTEXT_OPTIONS_NONE,
                )
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let write_factory: IDWriteFactory = unsafe {
            DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let font: Vec<u16> = "Microsoft JhengHei UI\0".encode_utf16().collect();
        let locale: Vec<u16> = "zh-TW\0".encode_utf16().collect();
        let text_format = unsafe {
            write_factory
                .CreateTextFormat(
                    PCWSTR(font.as_ptr()),
                    None,
                    DWRITE_FONT_WEIGHT_SEMI_BOLD,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    36.0,
                    PCWSTR(locale.as_ptr()),
                )
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        Ok(Self {
            context,
            text_format,
        })
    }

    pub fn prepare_tiles(&self, tiles: Vec<DecodedTile>) -> Result<GpuTileSet, RendererError> {
        use windows::Win32::Graphics::Direct2D::Common::{
            D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
        };
        use windows::Win32::Graphics::Direct2D::{
            D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1,
        };
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
        let mut result = HashMap::new();
        for tile in tiles {
            // Offline placeholders have no visible pixels. Do not create a D2D
            // bitmap for them: some drivers composite transparent bitmap data
            // as opaque black, producing a grid of black rectangles.
            if tile.bgra.is_empty() || tile.width == 0 || tile.height == 0 {
                continue;
            }
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                ..Default::default()
            };
            let bitmap = unsafe {
                self.context
                    .CreateBitmap(
                        D2D_SIZE_U {
                            width: tile.width,
                            height: tile.height,
                        },
                        Some(tile.bgra.as_ptr().cast()),
                        tile.width * 4,
                        &props,
                    )
                    .map_err(|e| RendererError::Api(e.to_string()))?
            };
            result.insert(tile.key, bitmap);
        }
        Ok(GpuTileSet { tiles: result })
    }
    pub fn render(
        &self,
        texture: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
        frame: &scene_core::FramePlan,
        options: &scene_core::SceneOptions,
        width: u32,
        height: u32,
    ) -> Result<(), RendererError> {
        self.render_with_tiles(texture, frame, options, width, height, None)
    }
    pub fn render_with_tiles(
        &self,
        texture: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
        frame: &scene_core::FramePlan,
        options: &scene_core::SceneOptions,
        width: u32,
        height: u32,
        tiles: Option<&GpuTileSet>,
    ) -> Result<(), RendererError> {
        use windows::Win32::Graphics::Direct2D::Common::{
            D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
        };
        use windows::Win32::Graphics::Direct2D::{
            D2D1_BITMAP_OPTIONS, D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_TARGET,
            D2D1_BITMAP_PROPERTIES1, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_ELLIPSE,
            D2D1_INTERPOLATION_MODE_LINEAR,
        };
        use windows::Win32::Graphics::DirectWrite::DWRITE_MEASURING_MODE_NATURAL;
        use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
        use windows::Win32::Graphics::Dxgi::IDXGISurface;
        use windows::core::Interface;
        use windows_numerics::Vector2;
        let surface: IDXGISurface = texture
            .cast()
            .map_err(|error| RendererError::Api(error.to_string()))?;
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS(
                D2D1_BITMAP_OPTIONS_TARGET.0 | D2D1_BITMAP_OPTIONS_CANNOT_DRAW.0,
            ),
            ..Default::default()
        };
        let bitmap = unsafe {
            self.context
                .CreateBitmapFromDxgiSurface(&surface, Some(&properties))
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        unsafe {
            self.context.SetTarget(&bitmap);
            self.context.BeginDraw();
        }
        let background = match options.map_style {
            scene_core::MapStyle::Light => D2D1_COLOR_F {
                r: 0.90,
                g: 0.92,
                b: 0.93,
                a: 1.0,
            },
            scene_core::MapStyle::Dark => D2D1_COLOR_F {
                r: 0.045,
                g: 0.064,
                b: 0.080,
                a: 1.0,
            },
            scene_core::MapStyle::Satellite => D2D1_COLOR_F {
                r: 0.08,
                g: 0.10,
                b: 0.12,
                a: 1.0,
            },
            scene_core::MapStyle::Transparent => D2D1_COLOR_F {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.0,
            },
        };
        unsafe {
            self.context.Clear(Some(&background));
        }
        if options.map_style != scene_core::MapStyle::Transparent
            && let Some(tiles) = tiles
        {
            let available: HashSet<_> = tiles.tiles.keys().copied().collect();
            let selected_zoom = fallback_tile_zoom(
                &available,
                frame.view_center_mercator,
                frame.view_span,
                width,
            );
            for (key, tile) in tiles
                .tiles
                .iter()
                .filter(|(key, _)| Some(key.zoom) == selected_zoom)
            {
                let n = (1u32 << key.zoom) as f64;
                let map = |x: f64, y: f64| {
                    (
                        (x - frame.view_center_mercator[0]) * 2.0 / frame.view_span,
                        -(y - frame.view_center_mercator[1]) * 2.0 / frame.view_span,
                    )
                };
                let (a, b) = map(key.x as f64 / n, key.y as f64 / n);
                let (c, d) = map((key.x + 1) as f64 / n, (key.y + 1) as f64 / n);
                let dest = D2D_RECT_F {
                    left: ((a * 0.5 + 0.5) * width as f64) as f32,
                    top: ((0.5 - b * 0.5) * height as f64) as f32,
                    right: ((c * 0.5 + 0.5) * width as f64) as f32,
                    bottom: ((0.5 - d * 0.5) * height as f64) as f32,
                };
                unsafe {
                    self.context.DrawBitmap(
                        tile,
                        Some(&dest),
                        1.0,
                        D2D1_INTERPOLATION_MODE_LINEAR,
                        None,
                        None,
                    );
                }
            }
        }
        let color = |rgba: [u8; 4]| D2D1_COLOR_F {
            r: rgba[0] as f32 / 255.0,
            g: rgba[1] as f32 / 255.0,
            b: rgba[2] as f32 / 255.0,
            a: rgba[3] as f32 / 255.0,
        };
        let route_brush = unsafe {
            self.context
                .CreateSolidColorBrush(&color(options.route_color), None)
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let marker_brush = unsafe {
            self.context
                .CreateSolidColorBrush(&color(options.marker_color), None)
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let hud_shadow_brush = unsafe {
            self.context
                .CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.70,
                    },
                    None,
                )
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let muted_brush = unsafe {
            self.context
                .CreateSolidColorBrush(
                    &D2D1_COLOR_F {
                        r: 0.38,
                        g: 0.43,
                        b: 0.46,
                        a: 0.62,
                    },
                    None,
                )
                .map_err(|error| RendererError::Api(error.to_string()))?
        };
        let to_pixel = |point: [f32; 2]| Vector2 {
            X: (point[0] * 0.5 + 0.5) * width as f32,
            Y: (0.5 - point[1] * 0.5) * height as f32,
        };
        for pair in frame.route_ndc.windows(2) {
            unsafe {
                self.context.DrawLine(
                    to_pixel(pair[0]),
                    to_pixel(pair[1]),
                    &muted_brush,
                    options.line_width_px,
                    None,
                );
            }
        }
        for pair in frame.route_ndc[..frame.completed_points.min(frame.route_ndc.len())].windows(2)
        {
            unsafe {
                self.context.DrawLine(
                    to_pixel(pair[0]),
                    to_pixel(pair[1]),
                    &route_brush,
                    options.line_width_px,
                    None,
                );
            }
        }
        let marker = to_pixel(frame.marker_ndc);
        let ellipse = D2D1_ELLIPSE {
            point: marker,
            radiusX: 12.0,
            radiusY: 12.0,
        };
        unsafe {
            self.context.FillEllipse(&ellipse, &marker_brush);
        }
        // Route landmarks are deliberately drawn as lightweight vector layers:
        // a soft offset shadow, a stem, a highlighted pin and an optional
        // two-line label.  This gives the Relive-like depth cue without an
        // image asset, GPU readback, or a per-frame bitmap allocation.
        if !frame.landmarks.is_empty() {
            let shadow_brush = unsafe {
                self.context
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 0.48,
                        },
                        None,
                    )
                    .map_err(|error| RendererError::Api(error.to_string()))?
            };
            let label_background = unsafe {
                self.context
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: 0.035,
                            g: 0.055,
                            b: 0.07,
                            a: 0.86,
                        },
                        None,
                    )
                    .map_err(|error| RendererError::Api(error.to_string()))?
            };
            let reference_scale = height as f32 / 2160.0;
            for landmark in &frame.landmarks {
                if landmark.pin_opacity <= 0.0
                    || landmark.ndc[0] < -1.25
                    || landmark.ndc[0] > 1.25
                    || landmark.ndc[1] < -1.25
                    || landmark.ndc[1] > 1.25
                {
                    continue;
                }
                let point = to_pixel(landmark.ndc);
                let scale = reference_scale * landmark.pin_scale.max(0.2);
                let radius = 18.0 * scale;
                let opacity = landmark.pin_opacity.clamp(0.0, 1.0);
                let shadow = D2D1_ELLIPSE {
                    point: Vector2 {
                        X: point.X + 6.0 * reference_scale,
                        Y: point.Y + 9.0 * reference_scale,
                    },
                    radiusX: radius * 1.10,
                    radiusY: radius * 0.78,
                };
                let stem_start = Vector2 {
                    X: point.X,
                    Y: point.Y + radius * 0.55,
                };
                let stem_end = Vector2 {
                    X: point.X,
                    Y: point.Y + 27.0 * reference_scale,
                };
                let outer = D2D1_ELLIPSE {
                    point,
                    radiusX: radius,
                    radiusY: radius,
                };
                let inner = D2D1_ELLIPSE {
                    point,
                    radiusX: radius * 0.66,
                    radiusY: radius * 0.66,
                };
                let pin_color = D2D1_COLOR_F {
                    r: landmark.color[0] as f32 / 255.0,
                    g: landmark.color[1] as f32 / 255.0,
                    b: landmark.color[2] as f32 / 255.0,
                    a: (landmark.color[3] as f32 / 255.0) * opacity,
                };
                let pin_brush = unsafe {
                    self.context
                        .CreateSolidColorBrush(&pin_color, None)
                        .map_err(|error| RendererError::Api(error.to_string()))?
                };
                let pulse_brush = unsafe {
                    self.context
                        .CreateSolidColorBrush(
                            &D2D1_COLOR_F {
                                r: landmark.color[0] as f32 / 255.0,
                                g: landmark.color[1] as f32 / 255.0,
                                b: landmark.color[2] as f32 / 255.0,
                                a: 0.20 * opacity,
                            },
                            None,
                        )
                        .map_err(|error| RendererError::Api(error.to_string()))?
                };
                let outer_brush = unsafe {
                    self.context
                        .CreateSolidColorBrush(
                            &D2D1_COLOR_F {
                                r: 1.0,
                                g: 0.96,
                                b: 0.84,
                                a: 0.95 * opacity,
                            },
                            None,
                        )
                        .map_err(|error| RendererError::Api(error.to_string()))?
                };
                unsafe {
                    if landmark.pulse_progress > 0.0 {
                        let pulse = D2D1_ELLIPSE {
                            point,
                            radiusX: radius * (1.25 + landmark.pulse_progress * 0.7),
                            radiusY: radius * (1.25 + landmark.pulse_progress * 0.7),
                        };
                        self.context.FillEllipse(&pulse, &pulse_brush);
                    }
                    self.context.FillEllipse(&shadow, &shadow_brush);
                    self.context.DrawLine(
                        stem_start,
                        stem_end,
                        &shadow_brush,
                        8.0 * reference_scale,
                        None,
                    );
                    self.context.DrawLine(
                        stem_start,
                        stem_end,
                        &pin_brush,
                        4.0 * reference_scale,
                        None,
                    );
                    self.context.FillEllipse(&outer, &outer_brush);
                    self.context.FillEllipse(&inner, &pin_brush);
                }
                if landmark.label_opacity > 0.0 && landmark.show_label {
                    let label_scale = reference_scale;
                    let label_width = 520.0 * label_scale;
                    let label_height = if landmark.category.is_some() {
                        94.0 * label_scale
                    } else {
                        60.0 * label_scale
                    };
                    let side = if landmark.ndc[0] > 0.55 { -1.0 } else { 1.0 };
                    let left = if side > 0.0 {
                        point.X + 28.0 * label_scale
                    } else {
                        point.X - 28.0 * label_scale - label_width
                    };
                    let top = point.Y - label_height - 18.0 * label_scale;
                    let rect = D2D_RECT_F {
                        left,
                        top,
                        right: left + label_width,
                        bottom: top + label_height,
                    };
                    let mut text = landmark.name.clone();
                    if let Some(category) = &landmark.category
                        && !category.trim().is_empty()
                    {
                        text.push('\n');
                        text.push_str(category);
                    }
                    let utf16: Vec<u16> = text.encode_utf16().collect();
                    let text_rect = D2D_RECT_F {
                        left: rect.left + 20.0 * label_scale,
                        top: rect.top + 10.0 * label_scale,
                        right: rect.right - 16.0 * label_scale,
                        bottom: rect.bottom - 8.0 * label_scale,
                    };
                    let alpha = landmark.label_opacity.clamp(0.0, 1.0);
                    let label_shadow = D2D1_COLOR_F {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 0.9 * alpha,
                    };
                    let label_foreground = D2D1_COLOR_F {
                        r: 1.0,
                        g: 0.97,
                        b: 0.89,
                        a: alpha,
                    };
                    let label_shadow_brush = unsafe {
                        self.context
                            .CreateSolidColorBrush(&label_shadow, None)
                            .map_err(|error| RendererError::Api(error.to_string()))?
                    };
                    let label_foreground_brush = unsafe {
                        self.context
                            .CreateSolidColorBrush(&label_foreground, None)
                            .map_err(|error| RendererError::Api(error.to_string()))?
                    };
                    unsafe {
                        self.context.FillRectangle(&rect, &label_background);
                        let shadow_rect = D2D_RECT_F {
                            left: text_rect.left + 2.0 * label_scale,
                            top: text_rect.top + 2.0 * label_scale,
                            right: text_rect.right + 2.0 * label_scale,
                            bottom: text_rect.bottom + 2.0 * label_scale,
                        };
                        self.context.DrawText(
                            &utf16,
                            &self.text_format,
                            &shadow_rect,
                            &label_shadow_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                            DWRITE_MEASURING_MODE_NATURAL,
                        );
                        self.context.DrawText(
                            &utf16,
                            &self.text_format,
                            &text_rect,
                            &label_foreground_brush,
                            D2D1_DRAW_TEXT_OPTIONS_NONE,
                            DWRITE_MEASURING_MODE_NATURAL,
                        );
                    }
                }
            }
        }
        if options.show_hud {
            let overlay = scene_core::overlay_layout(options.aspect);
            let rect = D2D_RECT_F {
                left: width as f32 * overlay.hud[0],
                top: height as f32 * overlay.hud[1],
                right: width as f32 * (overlay.hud[0] + overlay.hud[2]),
                bottom: height as f32 * (overlay.hud[1] + overlay.hud[3]),
            };
            let text_rect = D2D_RECT_F {
                left: rect.left + 24.0,
                top: rect.top + 14.0,
                right: rect.right - 16.0,
                bottom: rect.bottom - 8.0,
            };
            let altitude = frame
                .elevation_m
                .map(|value| format!("{value:.0} m"))
                .unwrap_or_else(|| "-- m".to_owned());
            let label = format!(
                "公里數 {:.2} km\n海拔 {altitude}",
                frame.distance_m / 1000.0
            );
            let text: Vec<u16> = label.encode_utf16().collect();
            unsafe {
                let shadow_rect = D2D_RECT_F {
                    left: text_rect.left + 2.0,
                    top: text_rect.top + 2.0,
                    right: text_rect.right + 2.0,
                    bottom: text_rect.bottom + 2.0,
                };
                self.context.DrawText(
                    &text,
                    &self.text_format,
                    &shadow_rect,
                    &hud_shadow_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
                self.context.DrawText(
                    &text,
                    &self.text_format,
                    &text_rect,
                    &marker_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
        }
        if options.show_elevation && frame.elevation_line.len() > 1 {
            let overlay = scene_core::overlay_layout(options.aspect);
            let chart_left = width as f32 * overlay.elevation[0];
            let chart_right = width as f32 * (overlay.elevation[0] + overlay.elevation[2]);
            // Compact, panel-free profile in the upper-right. The map remains the
            // primary visual and the profile is only a contextual cue.
            let chart_top = height as f32 * overlay.elevation[1];
            let chart_bottom = height as f32 * (overlay.elevation[1] + overlay.elevation[3]);
            let completed_fill = unsafe {
                self.context
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: options.route_color[0] as f32 / 255.0,
                            g: options.route_color[1] as f32 / 255.0,
                            b: options.route_color[2] as f32 / 255.0,
                            a: 0.18,
                        },
                        None,
                    )
                    .map_err(|error| RendererError::Api(error.to_string()))?
            };
            let elevation_brush = unsafe {
                self.context
                    .CreateSolidColorBrush(
                        &D2D1_COLOR_F {
                            r: options.route_color[0] as f32 / 255.0,
                            g: options.route_color[1] as f32 / 255.0,
                            b: options.route_color[2] as f32 / 255.0,
                            a: 0.70,
                        },
                        None,
                    )
                    .map_err(|error| RendererError::Api(error.to_string()))?
            };
            let progress_x =
                chart_left + (chart_right - chart_left) * frame.progress.clamp(0.0, 1.0);
            let map = |point: [f32; 2]| Vector2 {
                X: chart_left + (point[0] * 0.5 + 0.5) * (chart_right - chart_left),
                Y: chart_top + (0.5 - point[1] * 0.5) * (chart_bottom - chart_top),
            };
            for pair in frame.elevation_line.windows(2) {
                let a = map(pair[0]);
                let b = map(pair[1]);
                if a.X < progress_x {
                    let end_x = b.X.min(progress_x);
                    let ratio = if (b.X - a.X).abs() > f32::EPSILON {
                        ((end_x - a.X) / (b.X - a.X)).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let end_y = a.Y + (b.Y - a.Y) * ratio;
                    unsafe {
                        self.context.DrawLine(
                            Vector2 { X: a.X, Y: a.Y },
                            Vector2 {
                                X: a.X,
                                Y: chart_bottom,
                            },
                            &completed_fill,
                            (end_x - a.X).max(1.0) + 1.0,
                            None,
                        );
                        if end_x > a.X {
                            self.context.DrawLine(
                                Vector2 { X: end_x, Y: end_y },
                                Vector2 {
                                    X: end_x,
                                    Y: chart_bottom,
                                },
                                &completed_fill,
                                (end_x - a.X).max(1.0) + 1.0,
                                None,
                            );
                        }
                    }
                }
                unsafe {
                    self.context.DrawLine(a, b, &elevation_brush, 2.0, None);
                }
            }
        }
        unsafe {
            self.context
                .EndDraw(None, None)
                .map_err(|error| RendererError::Api(error.to_string()))?;
            self.context.SetTarget(None);
        }
        Ok(())
    }
}

pub fn select_rtx_adapter(adapters: &[AdapterInfo]) -> Result<&AdapterInfo, RendererError> {
    adapters
        .iter()
        .filter(|adapter| {
            adapter.is_nvidia
                && !adapter.is_software
                && adapter.name.to_ascii_uppercase().contains("RTX")
        })
        .max_by_key(|adapter| adapter.dedicated_vram)
        .ok_or(RendererError::NoRtxAdapter)
}

#[cfg(windows)]
pub fn enumerate_adapters() -> Result<Vec<AdapterInfo>, RendererError> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIFactory1,
    };
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1().map_err(|_| RendererError::DeviceLost)? };
    let mut adapters = Vec::new();
    for index in 0..32 {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(value) => value,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(_) => return Err(RendererError::DeviceLost),
        };
        let desc = unsafe { adapter.GetDesc1().map_err(|_| RendererError::DeviceLost)? };
        let name_end = desc
            .Description
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(desc.Description.len());
        let name = String::from_utf16_lossy(&desc.Description[..name_end]);
        let luid = ((desc.AdapterLuid.HighPart as u64) << 32) | desc.AdapterLuid.LowPart as u64;
        adapters.push(AdapterInfo {
            name,
            luid,
            dedicated_vram: desc.DedicatedVideoMemory as u64,
            is_nvidia: desc.VendorId == 0x10de,
            is_software: desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0,
        });
    }
    Ok(adapters)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureState {
    Free,
    Rendering(u64),
    Ready(u64),
    Encoding(u64),
}

#[derive(Debug)]
pub struct TextureRing {
    states: [TextureState; EXPORT_TEXTURE_COUNT],
    ready: VecDeque<usize>,
}

impl Default for TextureRing {
    fn default() -> Self {
        Self {
            states: [TextureState::Free; EXPORT_TEXTURE_COUNT],
            ready: VecDeque::new(),
        }
    }
}

impl TextureRing {
    pub fn acquire_render(&mut self, frame: u64) -> Option<usize> {
        let index = self
            .states
            .iter()
            .position(|state| *state == TextureState::Free)?;
        self.states[index] = TextureState::Rendering(frame);
        Some(index)
    }
    pub fn finish_render(&mut self, index: usize) {
        if let TextureState::Rendering(frame) = self.states[index] {
            self.states[index] = TextureState::Ready(frame);
            self.ready.push_back(index);
        }
    }
    pub fn acquire_encode(&mut self) -> Option<(usize, u64)> {
        let index = self.ready.pop_front()?;
        let TextureState::Ready(frame) = self.states[index] else {
            return None;
        };
        self.states[index] = TextureState::Encoding(frame);
        Some((index, frame))
    }
    pub fn finish_encode(&mut self, index: usize) {
        self.states[index] = TextureState::Free;
    }
    pub fn occupancy(&self) -> usize {
        self.states
            .iter()
            .filter(|state| **state != TextureState::Free)
            .count()
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selects_largest_real_rtx() {
        let adapters = vec![
            AdapterInfo {
                name: "Intel UHD".into(),
                luid: 1,
                dedicated_vram: 0,
                is_nvidia: false,
                is_software: false,
            },
            AdapterInfo {
                name: "NVIDIA GeForce RTX 2080 Ti".into(),
                luid: 2,
                dedicated_vram: 11,
                is_nvidia: true,
                is_software: false,
            },
            AdapterInfo {
                name: "NVIDIA GeForce RTX 2060".into(),
                luid: 3,
                dedicated_vram: 6,
                is_nvidia: true,
                is_software: false,
            },
        ];
        assert_eq!(select_rtx_adapter(&adapters).unwrap().luid, 2);
    }
    #[test]
    fn rejects_software_and_non_rtx() {
        let adapters = vec![AdapterInfo {
            name: "Microsoft WARP".into(),
            luid: 1,
            dedicated_vram: 99,
            is_nvidia: true,
            is_software: true,
        }];
        assert_eq!(
            select_rtx_adapter(&adapters),
            Err(RendererError::NoRtxAdapter)
        );
    }
    #[test]
    fn texture_ring_preserves_frame_order() {
        let mut ring = TextureRing::default();
        let a = ring.acquire_render(10).unwrap();
        let b = ring.acquire_render(11).unwrap();
        ring.finish_render(a);
        ring.finish_render(b);
        assert_eq!(ring.acquire_encode(), Some((a, 10)));
        assert_eq!(ring.acquire_encode(), Some((b, 11)));
    }
    #[test]
    fn texture_ring_backpressure_and_reset() {
        let mut ring = TextureRing::default();
        for frame in 0..EXPORT_TEXTURE_COUNT {
            assert!(ring.acquire_render(frame as u64).is_some());
        }
        assert_eq!(ring.acquire_render(99), None);
        assert_eq!(ring.occupancy(), EXPORT_TEXTURE_COUNT);
        ring.reset();
        assert_eq!(ring.occupancy(), 0);
    }
    #[test]
    fn tile_lru_evicts_least_recently_used() {
        let mut cache = TileLru::new(20);
        cache.insert(1, "a", 10);
        cache.insert(2, "b", 10);
        assert_eq!(cache.get(&1), Some(&"a"));
        cache.insert(3, "c", 10);
        assert!(cache.contains(&1));
        assert!(!cache.contains(&2));
        assert_eq!(cache.used_bytes(), 20);
    }
    #[test]
    fn tile_lru_rejects_oversize_and_replaces_accounting() {
        let mut cache = TileLru::new(10);
        cache.insert(1, "a", 6);
        cache.insert(1, "b", 4);
        assert_eq!(cache.used_bytes(), 4);
        cache.insert(2, "too big", 11);
        assert_eq!(cache.len(), 1);
    }
    #[test]
    fn required_tiles_are_stable_and_unique() {
        let tiles = required_tiles((121.4, 24.9, 121.7, 25.2), 10);
        assert!(!tiles.is_empty());
        let mut unique = std::collections::HashSet::new();
        assert!(tiles.iter().all(|tile| unique.insert(*tile)));
        assert!(tiles.iter().all(|tile| tile.zoom == 10));
    }
    #[test]
    fn frame_uses_one_complete_zoom_instead_of_mixing_tile_levels() {
        let center = [0.5, 0.5];
        let span = 0.01;
        let preferred = tile_zoom(span, 3840);
        let parent = preferred - 1;
        let mut available: HashSet<_> = required_view_tiles(center, span, parent)
            .into_iter()
            .collect();
        let mut detailed = required_view_tiles(center, span, preferred);
        detailed.pop(); // one missing detailed tile forces a whole-frame fallback
        available.extend(detailed);
        assert_eq!(
            complete_tile_zoom(&available, center, span, 3840),
            Some(parent)
        );
    }

    #[test]
    fn frame_prefers_highest_complete_zoom() {
        let center = [0.5, 0.5];
        let span = 0.01;
        let preferred = tile_zoom(span, 3840);
        let available: HashSet<_> = required_view_tiles(center, span, preferred)
            .into_iter()
            .collect();
        assert_eq!(
            complete_tile_zoom(&available, center, span, 3840),
            Some(preferred)
        );
    }
    #[test]
    fn satellite_uses_esri_xyz_order_and_separate_source() {
        let key = TileKey {
            zoom: 14,
            x: 13720,
            y: 7020,
        };
        let cache =
            TileDiskCache::new(std::env::temp_dir(), 1024).with_source(TileSource::EsriSatellite);
        assert_eq!(
            cache.tile_url(key),
            "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/14/7020/13720"
        );
        assert!(
            TileDiskCache::new(std::env::temp_dir(), 1024)
                .tile_url(key)
                .contains("openstreetmap.org")
        );
    }
    #[test]
    fn empty_cache_and_denied_network_returns_offline_tile_instead_of_gpu_error() {
        let root = std::env::temp_dir().join(format!("gpx-denied-network-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let key = TileKey {
            zoom: 17,
            x: 109773,
            y: 56579,
        };
        let tile = TileDiskCache::new(&root, 1024 * 1024)
            .load_with_fetch(key, |_| Err("os error 10013".into()))
            .unwrap();
        assert_eq!(tile.key, key);
        assert_eq!((tile.width, tile.height), (0, 0));
        assert!(tile.bgra.is_empty());
        assert!(!root.join("17/109773/56579.png").exists());
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn tile_manifest_is_sorted_deduplicated_and_counts_disk_hits() {
        let root = std::env::temp_dir().join(format!("gpx-manifest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cache = TileDiskCache::new(&root, 1024 * 1024);
        let cached = root.join("3/2/1.png");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"cached").unwrap();
        let keys = vec![
            TileKey {
                zoom: 3,
                x: 3,
                y: 1,
            },
            TileKey {
                zoom: 3,
                x: 2,
                y: 1,
            },
            TileKey {
                zoom: 3,
                x: 2,
                y: 1,
            },
        ];
        let manifest = TileManifest::new(&cache, keys);
        assert_eq!(manifest.total(), 2);
        assert_eq!(manifest.cached, 1);
        assert_eq!(manifest.missing, 1);
        assert_eq!(
            manifest.keys[0],
            TileKey {
                zoom: 3,
                x: 2,
                y: 1
            }
        );
        let _ = std::fs::remove_dir_all(root);
    }
    #[test]
    fn read_only_cache_degrades_to_placeholder() {
        let root = std::env::temp_dir().join(format!("gpx-read-only-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::write(&root, b"not a directory").unwrap();
        let key = TileKey {
            zoom: 4,
            x: 2,
            y: 3,
        };
        let cache = TileDiskCache::new(&root, 1024);
        let tile = cache.load_with_fetch(key, |_| Ok(vec![1, 2, 3])).unwrap();
        assert_eq!(tile.key, key);
        assert!(tile.bgra.is_empty());
        let _ = std::fs::remove_file(root);
    }
    #[test]
    fn view_tiles_and_zoom_cover_frame() {
        let z = tile_zoom(0.01, 3840);
        assert!((2..=18).contains(&z));
        let tiles = required_view_tiles([0.5, 0.5], 0.01, z);
        assert!(!tiles.is_empty());
    }
    #[cfg(windows)]
    #[test]
    fn enumerates_windows_adapters() {
        let adapters = enumerate_adapters().unwrap();
        assert!(!adapters.is_empty());
        assert!(
            adapters
                .iter()
                .any(|adapter| adapter.name.contains("NVIDIA"))
        );
    }
    #[cfg(windows)]
    #[test]
    fn creates_six_zero_cpu_access_4k_textures_on_rtx() {
        let device = D3d11ExportDevice::create_rtx().unwrap();
        assert!(device.info.name.contains("RTX"));
        let textures = device.create_export_textures(3840, 2160).unwrap();
        assert_eq!(textures.len(), EXPORT_TEXTURE_COUNT);
    }
    #[cfg(windows)]
    #[test]
    fn renders_route_marker_hud_and_elevation_into_d3d_texture() {
        use gpx_core::{ParseOptions, parse_gpx};
        use scene_core::{Scene, SceneOptions, build_frame};
        let track=parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"><ele>10</ele></trkpt><trkpt lat="25.01" lon="121.01"><ele>20</ele></trkpt><trkpt lat="25.02" lon="121.03"><ele>15</ele></trkpt></trkseg></trk></gpx>"#,ParseOptions::default()).unwrap();
        let scene = Scene {
            track,
            options: SceneOptions::default(),
            landmarks: Vec::new(),
            route_duration_seconds: 2.0,
        };
        let frame = build_frame(&scene, 0.5);
        let device = D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let renderer = D2dSceneRenderer::new(&device).unwrap();
        renderer
            .render(&textures[0], &frame, &scene.options, 3840, 2160)
            .unwrap();
        device.flush();
    }
}
