use nvenc_engine::{
    CancellationToken, ExportMetrics, ExportProgress, ExportStage, GpuCapabilities,
};
use scene_core::{Codec, QualityPreset, SceneOptions};
pub mod export;
pub use export::{
    ExportError, ExportOutcome, ExportRequest, detect_gpu_capabilities, load_gpx_file,
    run_native_export,
};
pub mod ui;

#[derive(Clone, Debug, PartialEq)]
pub struct ExportSettings {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_seconds: u32,
    pub codec: Codec,
    pub quality: QualityPreset,
    pub scene: SceneOptions,
}
impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            width: 3840,
            height: 2160,
            fps: 60,
            duration_seconds: 20,
            codec: Codec::Hevc,
            quality: QualityPreset::Balanced,
            scene: SceneOptions::default(),
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
            completed_frames: 0,
            total_frames: self.settings.total_frames(),
            fps: 0.0,
            eta_seconds: 0.0,
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
        assert_eq!(app.settings.scene.line_width_px, 6.0);
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
}
