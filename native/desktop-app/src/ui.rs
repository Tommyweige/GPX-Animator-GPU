use crate::{
    AppModel, AppPreferences, Diagnostics, ExportOutcome, ExportProgress, ExportRequest,
    ExportSettings, JobState, UiLanguage, detect_gpu_capabilities, load_gpx_file,
    run_native_export,
};
use eframe::egui;
use gpx_core::{ParseOptions, Track};
use nvenc_engine::CancellationToken;
use scene_core::{Aspect, CameraMode, Codec, MapStyle, QualityPreset, Scene, build_frame};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

enum WorkerMessage {
    Progress(ExportProgress),
    Finished(Result<ExportOutcome, String>),
}
enum TileMessage {
    Loaded(MapStyle, d3d11_renderer::DecodedTile),
    Failed(d3d11_renderer::TileKey),
}

pub struct NativeApp {
    model: AppModel,
    track: Option<Track>,
    gpx_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    preview_progress: f64,
    receiver: Option<Receiver<WorkerMessage>>,
    gpu_receiver: Option<Receiver<Result<nvenc_engine::GpuCapabilities, String>>>,
    active_token: Option<CancellationToken>,
    last_error: Option<String>,
    show_diagnostics: bool,
    tile_tx: Sender<TileMessage>,
    tile_rx: Receiver<TileMessage>,
    preview_tiles: HashMap<d3d11_renderer::TileKey, egui::TextureHandle>,
    pending_tiles: HashSet<d3d11_renderer::TileKey>,
    preview_map_style: MapStyle,
    language: UiLanguage,
    show_settings: bool,
    preferences: AppPreferences,
}

fn current_long_edge(settings: &ExportSettings) -> u32 {
    match settings.scene.aspect {
        Aspect::Portrait => settings.height,
        Aspect::Landscape | Aspect::Square => settings.width,
    }
}

fn apply_long_edge(settings: &mut ExportSettings, long_edge: u32) {
    let long_edge = long_edge.clamp(320, 8192);
    match settings.scene.aspect {
        Aspect::Landscape => {
            settings.width = long_edge;
            settings.height = long_edge * 9 / 16;
        }
        Aspect::Square => {
            settings.width = long_edge;
            settings.height = long_edge;
        }
        Aspect::Portrait => {
            settings.width = long_edge * 9 / 16;
            settings.height = long_edge;
        }
    }
}

fn resolution_label(long_edge: u32, language: UiLanguage) -> String {
    if language == UiLanguage::English {
        return match long_edge {
            3840 => "4K · 3840".to_owned(),
            2560 => "2.5K · 2560".to_owned(),
            1920 => "1080p · 1920".to_owned(),
            1280 => "720p · 1280".to_owned(),
            value => format!("Custom · {value}"),
        };
    }
    match long_edge {
        3840 => "4K · 3840".to_owned(),
        2560 => "2.5K · 2560".to_owned(),
        1920 => "1080p · 1920".to_owned(),
        1280 => "720p · 1280".to_owned(),
        value => format!("自訂 · {value}"),
    }
}

fn localized_error(language: UiLanguage, message: &str) -> String {
    if language == UiLanguage::TraditionalChinese {
        return message.to_owned();
    }
    if message.contains("需要具備所選 NVENC") {
        return "A compatible NVIDIA RTX/NVENC adapter is required; duplicate exports are disabled."
            .to_owned();
    }
    if message.contains("請先載入 GPX") {
        return "Load a GPX track and choose an output file first.".to_owned();
    }
    if message.contains("匯出已取消") {
        return "Export cancelled.".to_owned();
    }
    if message.contains("讀取 GPX 失敗") {
        return message.replacen("讀取 GPX 失敗", "Failed to read GPX", 1);
    }
    if message.contains("地圖圖磚失敗") {
        return message.replacen("地圖圖磚失敗", "Map tile failed", 1);
    }
    message.to_owned()
}

impl NativeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        install_chinese_font(&cc.egui_ctx);
        let preferences = AppPreferences::load();
        let mut settings = preferences.settings.clone();
        settings.cache_limit_bytes = preferences
            .cache_limit_bytes
            .max(crate::default_cache_limit_bytes() / 8);
        let model = AppModel {
            settings,
            ..AppModel::default()
        };
        let (gpu_tx, gpu_receiver) = channel();
        std::thread::spawn(move || {
            let result = detect_gpu_capabilities().map_err(|error| error.to_string());
            let _ = gpu_tx.send(result);
        });
        let (tile_tx, tile_rx) = channel();
        let preview_map_style = model.settings.scene.map_style;
        Self {
            model,
            track: None,
            gpx_path: None,
            output_path: None,
            preview_progress: 0.0,
            receiver: None,
            gpu_receiver: Some(gpu_receiver),
            active_token: None,
            last_error: None,
            show_diagnostics: false,
            tile_tx,
            tile_rx,
            preview_tiles: HashMap::new(),
            pending_tiles: HashSet::new(),
            preview_map_style,
            language: preferences.language,
            show_settings: false,
            preferences,
        }
    }

    pub fn new_with_path(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        let mut app = Self::new(cc);
        if let Some(path) = path {
            app.load_gpx(path);
        }
        app
    }

    fn load_gpx(&mut self, path: PathBuf) {
        match load_gpx_file(&path, ParseOptions::default()) {
            Ok(track) => {
                // A saved free-camera center belongs to the previous route and
                // can place a newly loaded GPX completely outside the viewport.
                // Start every track at a deterministic visible camera; users
                // can still choose Fit or Free after the route is displayed.
                reset_camera_for_new_track(&mut self.model.settings.scene);
                self.output_path = Some(path.with_extension("mp4"));
                self.gpx_path = Some(path);
                self.track = Some(track);
                if let Some(input) = self.gpx_path.as_ref().and_then(|value| value.parent()) {
                    self.preferences.last_input_directory = Some(input.to_path_buf());
                }
                self.preview_tiles.clear();
                self.pending_tiles.clear();
                self.last_error = None
            }
            Err(error) => self.last_error = Some(error.to_string()),
        }
    }

    fn poll_tiles(&mut self, ctx: &egui::Context) {
        while let Ok(message) = self.tile_rx.try_recv() {
            match message {
                TileMessage::Loaded(style, tile)
                    if style == self.model.settings.scene.map_style =>
                {
                    let key = tile.key;
                    if tile.bgra.is_empty() || tile.width == 0 || tile.height == 0 {
                        self.pending_tiles.remove(&key);
                        continue;
                    }
                    let mut rgba = tile.bgra;
                    for pixel in rgba.chunks_exact_mut(4) {
                        pixel.swap(0, 2)
                    }
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [tile.width as usize, tile.height as usize],
                        &rgba,
                    );
                    self.preview_tiles.insert(
                        key,
                        ctx.load_texture(
                            format!("osm-{}-{}-{}", key.zoom, key.x, key.y),
                            image,
                            egui::TextureOptions::LINEAR,
                        ),
                    );
                    self.pending_tiles.remove(&key);
                    ctx.request_repaint();
                }
                TileMessage::Loaded(_, tile) => {
                    self.pending_tiles.remove(&tile.key);
                }
                TileMessage::Failed(key) => {
                    self.pending_tiles.remove(&key);
                }
            }
        }
    }

    fn request_tiles(&mut self, ctx: &egui::Context, keys: &[d3d11_renderer::TileKey]) {
        let style = self.model.settings.scene.map_style;
        for &key in keys {
            if self.preview_tiles.contains_key(&key) || !self.pending_tiles.insert(key) {
                continue;
            }
            let tx = self.tile_tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let cache = d3d11_renderer::TileDiskCache::for_map_style(style);
                let message = match cache.load(key) {
                    Ok(tile) => TileMessage::Loaded(style, tile),
                    Err(_) => TileMessage::Failed(key),
                };
                let _ = tx.send(message);
                ctx.request_repaint();
            });
        }
    }

    fn start_export(&mut self) {
        let (Some(track), Some(output)) = (self.track.clone(), self.output_path.clone()) else {
            self.last_error = Some("請先載入 GPX 並選擇輸出檔案".into());
            return;
        };
        self.model.settings = self.validated_settings();
        let token = match self.model.begin_export() {
            Ok(value) => value,
            Err(error) => {
                self.last_error = Some(error.into());
                return;
            }
        };
        let request = ExportRequest {
            track,
            output_path: output,
            settings: self.model.settings.clone(),
        };
        let (tx, rx) = channel();
        let worker_token = token.clone();
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = run_native_export(request, &worker_token, move |value| {
                let _ = progress_tx.send(WorkerMessage::Progress(value));
            })
            .map_err(|error| error.to_string());
            let _ = tx.send(WorkerMessage::Finished(result));
        });
        self.receiver = Some(rx);
        self.active_token = Some(token);
        self.last_error = None;
    }

    fn validated_settings(&self) -> ExportSettings {
        let mut value = self.model.settings.clone();
        let long_edge = current_long_edge(&value).clamp(320, 8192);
        apply_long_edge(&mut value, long_edge);
        value.fps = match value.fps {
            24 | 30 | 60 | 120 => value.fps,
            _ => 60,
        };
        value.duration_seconds = value.duration_seconds.clamp(1, 3600);
        value.scene.line_width_px = value.scene.line_width_px.clamp(1.0, 32.0);
        value
    }

    fn poll_worker(&mut self) {
        if let Some(receiver) = &self.gpu_receiver
            && let Ok(result) = receiver.try_recv()
        {
            match result {
                Ok(value) => self.model.capabilities = Some(value),
                Err(error) => {
                    self.last_error = Some(error.clone());
                    self.model.state = JobState::Failed(error);
                }
            }
            self.gpu_receiver = None;
        }
        let mut messages = Vec::new();
        if let Some(receiver) = &self.receiver {
            while let Ok(message) = receiver.try_recv() {
                messages.push(message)
            }
        }
        for message in messages {
            match message {
                WorkerMessage::Progress(value) => self.model.update_progress(value),
                WorkerMessage::Finished(result) => {
                    match result {
                        Ok(outcome) => self.model.finish(&outcome.metrics),
                        Err(_)
                            if self
                                .active_token
                                .as_ref()
                                .is_some_and(|token| token.is_cancelled()) =>
                        {
                            self.model.state = JobState::Cancelled
                        }
                        Err(error) => {
                            self.last_error = Some(error.clone());
                            self.model.fail(error)
                        }
                    }
                    self.receiver = None;
                    self.active_token = None;
                }
            }
        }
    }

    fn draw_header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("GPX Animator");
                ui.label(
                    egui::RichText::new("GPU EDITION").color(egui::Color32::from_rgb(255, 93, 59)),
                );
                let settings_label = match self.language {
                    UiLanguage::TraditionalChinese => "設定",
                    UiLanguage::English => "Settings",
                };
                if ui.button(settings_label).clicked() {
                    self.show_settings = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let diagnostics_label = match self.language {
                        UiLanguage::TraditionalChinese => "GPU 診斷",
                        UiLanguage::English => "GPU Diagnostics",
                    };
                    if ui.button(diagnostics_label).clicked() {
                        self.show_diagnostics = !self.show_diagnostics
                    }
                    if let Some(gpu) = &self.model.capabilities {
                        ui.label(format!("NVENC · {}", gpu.adapter_name));
                    } else if self.gpu_receiver.is_some() {
                        ui.spinner();
                        ui.label(match self.language {
                            UiLanguage::TraditionalChinese => "正在偵測 RTX / NVENC…",
                            UiLanguage::English => "Detecting RTX / NVENC…",
                        });
                    }
                });
            });
        });
    }

    fn draw_controls(&mut self, ctx: &egui::Context) {
        if self.language == UiLanguage::English {
            self.draw_controls_english(ctx);
            return;
        }
        egui::SidePanel::left("controls")
            .resizable(true)
            .default_width(340.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("01 載入軌跡");
                    if ui.button("選擇 GPX 檔案…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("GPX 軌跡", &["gpx"])
                            .pick_file()
                    {
                        self.load_gpx(path)
                    }
                    if let Some(track) = &self.track {
                        ui.group(|ui| {
                            ui.strong(&track.name);
                            ui.horizontal(|ui| {
                                ui.label(format!("{:.2} km", track.distance_m / 1000.0));
                                ui.label(format!("爬升 {:.0} m", track.elevation_gain_m));
                                ui.label(format!("GPS {} 點", track.source_point_count));
                            });
                            ui.small(format!(
                                "偵測到 {} 筆停留記錄；勻速動畫不使用停留時間。",
                                track.removed_stop_points
                            ));
                        });
                    }
                    ui.separator();
                    ui.heading("02 畫面設定");
                    egui::ComboBox::from_label("比例")
                        .selected_text(match self.model.settings.scene.aspect {
                            Aspect::Landscape => "16:9",
                            Aspect::Square => "1:1",
                            Aspect::Portrait => "9:16",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.scene.aspect,
                                Aspect::Landscape,
                                "16:9",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.aspect,
                                Aspect::Square,
                                "1:1",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.aspect,
                                Aspect::Portrait,
                                "9:16",
                            );
                        });
                    let mut long_edge = current_long_edge(&self.model.settings);
                    egui::ComboBox::from_label("解析度")
                        .selected_text(resolution_label(long_edge, self.language))
                        .show_ui(ui, |ui| {
                            for (edge, label) in [
                                (3840, "4K · 3840"),
                                (2560, "2.5K · 2560"),
                                (1920, "1080p · 1920"),
                                (1280, "720p · 1280"),
                            ] {
                                if ui.selectable_label(long_edge == edge, label).clicked() {
                                    long_edge = edge;
                                }
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.label("自訂長邊");
                        ui.add(
                            egui::DragValue::new(&mut long_edge)
                                .range(320..=8192)
                                .speed(16),
                        );
                    });
                    apply_long_edge(&mut self.model.settings, long_edge);
                    egui::ComboBox::from_label("地圖樣式")
                        .selected_text(match self.model.settings.scene.map_style {
                            MapStyle::Light => "淺色地圖",
                            MapStyle::Dark => "深色地圖",
                            MapStyle::Satellite => "衛星圖",
                            MapStyle::Transparent => "透明背景",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Light,
                                "淺色地圖",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Dark,
                                "深色地圖",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Satellite,
                                "衛星圖",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Transparent,
                                "透明背景",
                            );
                        });
                    egui::ComboBox::from_label("攝影機")
                        .selected_text(match self.model.settings.scene.camera_mode {
                            CameraMode::Fit => "顯示完整路線",
                            CameraMode::Follow => "跟隨標記",
                            CameraMode::Free => "自由拖曳／縮放",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.scene.camera_mode,
                                CameraMode::Fit,
                                "顯示完整路線",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.camera_mode,
                                CameraMode::Follow,
                                "跟隨標記",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.camera_mode,
                                CameraMode::Free,
                                "自由拖曳／縮放",
                            );
                        });
                    ui.add(
                        egui::Slider::new(&mut self.model.settings.scene.line_width_px, 1.0..=16.0)
                            .text("路線寬度 px"),
                    );
                    ui.checkbox(&mut self.model.settings.scene.show_hud, "顯示 HUD");
                    ui.checkbox(&mut self.model.settings.scene.show_elevation, "顯示海拔圖");
                    ui.separator();
                    ui.heading("03 輸出");
                    egui::ComboBox::from_label("Codec")
                        .selected_text(match self.model.settings.codec {
                            Codec::Hevc => "H.265 / HEVC",
                            Codec::H264 => "H.264 / AVC",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.codec,
                                Codec::Hevc,
                                "H.265 / HEVC",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.codec,
                                Codec::H264,
                                "H.264 / AVC",
                            );
                        });
                    egui::ComboBox::from_label("品質")
                        .selected_text(match self.model.settings.quality {
                            QualityPreset::Balanced => "平衡 P4 / CQ22",
                            QualityPreset::Quality => "高畫質 P5 / CQ19",
                            QualityPreset::Speed => "高速 P3 / CQ25",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.quality,
                                QualityPreset::Balanced,
                                "平衡 P4 / CQ22",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.quality,
                                QualityPreset::Quality,
                                "高畫質 P5 / CQ19",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.quality,
                                QualityPreset::Speed,
                                "高速 P3 / CQ25",
                            );
                        });
                    ui.horizontal(|ui| {
                        ui.label("影片秒數");
                        ui.add(
                            egui::DragValue::new(&mut self.model.settings.duration_seconds)
                                .range(1..=3600),
                        );
                    });
                    egui::ComboBox::from_label("幀數 (FPS)")
                        .selected_text(format!("{} FPS", self.model.settings.fps))
                        .show_ui(ui, |ui| {
                            for fps in [24, 30, 60, 120] {
                                ui.selectable_value(
                                    &mut self.model.settings.fps,
                                    fps,
                                    format!("{fps} FPS"),
                                );
                            }
                        });
                    if ui.button("選擇輸出 MP4…").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("MP4 影片", &["mp4"])
                            .set_file_name("gpx-animation.mp4")
                            .save_file()
                    {
                        self.preferences.last_output_directory = path.parent().map(PathBuf::from);
                        self.output_path = Some(path)
                    }
                    if let Some(path) = &self.output_path {
                        ui.small(path.display().to_string());
                    }
                    match &self.model.state {
                        JobState::Running(value) => {
                            if value.stage == nvenc_engine::ExportStage::Preflight {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("正在讀取本地地圖快取並補齊缺少圖磚…");
                                });
                            }
                            let ratio = if value.stage_total == 0 {
                                0.0
                            } else {
                                value.stage_completed as f32 / value.stage_total as f32
                            };
                            let status = if value.stage == nvenc_engine::ExportStage::Preflight {
                                format!("地圖圖磚 {}/{}", value.stage_completed, value.stage_total)
                            } else {
                                format!(
                                    "影片 {} FPS · 輸出 {:.1} fps ({:.2}×) · ETA {:.1}s",
                                    self.model.settings.fps,
                                    value.fps,
                                    value.fps / self.model.settings.fps as f64,
                                    value.eta_seconds
                                )
                            };
                            ui.add(egui::ProgressBar::new(ratio).show_percentage().text(status));
                            if ui.button("取消匯出").clicked() {
                                if let Some(token) = &self.active_token {
                                    token.cancel()
                                }
                                self.model.cancel();
                            }
                        }
                        _ => {
                            if ui
                                .add_enabled(
                                    self.track.is_some() && self.model.can_export(),
                                    egui::Button::new(format!(
                                        "輸出 4K{} MP4",
                                        self.model.settings.fps
                                    )),
                                )
                                .clicked()
                            {
                                self.start_export();
                            }
                        }
                    }
                    if let Some(error) = &self.last_error {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            localized_error(self.language, error),
                        );
                    }
                });
            });
    }

    fn draw_controls_english(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("controls")
            .resizable(true)
            .default_width(340.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.heading("01  Load track");
                    if ui.button("Choose GPX file").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("GPX track", &["gpx"])
                            .pick_file()
                    {
                        self.load_gpx(path);
                    }
                    if let Some(track) = &self.track {
                        ui.group(|ui| {
                            ui.strong(&track.name);
                            ui.horizontal(|ui| {
                                ui.label(format!("{:.2} km", track.distance_m / 1000.0));
                                ui.label(format!("Gain {:.0} m", track.elevation_gain_m));
                                ui.label(format!("GPS points {}", track.source_point_count));
                            });
                            ui.small(format!("Filtered stops: {}", track.removed_stop_points));
                        });
                    }
                    ui.separator();
                    ui.heading("02  Scene");
                    egui::ComboBox::from_label("Aspect ratio")
                        .selected_text(match self.model.settings.scene.aspect {
                            Aspect::Landscape => "16:9",
                            Aspect::Square => "1:1",
                            Aspect::Portrait => "9:16",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.scene.aspect,
                                Aspect::Landscape,
                                "16:9",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.aspect,
                                Aspect::Square,
                                "1:1",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.aspect,
                                Aspect::Portrait,
                                "9:16",
                            );
                        });
                    let mut long_edge = current_long_edge(&self.model.settings);
                    egui::ComboBox::from_label("Resolution")
                        .selected_text(resolution_label(long_edge, UiLanguage::English))
                        .show_ui(ui, |ui| {
                            for (edge, label) in [
                                (3840, "4K · 3840"),
                                (2560, "2.5K · 2560"),
                                (1920, "1080p · 1920"),
                                (1280, "720p · 1280"),
                            ] {
                                if ui.selectable_label(long_edge == edge, label).clicked() {
                                    long_edge = edge;
                                }
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.label("Custom long edge");
                        ui.add(
                            egui::DragValue::new(&mut long_edge)
                                .range(320..=8192)
                                .speed(16),
                        );
                    });
                    apply_long_edge(&mut self.model.settings, long_edge);
                    egui::ComboBox::from_label("Map style")
                        .selected_text(match self.model.settings.scene.map_style {
                            MapStyle::Light => "Light",
                            MapStyle::Dark => "Dark",
                            MapStyle::Satellite => "Satellite",
                            MapStyle::Transparent => "Transparent",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Light,
                                "Light",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Dark,
                                "Dark",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Satellite,
                                "Satellite",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.map_style,
                                MapStyle::Transparent,
                                "Transparent",
                            );
                        });
                    egui::ComboBox::from_label("Camera")
                        .selected_text(match self.model.settings.scene.camera_mode {
                            CameraMode::Fit => "Fit route",
                            CameraMode::Follow => "Follow route",
                            CameraMode::Free => "Free pan / zoom",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.scene.camera_mode,
                                CameraMode::Fit,
                                "Fit route",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.camera_mode,
                                CameraMode::Follow,
                                "Follow route",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.scene.camera_mode,
                                CameraMode::Free,
                                "Free pan / zoom",
                            );
                        });
                    ui.add(
                        egui::Slider::new(&mut self.model.settings.scene.line_width_px, 1.0..=16.0)
                            .text("Route width (px)"),
                    );
                    ui.checkbox(&mut self.model.settings.scene.show_hud, "Show HUD");
                    ui.checkbox(
                        &mut self.model.settings.scene.show_elevation,
                        "Show elevation profile",
                    );
                    ui.separator();
                    ui.heading("03  Export");
                    egui::ComboBox::from_label("Codec")
                        .selected_text(match self.model.settings.codec {
                            Codec::Hevc => "H.265 / HEVC",
                            Codec::H264 => "H.264 / AVC",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.codec,
                                Codec::Hevc,
                                "H.265 / HEVC",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.codec,
                                Codec::H264,
                                "H.264 / AVC",
                            );
                        });
                    egui::ComboBox::from_label("Quality")
                        .selected_text(match self.model.settings.quality {
                            QualityPreset::Balanced => "Balanced",
                            QualityPreset::Quality => "High",
                            QualityPreset::Speed => "Speed",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.model.settings.quality,
                                QualityPreset::Balanced,
                                "Balanced",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.quality,
                                QualityPreset::Quality,
                                "High",
                            );
                            ui.selectable_value(
                                &mut self.model.settings.quality,
                                QualityPreset::Speed,
                                "Speed",
                            );
                        });
                    ui.horizontal(|ui| {
                        ui.label("Route seconds");
                        ui.add(
                            egui::DragValue::new(&mut self.model.settings.duration_seconds)
                                .range(1..=3600),
                        );
                    });
                    egui::ComboBox::from_label("Frame rate")
                        .selected_text(format!("{} FPS", self.model.settings.fps))
                        .show_ui(ui, |ui| {
                            for fps in [24, 30, 60, 120] {
                                ui.selectable_value(
                                    &mut self.model.settings.fps,
                                    fps,
                                    format!("{fps} FPS"),
                                );
                            }
                        });
                    if ui.button("Choose output MP4").clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("MP4 video", &["mp4"])
                            .set_file_name("gpx-animation.mp4")
                            .save_file()
                    {
                        self.preferences.last_output_directory = path.parent().map(PathBuf::from);
                        self.output_path = Some(path);
                    }
                    if let Some(path) = &self.output_path {
                        ui.small(path.display().to_string());
                    }
                    match &self.model.state {
                        JobState::Running(value) => {
                            if value.stage == nvenc_engine::ExportStage::Preflight {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Preparing cached map tiles…");
                                });
                            }
                            let ratio = if value.stage_total == 0 {
                                0.0
                            } else {
                                value.stage_completed as f32 / value.stage_total as f32
                            };
                            let status = if value.stage == nvenc_engine::ExportStage::Preflight {
                                format!(
                                    "Map tiles {} / {}",
                                    value.stage_completed, value.stage_total
                                )
                            } else {
                                format!(
                                    "{} / {} · {:.1} FPS · ETA {:.1}s",
                                    value.stage_completed,
                                    value.stage_total,
                                    value.fps,
                                    value.eta_seconds
                                )
                            };
                            ui.add(egui::ProgressBar::new(ratio).show_percentage().text(status));
                            if ui.button("Cancel").clicked() {
                                if let Some(token) = &self.active_token {
                                    token.cancel();
                                }
                                self.model.cancel();
                            }
                        }
                        _ => {
                            if ui
                                .add_enabled(
                                    self.track.is_some() && self.model.can_export(),
                                    egui::Button::new(format!(
                                        "Export {} FPS MP4",
                                        self.model.settings.fps
                                    )),
                                )
                                .clicked()
                            {
                                self.start_export();
                            }
                        }
                    }
                    if let Some(error) = &self.last_error {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            localized_error(self.language, error),
                        );
                    }
                });
            });
    }

    fn draw_preview(&mut self, ctx: &egui::Context) {
        if self.preview_map_style != self.model.settings.scene.map_style {
            self.preview_map_style = self.model.settings.scene.map_style;
            self.preview_tiles.clear();
            self.pending_tiles.clear();
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.language == UiLanguage::English {
                ui.horizontal(|ui| {
                    ui.label("Timeline");
                    ui.add(
                        egui::Slider::new(&mut self.preview_progress, 0.0..=1.0).show_value(false),
                    );
                    ui.label("Drag to pan · Scroll to zoom");
                });
            }
            if self.language == UiLanguage::TraditionalChinese {
                ui.horizontal(|ui| {
                    ui.label("預覽位置");
                    ui.add(
                        egui::Slider::new(&mut self.preview_progress, 0.0..=1.0).show_value(false),
                    );
                    ui.label("拖曳平移 · 滾輪縮放");
                });
            }
            let available = ui.available_size();
            let (response, painter) = ui.allocate_painter(available, egui::Sense::drag());
            let rect = response.rect;
            let frame = scene_core::fit_aspect_rect(
                available.x,
                available.y,
                self.model.settings.scene.aspect,
            );
            let frame_rect = egui::Rect::from_min_size(
                rect.min + egui::vec2(frame[0], frame[1]),
                egui::vec2(frame[2], frame[3]),
            );
            let background = match self.model.settings.scene.map_style {
                MapStyle::Light => egui::Color32::from_rgb(232, 236, 232),
                MapStyle::Dark => egui::Color32::from_rgb(26, 35, 42),
                MapStyle::Satellite => egui::Color32::from_rgb(24, 28, 32),
                MapStyle::Transparent => egui::Color32::from_gray(45),
            };
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(12, 16, 20));
            painter.rect_filled(frame_rect, 8.0, background);
            let Some(track) = &self.track else {
                painter.text(
                    frame_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    match self.language {
                        UiLanguage::TraditionalChinese => "載入 GPX 後顯示預覽",
                        UiLanguage::English => "Load a GPX track to preview",
                    },
                    egui::FontId::proportional(22.0),
                    egui::Color32::GRAY,
                );
                return;
            };
            if response.dragged() {
                let delta = ctx.input(|input| input.pointer.delta());
                apply_pan(
                    &mut self.model.settings.scene,
                    track,
                    delta,
                    frame_rect.size(),
                );
            }
            if response.hovered() {
                let scroll = ctx.input(|input| input.smooth_scroll_delta.y);
                if scroll.abs() > 0.0 {
                    self.model.settings.scene.camera_mode = CameraMode::Free;
                    self.model.settings.scene.camera_zoom = (self.model.settings.scene.camera_zoom
                        * (scroll as f64 * 0.002).exp())
                    .clamp(0.25, 64.0);
                }
            }
            let scene = Scene {
                track: track.clone(),
                options: self.model.settings.scene.clone(),
            };
            let frame = build_frame(&scene, self.preview_progress);
            let zoom = d3d11_renderer::tile_zoom(frame.view_span, frame_rect.width() as u32);
            let keys = d3d11_renderer::required_view_tiles(
                frame.view_center_mercator,
                frame.view_span,
                zoom,
            );
            self.request_tiles(ctx, &keys);
            let painter = painter.with_clip_rect(frame_rect);
            let n = (1u32 << zoom) as f64;
            for key in &keys {
                if let Some(texture) = self.preview_tiles.get(key) {
                    let map = |x: f64, y: f64| {
                        egui::pos2(
                            frame_rect.center().x
                                + ((x - frame.view_center_mercator[0]) * 2.0 / frame.view_span)
                                    as f32
                                    * frame_rect.width()
                                    * 0.5,
                            frame_rect.center().y
                                - (-(y - frame.view_center_mercator[1]) * 2.0 / frame.view_span)
                                    as f32
                                    * frame_rect.height()
                                    * 0.5,
                        )
                    };
                    let min = map(key.x as f64 / n, key.y as f64 / n);
                    let max = map((key.x + 1) as f64 / n, (key.y + 1) as f64 / n);
                    painter.image(
                        texture.id(),
                        egui::Rect::from_min_max(min, max),
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
            }
            let to_screen = |point: [f32; 2]| {
                egui::pos2(
                    frame_rect.center().x + point[0] * frame_rect.width() * 0.5,
                    frame_rect.center().y - point[1] * frame_rect.height() * 0.5,
                )
            };
            let route: Vec<_> = frame.route_ndc.iter().copied().map(to_screen).collect();
            let completed = route
                .iter()
                .take(frame.completed_points)
                .copied()
                .collect::<Vec<_>>();
            painter.add(egui::Shape::line(
                route,
                egui::Stroke::new(
                    self.model.settings.scene.line_width_px,
                    egui::Color32::from_gray(135),
                ),
            ));
            painter.add(egui::Shape::line(
                completed,
                egui::Stroke::new(
                    self.model.settings.scene.line_width_px,
                    egui::Color32::from_rgba_unmultiplied(
                        self.model.settings.scene.route_color[0],
                        self.model.settings.scene.route_color[1],
                        self.model.settings.scene.route_color[2],
                        self.model.settings.scene.route_color[3],
                    ),
                ),
            ));
            painter.circle_filled(
                to_screen(frame.marker_ndc),
                9.0,
                egui::Color32::from_rgb(255, 93, 59),
            );
            if self.model.settings.scene.show_elevation && frame.elevation_line.len() > 1 {
                let overlay = scene_core::overlay_layout(self.model.settings.scene.aspect);
                let chart = egui::Rect::from_min_max(
                    egui::pos2(
                        frame_rect.left() + frame_rect.width() * overlay.elevation[0],
                        frame_rect.top() + frame_rect.height() * overlay.elevation[1],
                    ),
                    egui::pos2(
                        frame_rect.left()
                            + frame_rect.width() * (overlay.elevation[0] + overlay.elevation[2]),
                        frame_rect.top()
                            + frame_rect.height() * (overlay.elevation[1] + overlay.elevation[3]),
                    ),
                );
                let elevation_point = |point: [f32; 2]| {
                    egui::pos2(
                        chart.left() + (point[0] * 0.5 + 0.5) * chart.width(),
                        chart.top() + (0.5 - point[1] * 0.5) * chart.height(),
                    )
                };
                let progress_x = chart.left() + chart.width() * frame.progress.clamp(0.0, 1.0);
                let fill = egui::Color32::from_rgba_unmultiplied(
                    self.model.settings.scene.route_color[0],
                    self.model.settings.scene.route_color[1],
                    self.model.settings.scene.route_color[2],
                    46,
                );
                for pair in frame.elevation_line.windows(2) {
                    let a = elevation_point(pair[0]);
                    let b = elevation_point(pair[1]);
                    if a.x < progress_x {
                        let end_x = b.x.min(progress_x);
                        let ratio = if (b.x - a.x).abs() > f32::EPSILON {
                            ((end_x - a.x) / (b.x - a.x)).clamp(0.0, 1.0)
                        } else {
                            0.0
                        };
                        let end = egui::pos2(end_x, a.y + (b.y - a.y) * ratio);
                        painter.add(egui::Shape::convex_polygon(
                            vec![
                                a,
                                end,
                                egui::pos2(end.x, chart.bottom()),
                                egui::pos2(a.x, chart.bottom()),
                            ],
                            fill,
                            egui::Stroke::NONE,
                        ));
                    }
                }
                let profile: Vec<_> = frame
                    .elevation_line
                    .iter()
                    .copied()
                    .map(elevation_point)
                    .collect();
                painter.add(egui::Shape::line(
                    profile,
                    egui::Stroke::new(
                        2.0,
                        egui::Color32::from_rgba_unmultiplied(
                            self.model.settings.scene.route_color[0],
                            self.model.settings.scene.route_color[1],
                            self.model.settings.scene.route_color[2],
                            178,
                        ),
                    ),
                ));
            }
            painter.text(
                frame_rect.right_bottom() - egui::vec2(12.0, 10.0),
                egui::Align2::RIGHT_BOTTOM,
                match self.model.settings.scene.map_style {
                    MapStyle::Satellite => {
                        "Tiles © Esri — Source: Esri, Maxar, Earthstar Geographics"
                    }
                    _ => "© OpenStreetMap contributors",
                },
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(90),
            );
            if self.model.settings.scene.show_hud && self.language == UiLanguage::TraditionalChinese
            {
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 22.0),
                    egui::Align2::LEFT_TOP,
                    format!("公里數 {:.2} km", frame.distance_m / 1000.0),
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 54.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "海拔 {}",
                        frame
                            .elevation_m
                            .map(|value| format!("{value:.0} m"))
                            .unwrap_or_else(|| "-- m".to_owned())
                    ),
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
            }
            if self.model.settings.scene.show_hud && self.language == UiLanguage::English {
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 22.0),
                    egui::Align2::LEFT_TOP,
                    format!("Distance {:.2} km", frame.distance_m / 1000.0),
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 54.0),
                    egui::Align2::LEFT_TOP,
                    format!(
                        "Elevation {}",
                        frame
                            .elevation_m
                            .map(|value| format!("{value:.0} m"))
                            .unwrap_or_else(|| "-- m".to_owned())
                    ),
                    egui::FontId::proportional(24.0),
                    egui::Color32::WHITE,
                );
            }
        });
    }

    fn diagnostics_window(&mut self, ctx: &egui::Context) {
        if !self.show_diagnostics {
            return;
        }
        if self.language == UiLanguage::English {
            self.diagnostics_window_english(ctx);
            return;
        }
        egui::Window::new("GPU 診斷")
            .open(&mut self.show_diagnostics)
            .show(ctx, |ui| {
                if let Some(gpu) = &self.model.capabilities {
                    ui.label(format!("Adapter: {}", gpu.adapter_name));
                    ui.label(format!("LUID: {}", gpu.luid));
                    ui.label(format!(
                        "專用 VRAM: {:.1} GB",
                        gpu.dedicated_vram as f64 / (1u64 << 30) as f64
                    ));
                    ui.label(format!(
                        "NVENC HEVC: {} · H.264: {} · Async: {}",
                        gpu.hevc, gpu.h264, gpu.async_encode
                    ));
                }
                let d: &Diagnostics = &self.model.diagnostics;
                ui.separator();
                ui.label(format!("CPU frame readbacks: {}", d.cpu_frame_readbacks));
                ui.label(format!(
                    "Encoded: {} · dropped: {} · duplicated: {}",
                    d.encoded_frames, d.dropped_frames, d.duplicated_frames
                ));
                ui.label(format!(
                    "Render p50/p95: {:.2}/{:.2} ms · Encode p95: {:.2} ms · Mux p95: {:.2} ms",
                    d.render_p50_ms, d.render_p95_ms, d.encode_p95_ms, d.mux_p95_ms
                ));
                ui.label(format!(
                    "Ring peak: {} · VRAM peak: {:.1} GB · elapsed: {:.2} s",
                    d.ring_occupancy_peak,
                    d.peak_vram_bytes as f64 / (1u64 << 30) as f64,
                    d.elapsed_seconds
                ));
            });
    }

    fn diagnostics_window_english(&mut self, ctx: &egui::Context) {
        egui::Window::new("GPU Diagnostics")
            .open(&mut self.show_diagnostics)
            .fixed_pos(egui::pos2(24.0, 72.0))
            .show(ctx, |ui| {
                if let Some(gpu) = &self.model.capabilities {
                    ui.label(format!("Adapter: {}", gpu.adapter_name));
                    ui.label(format!("LUID: {}", gpu.luid));
                    ui.label(format!(
                        "Dedicated VRAM: {:.1} GB",
                        gpu.dedicated_vram as f64 / (1u64 << 30) as f64
                    ));
                    ui.label(format!(
                        "NVENC HEVC: {} · H.264: {} · Async: {}",
                        gpu.hevc, gpu.h264, gpu.async_encode
                    ));
                }
                let d = &self.model.diagnostics;
                ui.separator();
                ui.label(format!("CPU frame readbacks: {}", d.cpu_frame_readbacks));
                ui.label(format!(
                    "Encoded: {} · dropped: {} · duplicated: {}",
                    d.encoded_frames, d.dropped_frames, d.duplicated_frames
                ));
                ui.label(format!(
                    "Render p50/p95: {:.2}/{:.2} ms · Encode p95: {:.2} ms · Mux p95: {:.2} ms",
                    d.render_p50_ms, d.render_p95_ms, d.encode_p95_ms, d.mux_p95_ms
                ));
                ui.label(format!(
                    "Ring peak: {} · VRAM peak: {:.1} GB · elapsed: {:.2} s",
                    d.ring_occupancy_peak,
                    d.peak_vram_bytes as f64 / (1u64 << 30) as f64,
                    d.elapsed_seconds
                ));
            });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let title = match self.language {
            UiLanguage::TraditionalChinese => "設定",
            UiLanguage::English => "Settings",
        };
        egui::Window::new(title)
            .open(&mut self.show_settings)
            .fixed_pos(egui::pos2(24.0, 72.0))
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => "語言",
                    UiLanguage::English => "Language",
                });
                egui::ComboBox::from_id_salt("language")
                    .selected_text(match self.language {
                        UiLanguage::TraditionalChinese => "繁體中文",
                        UiLanguage::English => "English",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.language,
                            UiLanguage::TraditionalChinese,
                            "繁體中文",
                        );
                        ui.selectable_value(&mut self.language, UiLanguage::English, "English");
                    });
                ui.separator();
                let mut cache_gb =
                    self.model.settings.cache_limit_bytes as f32 / (1024.0 * 1024.0 * 1024.0);
                ui.add(
                    egui::Slider::new(&mut cache_gb, 0.25..=8.0)
                        .text(match self.language {
                            UiLanguage::TraditionalChinese => "地圖快取上限 GB",
                            UiLanguage::English => "Map cache limit GB",
                        })
                        .step_by(0.25),
                );
                self.model.settings.cache_limit_bytes =
                    (cache_gb * 1024.0 * 1024.0 * 1024.0) as u64;
                ui.small(match self.language {
                    UiLanguage::TraditionalChinese => "地圖只會預載一次；已快取的圖磚會離線重用。",
                    UiLanguage::English => "Tiles are reused locally after the first preload.",
                });
                if ui
                    .button(match self.language {
                        UiLanguage::TraditionalChinese => "清除目前地圖快取",
                        UiLanguage::English => "Clear map cache",
                    })
                    .clicked()
                {
                    let style = self.model.settings.scene.map_style;
                    if let Err(error) = d3d11_renderer::TileDiskCache::for_map_style_with_limit(
                        style,
                        Some(self.model.settings.cache_limit_bytes),
                    )
                    .clear()
                    {
                        self.last_error = Some(error.to_string());
                    } else {
                        self.preview_tiles.clear();
                        self.pending_tiles.clear();
                    }
                }
                ui.separator();
                ui.small(match self.language {
                    UiLanguage::TraditionalChinese => "語言會立即套用到介面。",
                    UiLanguage::English => "Language changes apply immediately.",
                });
            });
    }

    fn persist_preferences(&mut self) {
        let mut current = self.preferences.clone();
        current.language = self.language;
        current.settings = self.model.settings.clone();
        current.cache_limit_bytes = current.settings.cache_limit_bytes;
        if current != self.preferences {
            if let Err(error) = current.save() {
                self.last_error = Some(format!("Failed to save settings: {error}"));
            }
            self.preferences = current;
        }
    }
}

impl eframe::App for NativeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(path) = ctx.input(|input| {
            input
                .raw
                .dropped_files
                .iter()
                .filter_map(|file| file.path.clone())
                .find(|path| {
                    path.extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("gpx"))
                })
        }) {
            self.load_gpx(path);
        }
        self.poll_worker();
        self.poll_tiles(ctx);
        self.draw_header(ctx);
        self.draw_controls(ctx);
        self.draw_preview(ctx);
        self.diagnostics_window(ctx);
        self.settings_window(ctx);
        self.persist_preferences();
        if matches!(self.model.state, JobState::Running(_)) || self.gpu_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}

fn apply_pan(
    options: &mut scene_core::SceneOptions,
    track: &Track,
    delta: egui::Vec2,
    size: egui::Vec2,
) {
    if size.x <= 1.0 || size.y <= 1.0 {
        return;
    }
    let min_lon = track
        .points
        .iter()
        .map(|point| point.lon)
        .fold(f64::INFINITY, f64::min);
    let max_lon = track
        .points
        .iter()
        .map(|point| point.lon)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_lat = track
        .points
        .iter()
        .map(|point| point.lat)
        .fold(f64::INFINITY, f64::min);
    let max_lat = track
        .points
        .iter()
        .map(|point| point.lat)
        .fold(f64::NEG_INFINITY, f64::max);
    let mut center = options
        .free_camera_center
        .unwrap_or([(min_lon + max_lon) * 0.5, (min_lat + max_lat) * 0.5]);
    center[0] -=
        delta.x as f64 / size.x as f64 * (max_lon - min_lon) / options.camera_zoom.max(0.25);
    center[1] +=
        delta.y as f64 / size.y as f64 * (max_lat - min_lat) / options.camera_zoom.max(0.25);
    options.free_camera_center = Some(center);
    options.camera_mode = CameraMode::Free;
}

fn reset_camera_for_new_track(options: &mut scene_core::SceneOptions) {
    options.camera_mode = CameraMode::Follow;
    options.free_camera_center = None;
    options.camera_zoom = 1.0;
}

fn install_chinese_font(ctx: &egui::Context) {
    let path = PathBuf::from(r"C:\Windows\Fonts\msjh.ttc");
    if let Ok(bytes) = std::fs::read(path) {
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "microsoft-jhenghei".into(),
            egui::FontData::from_owned(bytes).into(),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "microsoft-jhenghei".into());
        }
        ctx.set_fonts(fonts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpx_core::parse_gpx;
    #[test]
    fn pan_switches_to_free_camera() {
        let track=parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"/><trkpt lat="26" lon="122"/></trkseg></trk></gpx>"#,ParseOptions::default()).unwrap();
        let mut options = scene_core::SceneOptions::default();
        apply_pan(
            &mut options,
            &track,
            egui::vec2(100.0, 50.0),
            egui::vec2(1000.0, 500.0),
        );
        assert_eq!(options.camera_mode, CameraMode::Free);
        assert!(options.free_camera_center.is_some());
    }
    #[test]
    fn loading_a_track_resets_saved_free_camera() {
        let mut options = scene_core::SceneOptions {
            camera_mode: CameraMode::Free,
            free_camera_center: Some([121.3, 23.0]),
            camera_zoom: 8.0,
            ..scene_core::SceneOptions::default()
        };
        reset_camera_for_new_track(&mut options);
        assert_eq!(options.camera_mode, CameraMode::Follow);
        assert_eq!(options.free_camera_center, None);
        assert_eq!(options.camera_zoom, 1.0);
    }
    #[test]
    fn english_localizes_core_export_errors() {
        assert!(localized_error(UiLanguage::English, "匯出已取消").contains("cancelled"));
        assert!(
            localized_error(UiLanguage::English, "請先載入 GPX 並選擇輸出檔案").contains("Load")
        );
        assert_eq!(
            localized_error(UiLanguage::TraditionalChinese, "錯誤"),
            "錯誤"
        );
    }
}
