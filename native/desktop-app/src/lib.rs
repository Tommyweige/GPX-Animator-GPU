use nvenc_engine::{
    CancellationToken, ExportActivity, ExportMetrics, ExportProgress, ExportStage, GpuCapabilities,
};
use scene_core::{Codec, QualityPreset, SceneOptions};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
pub mod export;
pub mod project;
pub mod secrets;
pub use export::{
    ExportError, ExportOutcome, ExportRequest, detect_gpu_capabilities, load_gpx_file,
    run_native_export,
};
pub mod ui;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExportSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_seconds: u32,
    pub codec: Codec,
    pub quality: QualityPreset,
    pub scene: SceneOptions,
    #[serde(default = "default_cache_limit_bytes")]
    pub cache_limit_bytes: u64,
}

pub const fn default_cache_limit_bytes() -> u64 {
    2 * 1024 * 1024 * 1024
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiLanguage {
    #[default]
    TraditionalChinese,
    English,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPreferences {
    pub schema_version: u32,
    pub language: UiLanguage,
    pub settings: ExportSettings,
    pub last_input_directory: Option<PathBuf>,
    pub last_output_directory: Option<PathBuf>,
    pub cache_limit_bytes: u64,
    #[serde(default = "default_nearby_radius_m")]
    pub nearby_radius_m: u32,
    #[serde(default)]
    pub nearby_provider: places_core::NearbyProviderPreference,
    /// New profile selector. `nearby_provider` is retained for migration from
    /// pre-profile settings files.
    #[serde(default)]
    pub poi_profile: places_core::PoiProfile,
    #[serde(default = "default_nearby_online")]
    pub nearby_online: bool,
    #[serde(default)]
    pub gateway_base_url: Option<String>,
    #[serde(default)]
    pub ui_layout: UiLayoutPreferences,
}

/// Persisted desktop layout state.  The values are user preferences rather
/// than egui's transient panel memory so they survive closing and reopening a
/// nearby-places inspector or the application itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiLayoutPreferences {
    pub left_panel_width: f32,
    pub nearby_panel_width: f32,
    pub preview_section_open: bool,
    pub landmarks_section_open: bool,
    pub export_advanced_open: bool,
    #[serde(default)]
    pub settings_window: SettingsWindowPreferences,
}

/// Persisted position and size for the Settings dialog.
///
/// Both values are optional so older settings files keep using the normal
/// centered default until the user has opened and positioned the dialog.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsWindowPreferences {
    pub position: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
}

impl Default for UiLayoutPreferences {
    fn default() -> Self {
        Self {
            left_panel_width: 320.0,
            nearby_panel_width: 420.0,
            preview_section_open: true,
            landmarks_section_open: false,
            export_advanced_open: false,
            settings_window: SettingsWindowPreferences::default(),
        }
    }
}

pub const fn default_nearby_radius_m() -> u32 {
    places_core::DEFAULT_RADIUS_M
}

pub const fn default_nearby_online() -> bool {
    true
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            schema_version: 6,
            language: UiLanguage::default(),
            settings: ExportSettings::default(),
            last_input_directory: None,
            last_output_directory: None,
            cache_limit_bytes: default_cache_limit_bytes(),
            nearby_radius_m: default_nearby_radius_m(),
            nearby_provider: places_core::NearbyProviderPreference::default(),
            poi_profile: places_core::PoiProfile::default(),
            nearby_online: default_nearby_online(),
            gateway_base_url: None,
            ui_layout: UiLayoutPreferences::default(),
        }
    }
}

impl AppPreferences {
    pub fn path() -> PathBuf {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("GPX Animator")
            .join("settings.json")
    }

    pub fn load() -> Self {
        let path = Self::path();
        let Ok(bytes) = std::fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<Self>(&bytes) {
            Ok(mut value) => {
                value.schema_version = 6;
                if value.cache_limit_bytes == 0 {
                    value.cache_limit_bytes = default_cache_limit_bytes();
                }
                if value.settings.cache_limit_bytes == 0 {
                    value.settings.cache_limit_bytes = value.cache_limit_bytes;
                }
                value.nearby_radius_m = places_core::normalize_radius(value.nearby_radius_m);
                value
            }
            Err(_) => {
                let corrupt = path.with_extension(format!(
                    "corrupt-{}.json",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |value| value.as_secs())
                ));
                let _ = std::fs::rename(path, corrupt);
                Self::default()
            }
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::path();
        let Some(parent) = path.parent() else {
            return Ok(());
        };
        std::fs::create_dir_all(parent)?;
        let temporary = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        let mut file = std::fs::File::create(&temporary)?;
        use std::io::Write;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        match std::fs::rename(&temporary, &path) {
            Ok(()) => Ok(()),
            Err(first_error) if path.exists() => {
                std::fs::remove_file(&path)?;
                std::fs::rename(&temporary, &path).map_err(|_| first_error)
            }
            Err(error) => Err(error),
        }
    }
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            width: 3840,
            height: 2160,
            fps: 60,
            duration_seconds: 20,
            codec: Codec::Hevc,
            quality: QualityPreset::Quality,
            scene: SceneOptions::default(),
            cache_limit_bytes: default_cache_limit_bytes(),
        }
    }
}
impl ExportSettings {
    pub fn route_frames(&self) -> u64 {
        self.duration_seconds as u64 * self.fps as u64
    }
    pub fn total_frames(&self) -> u64 {
        self.route_frames()
            + if self.scene.camera_mode == scene_core::CameraMode::Follow {
                5 * self.fps as u64
            } else {
                0
            }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum JobState {
    Idle,
    Running(ExportProgress),
    Completed,
    Cancelled,
    Failed(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    pub adapter: String,
    pub adapter_luid: u64,
    pub dedicated_vram: u64,
    pub codec: String,
    pub cpu_frame_readbacks: u64,
    pub encoded_frames: u64,
    pub dropped_frames: u64,
    pub duplicated_frames: u64,
    pub render_p95_ms: f64,
    pub render_p50_ms: f64,
    pub encode_p95_ms: f64,
    pub mux_p95_ms: f64,
    pub ring_occupancy_peak: usize,
    pub peak_vram_bytes: u64,
    pub elapsed_seconds: f64,
}

pub struct AppModel {
    pub settings: ExportSettings,
    pub state: JobState,
    pub capabilities: Option<GpuCapabilities>,
    pub diagnostics: Diagnostics,
    cancellation: Option<CancellationToken>,
}
impl Default for AppModel {
    fn default() -> Self {
        Self {
            settings: ExportSettings::default(),
            state: JobState::Idle,
            capabilities: None,
            diagnostics: Diagnostics::default(),
            cancellation: None,
        }
    }
}

impl AppModel {
    pub fn can_export(&self) -> bool {
        let Some(gpu) = &self.capabilities else {
            return false;
        };
        let codec_ok = match self.settings.codec {
            Codec::Hevc => gpu.hevc,
            Codec::H264 => gpu.h264,
        };
        gpu.adapter_name.to_ascii_uppercase().contains("RTX")
            && codec_ok
            && !matches!(self.state, JobState::Running(_))
    }
    pub fn begin_export(&mut self) -> Result<CancellationToken, &'static str> {
        if !self.can_export() {
            return Err("需要具備所選 NVENC 編碼能力的 NVIDIA RTX，且不可重複匯出");
        }
        let token = CancellationToken::default();
        let progress = ExportProgress {
            stage: ExportStage::Preflight,
            activity: ExportActivity::PreparingRoute,
            completed_frames: 0,
            total_frames: self.settings.total_frames(),
            stage_completed: 0,
            stage_total: None,
            fps: None,
            eta_seconds: None,
        };
        self.cancellation = Some(token.clone());
        self.state = JobState::Running(progress);
        Ok(token)
    }
    pub fn update_progress(&mut self, progress: ExportProgress) {
        self.state = JobState::Running(progress);
    }
    pub fn cancel(&mut self) {
        if let Some(token) = self.cancellation.take() {
            token.cancel();
            self.state = JobState::Cancelled;
        }
    }
    pub fn finish(&mut self, metrics: &ExportMetrics) {
        let gpu = self.capabilities.as_ref();
        self.diagnostics = Diagnostics {
            adapter: gpu.map(|v| v.adapter_name.clone()).unwrap_or_default(),
            adapter_luid: if metrics.gpu_adapter_luid == 0 {
                gpu.map(|v| v.luid).unwrap_or_default()
            } else {
                metrics.gpu_adapter_luid
            },
            dedicated_vram: gpu.map(|v| v.dedicated_vram).unwrap_or_default(),
            codec: format!("{:?}", self.settings.codec),
            cpu_frame_readbacks: metrics.cpu_frame_readbacks,
            encoded_frames: metrics.encoded_frames,
            dropped_frames: metrics.dropped_frames,
            duplicated_frames: metrics.duplicated_frames,
            render_p95_ms: metrics.render_p95_ms,
            render_p50_ms: metrics.render_p50_ms,
            encode_p95_ms: metrics.encode_p95_ms,
            mux_p95_ms: metrics.mux_p95_ms,
            ring_occupancy_peak: metrics.ring_occupancy_peak,
            peak_vram_bytes: metrics.peak_vram_bytes,
            elapsed_seconds: metrics.elapsed_seconds,
        };
        self.cancellation = None;
        self.state = JobState::Completed;
    }
    pub fn fail(&mut self, message: impl Into<String>) {
        self.cancellation = None;
        self.state = JobState::Failed(message.into());
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    fn gpu(name: &str) -> GpuCapabilities {
        GpuCapabilities {
            adapter_name: name.into(),
            luid: 7,
            dedicated_vram: 11,
            hevc: true,
            h264: true,
            max_width: 8192,
            max_height: 8192,
            async_encode: true,
            api_version: "test".into(),
        }
    }
    #[test]
    fn defaults_match_product_requirements() {
        let app = AppModel::default();
        assert_eq!(
            (app.settings.width, app.settings.height, app.settings.fps),
            (3840, 2160, 60)
        );
        assert_eq!(app.settings.scene.line_width_px, 8.0);
        assert_eq!(
            app.settings.scene.map_style,
            scene_core::MapStyle::Satellite
        );
        assert_eq!(
            app.settings.scene.camera_mode,
            scene_core::CameraMode::Follow
        );
        assert_eq!(app.settings.quality, scene_core::QualityPreset::Quality);
    }
    #[test]
    fn follow_export_appends_transition_and_hold() {
        let mut settings = ExportSettings::default();
        settings.duration_seconds = 20;
        settings.fps = 60;
        settings.scene.camera_mode = scene_core::CameraMode::Follow;
        assert_eq!(settings.route_frames(), 1200);
        assert_eq!(settings.total_frames(), 1500);
        settings.scene.camera_mode = scene_core::CameraMode::Fit;
        assert_eq!(settings.total_frames(), 1200);
    }
    #[test]
    fn rejects_intel_and_duplicate_export() {
        let mut app = AppModel::default();
        app.capabilities = Some(gpu("Intel UHD"));
        assert!(!app.can_export());
        app.capabilities = Some(gpu("NVIDIA RTX 2080 Ti"));
        assert!(app.begin_export().is_ok());
        assert!(app.begin_export().is_err());
    }
    #[test]
    fn cancel_is_idempotent() {
        let mut app = AppModel::default();
        app.capabilities = Some(gpu("NVIDIA RTX 2080 Ti"));
        let token = app.begin_export().unwrap();
        app.cancel();
        app.cancel();
        assert!(token.is_cancelled());
        assert_eq!(app.state, JobState::Cancelled);
    }
    #[test]
    fn completion_exposes_diagnostics() {
        let mut app = AppModel::default();
        app.capabilities = Some(gpu("NVIDIA RTX 2080 Ti"));
        let mut m = ExportMetrics::default();
        m.encoded_frames = 120;
        app.finish(&m);
        assert_eq!(app.diagnostics.encoded_frames, 120);
        assert_eq!(app.diagnostics.adapter_luid, 7);
        assert_eq!(app.diagnostics.cpu_frame_readbacks, 0);
    }
    #[test]
    fn failure_allows_retry() {
        let mut app = AppModel::default();
        app.capabilities = Some(gpu("NVIDIA RTX 2080 Ti"));
        app.begin_export().unwrap();
        app.fail("device lost");
        assert!(matches!(app.state, JobState::Failed(_)));
        assert!(app.can_export());
    }
    #[test]
    fn preferences_round_trip_preserves_language_and_cache_policy() {
        let mut value = AppPreferences::default();
        value.language = UiLanguage::English;
        value.settings.cache_limit_bytes = 512 * 1024 * 1024;
        value.cache_limit_bytes = value.settings.cache_limit_bytes;
        value.nearby_radius_m = 5_000;
        value.nearby_provider = places_core::NearbyProviderPreference::GoogleFirst;
        value.ui_layout.nearby_panel_width = 512.0;
        value.ui_layout.preview_section_open = false;
        let bytes = serde_json::to_vec(&value).unwrap();
        let decoded: AppPreferences = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded.language, UiLanguage::English);
        assert_eq!(decoded.settings.cache_limit_bytes, 512 * 1024 * 1024);
        assert_eq!(decoded.nearby_radius_m, 5_000);
        assert_eq!(
            decoded.nearby_provider,
            places_core::NearbyProviderPreference::GoogleFirst
        );
        assert_eq!(decoded.ui_layout.nearby_panel_width, 512.0);
        assert!(!decoded.ui_layout.preview_section_open);
        assert!(!String::from_utf8(bytes).unwrap().contains("api_key"));
    }

    #[test]
    fn layout_defaults_are_compact_and_persistable() {
        let value = UiLayoutPreferences::default();
        assert_eq!(value.left_panel_width, 320.0);
        assert_eq!(value.nearby_panel_width, 420.0);
        assert!(value.preview_section_open);
        assert!(!value.landmarks_section_open);
        assert!(!value.export_advanced_open);
        let json = serde_json::to_string(&value).unwrap();
        let decoded: UiLayoutPreferences = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn old_preferences_without_radius_migrate_to_default() {
        let mut json = serde_json::to_value(AppPreferences::default()).unwrap();
        json.as_object_mut().unwrap().remove("nearby_radius_m");
        let value: AppPreferences = serde_json::from_value(json).unwrap();
        assert_eq!(value.nearby_radius_m, places_core::DEFAULT_RADIUS_M);
        assert_eq!(value.schema_version, 6);
        assert_eq!(
            value.nearby_provider,
            places_core::NearbyProviderPreference::TomTomFirst
        );
    }
}
