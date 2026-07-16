use crate::{
    AppModel, AppPreferences, Diagnostics, ExportOutcome, ExportProgress, ExportRequest,
    ExportSettings, JobState, SettingsWindowPreferences, UiLanguage, UiLayoutPreferences,
    detect_gpu_capabilities, load_gpx_file, run_native_export,
};
use eframe::egui;
use gpx_core::{ParseOptions, Track};
use nvenc_engine::CancellationToken;
use places_core::{
    DataPackManager, GatewayConfig, NearbyProviderPreference, NearbySearchRequest, PlaceLanguage,
    PlaceProvider, PlaceSummary, PoiProfile, PoiSearchResponse, PoiService, ProviderCredentials,
    SearchCoordinate,
};
use scene_core::{
    Aspect, CameraMode, Codec, FrameBuildContext, FramePurpose, LandmarkSource, LandmarkStyle,
    MapStyle, QualityPreset, RenderLanguage, RouteLandmark, Scene, anchor_landmark_to_route,
    build_frame_with_context, screen_point_to_geo,
};
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

enum PlacesMessage {
    Finished {
        request_id: u64,
        result: Result<PoiSearchResponse, String>,
    },
    PackFinished {
        result: Result<String, String>,
    },
}

#[derive(Clone, Copy)]
struct ContextMenuState {
    screen_pos: egui::Pos2,
    coordinate: SearchCoordinate,
}

struct NearbyDialogState {
    coordinate: SearchCoordinate,
    request_id: u64,
    loading: bool,
    places: Vec<PlaceSummary>,
    error: Option<String>,
    attempts: Vec<places_core::FallbackAttempt>,
    attribution: Vec<String>,
    degraded: bool,
}

struct CustomLandmarkState {
    coordinate: SearchCoordinate,
    name: String,
    category: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SettingsPage {
    General,
    Places,
    ApiKeys,
    Storage,
    Advanced,
}

impl SettingsPage {
    fn scroll_id(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Places => "places",
            Self::ApiKeys => "api-keys",
            Self::Storage => "storage",
            Self::Advanced => "advanced",
        }
    }
}

pub struct NativeApp {
    model: AppModel,
    track: Option<Track>,
    gpx_path: Option<PathBuf>,
    output_path: Option<PathBuf>,
    preview_progress: f64,
    /// Temporary map inspection state. These values are never persisted or
    /// exported; the Follow zoom remains owned by SceneOptions.
    preview_inspecting: bool,
    preview_center_mercator: Option<[f64; 2]>,
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
    settings_page: SettingsPage,
    preferences: AppPreferences,
    layout: UiLayoutPreferences,
    context_menu: Option<ContextMenuState>,
    nearby_dialog: Option<NearbyDialogState>,
    candidate_place: Option<PlaceSummary>,
    selected_landmark_id: Option<String>,
    landmarks: Vec<RouteLandmark>,
    project_path: Option<PathBuf>,
    project_warning: Option<String>,
    custom_landmark: Option<CustomLandmarkState>,
    places_tx: Sender<PlacesMessage>,
    places_rx: Receiver<PlacesMessage>,
    next_places_request_id: u64,
    google_key_input: String,
    google_key_status: Option<String>,
    tomtom_key_input: String,
    tomtom_key_status: Option<String>,
    foursquare_key_input: String,
    foursquare_key_status: Option<String>,
    gateway_token_input: String,
    gateway_token_status: Option<String>,
    gateway_url_input: String,
    poi_pack_loading: bool,
    poi_pack_status: Option<String>,
}

const DEFAULT_POI_MANIFEST_URL: &str =
    "https://github.com/Tommyweige/GPX-Animator-GPU/releases/latest/download/poi-manifest.json";
// Public verification key for the signed release data-pack channel.  The
// private signing key never belongs in the repository or the executable.
const DEFAULT_POI_PACK_PUBLIC_KEY_HEX: &str =
    "6c39c86798e836d9f312c5737ed916bfd5ed4b964dee43dd51eaf9d0b01bd207";

fn poi_data_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("GPX Animator")
        .join("poi")
}

fn offline_poi_pack_summary(language: UiLanguage) -> String {
    let Ok(catalog) = places_core::LocalPoiCatalog::from_app_data(poi_data_root()) else {
        return if language == UiLanguage::English {
            "Offline POI data pack status unavailable.".into()
        } else {
            "無法取得離線 POI 資料包狀態。".into()
        };
    };
    let overture = catalog
        .overture
        .as_ref()
        .and_then(|store| store.stats().ok())
        .map(|stats| stats.place_count)
        .unwrap_or_default();
    let osm = catalog
        .osm
        .as_ref()
        .and_then(|store| store.stats().ok())
        .map(|stats| stats.place_count)
        .unwrap_or_default();
    if overture == 0 && osm == 0 {
        if language == UiLanguage::English {
            "Offline POI data pack is empty; download it before using Offline Free.".into()
        } else {
            "離線 POI 資料包是空的，使用 Offline Free 前請先下載。".into()
        }
    } else if language == UiLanguage::English {
        format!("Offline POI data: Overture {overture} places · OSM {osm} places")
    } else {
        format!("離線 POI 資料：Overture {overture} 筆 · OSM {osm} 筆")
    }
}

fn place_name_matches(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        value
            .chars()
            .filter(|character| {
                !character.is_whitespace() && !"，。、,.()（）-".contains(*character)
            })
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let left = normalize(left);
    let right = normalize(right);
    !left.is_empty() && (left == right || left.contains(&right) || right.contains(&left))
}

fn current_long_edge(settings: &ExportSettings) -> u32 {
    match settings.scene.aspect {
        Aspect::Portrait => settings.height,
        Aspect::Landscape | Aspect::Square => settings.width,
    }
}

const SETTINGS_WINDOW_DEFAULT_WIDTH: f32 = 760.0;
const SETTINGS_WINDOW_DEFAULT_HEIGHT: f32 = 620.0;
const SETTINGS_WINDOW_MIN_WIDTH: f32 = 680.0;
const SETTINGS_WINDOW_MIN_HEIGHT: f32 = 500.0;
const SETTINGS_WINDOW_OUTER_MARGIN: f32 = 24.0;

fn settings_window_max_size(available_rect: egui::Rect) -> egui::Vec2 {
    let available_size = available_rect.size();
    egui::vec2(
        (available_size.x - SETTINGS_WINDOW_OUTER_MARGIN * 2.0).max(0.0),
        (available_size.y - SETTINGS_WINDOW_OUTER_MARGIN * 2.0).max(0.0),
    )
}

fn settings_window_min_size(available_rect: egui::Rect) -> egui::Vec2 {
    let max_size = settings_window_max_size(available_rect);
    egui::vec2(
        SETTINGS_WINDOW_MIN_WIDTH.min(max_size.x),
        SETTINGS_WINDOW_MIN_HEIGHT.min(max_size.y),
    )
}

fn settings_window_size(available_rect: egui::Rect) -> egui::Vec2 {
    let min_size = settings_window_min_size(available_rect);
    let max_size = settings_window_max_size(available_rect);
    egui::vec2(
        SETTINGS_WINDOW_DEFAULT_WIDTH.clamp(min_size.x, max_size.x),
        SETTINGS_WINDOW_DEFAULT_HEIGHT.clamp(min_size.y, max_size.y),
    )
}

fn settings_window_initial_rect(
    available_rect: egui::Rect,
    default_size: egui::Vec2,
    min_size: egui::Vec2,
    max_size: egui::Vec2,
    saved: &SettingsWindowPreferences,
) -> egui::Rect {
    let size = saved
        .size
        .and_then(|size| {
            if size[0].is_finite() && size[1].is_finite() && size[0] > 0.0 && size[1] > 0.0 {
                Some(egui::vec2(size[0], size[1]))
            } else {
                None
            }
        })
        .unwrap_or(default_size);
    let size = egui::vec2(
        size.x.clamp(min_size.x, max_size.x),
        size.y.clamp(min_size.y, max_size.y),
    );
    let default_position = available_rect.center() - size / 2.0;
    let position = saved
        .position
        .and_then(|position| {
            if position[0].is_finite() && position[1].is_finite() {
                Some(egui::pos2(position[0], position[1]))
            } else {
                None
            }
        })
        .unwrap_or(default_position);
    let margin_x = SETTINGS_WINDOW_OUTER_MARGIN.min(available_rect.width() * 0.5);
    let margin_y = SETTINGS_WINDOW_OUTER_MARGIN.min(available_rect.height() * 0.5);
    let min_x = available_rect.left() + margin_x;
    let min_y = available_rect.top() + margin_y;
    let max_x = (available_rect.right() - margin_x - size.x).max(min_x);
    let max_y = (available_rect.bottom() - margin_y - size.y).max(min_y);
    egui::Rect::from_min_size(
        egui::pos2(
            position.x.clamp(min_x, max_x),
            position.y.clamp(min_y, max_y),
        ),
        size,
    )
}

fn settings_window_preferences_from_rect(rect: egui::Rect) -> SettingsWindowPreferences {
    let values = [rect.left(), rect.top(), rect.width(), rect.height()];
    if values.iter().all(|value| value.is_finite()) {
        SettingsWindowPreferences {
            position: Some([rect.left(), rect.top()]),
            size: Some([rect.width(), rect.height()]),
        }
    } else {
        SettingsWindowPreferences::default()
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

fn render_language(language: UiLanguage) -> RenderLanguage {
    match language {
        UiLanguage::TraditionalChinese => RenderLanguage::TraditionalChinese,
        UiLanguage::English => RenderLanguage::English,
    }
}

fn poi_profile_label(profile: PoiProfile, language: UiLanguage) -> &'static str {
    match (profile, language) {
        (PoiProfile::OfflineFree, UiLanguage::English) => "Offline Free",
        (PoiProfile::OfflineFree, UiLanguage::TraditionalChinese) => "Offline Free（離線免費）",
        (PoiProfile::TomTomLive, UiLanguage::English) => "TomTom Live",
        (PoiProfile::TomTomLive, UiLanguage::TraditionalChinese) => "TomTom 線上",
        (PoiProfile::FoursquareEnhanced, UiLanguage::English) => "Foursquare Enhanced",
        (PoiProfile::FoursquareEnhanced, UiLanguage::TraditionalChinese) => "Foursquare 強化",
        (PoiProfile::GatewayPro, UiLanguage::English) => "Gateway Pro",
        (PoiProfile::GatewayPro, UiLanguage::TraditionalChinese) => "Gateway Pro",
        (PoiProfile::GoogleByok, UiLanguage::English) => "Google Places (BYOK)",
        (PoiProfile::GoogleByok, UiLanguage::TraditionalChinese) => "Google Places（自備金鑰）",
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
        let mut model = AppModel {
            settings,
            ..AppModel::default()
        };
        let (gpu_tx, gpu_receiver) = channel();
        std::thread::spawn(move || {
            let result = detect_gpu_capabilities().map_err(|error| error.to_string());
            let _ = gpu_tx.send(result);
        });
        let (tile_tx, tile_rx) = channel();
        let (places_tx, places_rx) = channel();
        let google_key_input = crate::secrets::read_google_places_api_key()
            .ok()
            .flatten()
            .unwrap_or_default();
        let tomtom_key_input = crate::secrets::read_tomtom_api_key()
            .ok()
            .flatten()
            .unwrap_or_default();
        let foursquare_key_input = crate::secrets::read_foursquare_api_key()
            .ok()
            .flatten()
            .unwrap_or_default();
        let gateway_token_input = crate::secrets::read_gateway_bearer_token()
            .ok()
            .flatten()
            .unwrap_or_default();
        let gateway_url_input = preferences.gateway_base_url.clone().unwrap_or_default();
        let preview_map_style = model.settings.scene.map_style;
        let language = preferences.language;
        model.settings.scene.render_language = render_language(language);
        Self {
            model,
            track: None,
            gpx_path: None,
            output_path: None,
            preview_progress: 0.0,
            preview_inspecting: false,
            preview_center_mercator: None,
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
            language,
            show_settings: false,
            settings_page: SettingsPage::General,
            layout: preferences.ui_layout.clone(),
            preferences,
            context_menu: None,
            nearby_dialog: None,
            candidate_place: None,
            selected_landmark_id: None,
            landmarks: Vec::new(),
            project_path: None,
            project_warning: None,
            custom_landmark: None,
            places_tx,
            places_rx,
            next_places_request_id: 0,
            google_key_input,
            google_key_status: None,
            tomtom_key_input,
            tomtom_key_status: None,
            foursquare_key_input,
            foursquare_key_status: None,
            gateway_token_input,
            gateway_token_status: None,
            gateway_url_input,
            poi_pack_loading: false,
            poi_pack_status: Some(offline_poi_pack_summary(language)),
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
                self.preview_inspecting = false;
                self.preview_center_mercator = None;
                self.candidate_place = None;
                self.selected_landmark_id = None;
                self.output_path = Some(path.with_extension("mp4"));
                self.gpx_path = Some(path);
                self.track = Some(track);
                if let (Some(gpx_path), Some(track)) = (&self.gpx_path, &self.track) {
                    match crate::project::load_for_route(gpx_path, track) {
                        Ok(project) => {
                            self.landmarks = project.landmarks;
                            self.project_path = Some(project.path);
                            self.project_warning = project.warning;
                        }
                        Err(error) => {
                            self.landmarks.clear();
                            self.project_path = None;
                            self.project_warning =
                                Some(format!("Could not load route project: {error}"));
                        }
                    }
                }
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

    fn save_landmarks(&mut self) {
        let (Some(gpx_path), Some(track)) = (&self.gpx_path, &self.track) else {
            return;
        };
        match crate::project::save_for_route(gpx_path, &self.landmarks, track) {
            Ok(crate::project::ProjectSaveLocation::Sidecar(path)) => {
                self.project_path = Some(path);
                self.project_warning = None;
            }
            Ok(crate::project::ProjectSaveLocation::AppData(path)) => {
                self.project_path = Some(path.clone());
                self.project_warning = Some(format!(
                    "Route project saved in AppData: {}",
                    path.display()
                ));
            }
            Err(error) => {
                self.project_warning = Some(format!("Could not save route project: {error}"));
            }
        }
    }

    fn route_landmark_from_place(&self, place: &PlaceSummary) -> Option<RouteLandmark> {
        let open_place = match place.provider {
            PlaceProvider::Overture | PlaceProvider::OpenStreetMap => place.clone(),
            PlaceProvider::TomTom | PlaceProvider::Foursquare => {
                let catalog = places_core::LocalPoiCatalog::from_app_data(poi_data_root()).ok()?;
                let request = NearbySearchRequest {
                    coordinate: SearchCoordinate {
                        latitude: place.latitude,
                        longitude: place.longitude,
                    },
                    radius_m: 100,
                    limit: 20,
                    language: PlaceLanguage::TraditionalChinese,
                };
                catalog
                    .search(request)
                    .ok()?
                    .into_iter()
                    .filter(|candidate| {
                        matches!(
                            candidate.provider,
                            PlaceProvider::Overture | PlaceProvider::OpenStreetMap
                        ) && place_name_matches(&place.name, &candidate.name)
                    })
                    .min_by(|a, b| a.distance_m.total_cmp(&b.distance_m))?
            }
            PlaceProvider::Google | PlaceProvider::Gateway => return None,
        };
        let source = match open_place.provider {
            PlaceProvider::Overture => LandmarkSource::Overture,
            PlaceProvider::OpenStreetMap => LandmarkSource::OpenStreetMap,
            _ => return None,
        };
        let track = self.track.as_ref()?;
        let anchor = anchor_landmark_to_route(track, open_place.latitude, open_place.longitude)?;
        let source_tag = match open_place.provider {
            PlaceProvider::Overture => "overture",
            PlaceProvider::OpenStreetMap => "osm",
            _ => "provider",
        };
        Some(RouteLandmark {
            id: format!("{source_tag}:{}", open_place.id),
            source,
            source_id: Some(open_place.id.clone()),
            name: open_place.name.clone(),
            category: open_place.category.clone(),
            latitude: open_place.latitude,
            longitude: open_place.longitude,
            anchor_distance_m: anchor.anchor_distance_m,
            anchor_progress: anchor.anchor_progress,
            distance_from_route_m: anchor.distance_from_route_m,
            enabled: true,
            style: LandmarkStyle::default(),
        })
    }

    fn add_landmark(&mut self, mut landmark: RouteLandmark) -> String {
        if let Some(existing) = self.landmarks.iter_mut().find(|value| {
            value.id == landmark.id
                || ((value.latitude - landmark.latitude).abs() < 0.0003
                    && (value.longitude - landmark.longitude).abs() < 0.0003
                    && value.name.eq_ignore_ascii_case(&landmark.name))
        }) {
            existing.enabled = true;
            let id = existing.id.clone();
            self.selected_landmark_id = Some(id.clone());
            return id;
        }
        let id = landmark.id.clone();
        landmark.anchor_progress = landmark.anchor_progress.clamp(0.0, 1.0);
        self.landmarks.push(landmark);
        self.landmarks.sort_by(|a, b| {
            a.anchor_distance_m
                .total_cmp(&b.anchor_distance_m)
                .then_with(|| a.id.cmp(&b.id))
        });
        self.save_landmarks();
        self.selected_landmark_id = Some(id.clone());
        id
    }

    fn add_place_from_dialog(&mut self, index: usize) {
        let Some(dialog) = &self.nearby_dialog else {
            return;
        };
        let Some(place) = dialog.places.get(index).cloned() else {
            return;
        };
        if let Some(landmark) = self.route_landmark_from_place(&place) {
            self.add_landmark(landmark);
            self.candidate_place = None;
            self.preview_inspecting = true;
            self.preview_center_mercator =
                Some(scene_core::geo_to_mercator(place.latitude, place.longitude));
        } else {
            self.last_error = Some(if self.language == UiLanguage::English {
                "This provider result has no matching open-data place; use Add custom route marker from the map."
                    .into()
            } else {
                "找不到對應的開放資料地點，請從地圖右鍵使用「新增自訂沿途地點」。".into()
            });
        }
    }

    fn add_custom_landmark(&mut self) {
        let Some(state) = self.custom_landmark.take() else {
            return;
        };
        let Some(track) = &self.track else {
            return;
        };
        let Some(anchor) =
            anchor_landmark_to_route(track, state.coordinate.latitude, state.coordinate.longitude)
        else {
            return;
        };
        let id = format!(
            "manual:{:.6}:{:.6}:{}",
            state.coordinate.latitude,
            state.coordinate.longitude,
            self.landmarks.len()
        );
        let coordinate = state.coordinate;
        self.add_landmark(RouteLandmark {
            id,
            source: LandmarkSource::Manual,
            source_id: None,
            name: if state.name.trim().is_empty() {
                "Route location".into()
            } else {
                state.name.trim().into()
            },
            category: (!state.category.trim().is_empty()).then(|| state.category.trim().into()),
            latitude: state.coordinate.latitude,
            longitude: state.coordinate.longitude,
            anchor_distance_m: anchor.anchor_distance_m,
            anchor_progress: anchor.anchor_progress,
            distance_from_route_m: anchor.distance_from_route_m,
            enabled: true,
            style: LandmarkStyle::default(),
        });
        self.preview_inspecting = true;
        self.preview_center_mercator = Some(scene_core::geo_to_mercator(
            coordinate.latitude,
            coordinate.longitude,
        ));
    }

    fn draw_landmark_manager(&mut self, ui: &mut egui::Ui) {
        if self.track.is_none() {
            return;
        }
        let english = self.language == UiLanguage::English;
        ui.small(if english {
            "Markers appear when the route reaches them and remain on the final fit view."
        } else {
            "路線抵達地點時標記會浮起，最後完整視角會保留所有圖釘。"
        });
        let mut changed = false;
        let mut remove = None;
        let mut jump = None;
        for index in 0..self.landmarks.len() {
            let landmark = &mut self.landmarks[index];
            let id = landmark.id.clone();
            ui.push_id(id, |ui| {
                ui.group(|ui| {
                    ui.horizontal(|ui| {
                        if ui.checkbox(&mut landmark.enabled, "").changed() {
                            changed = true;
                        }
                        let source = match landmark.source {
                            LandmarkSource::Overture => "Overture",
                            LandmarkSource::OpenStreetMap => "OSM",
                            LandmarkSource::Manual => "Manual",
                            LandmarkSource::TomTom => "TomTom",
                            LandmarkSource::Foursquare => "Foursquare",
                            LandmarkSource::Google => "Google",
                        };
                        if ui
                            .button(if english { "Preview" } else { "預覽" })
                            .clicked()
                        {
                            jump = Some(landmark.anchor_progress);
                        }
                        ui.small(format!(
                            "{:.2} km · {source}",
                            landmark.anchor_distance_m / 1000.0
                        ));
                        if ui.button(if english { "Remove" } else { "移除" }).clicked() {
                            remove = Some(index);
                        }
                    });
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut landmark.name).hint_text(if english {
                                "Place name"
                            } else {
                                "地點名稱"
                            }),
                        )
                        .changed()
                    {
                        changed = true;
                    }
                    ui.horizontal(|ui| {
                        ui.label(if english { "Category" } else { "分類" });
                        if ui
                            .add(
                                egui::TextEdit::singleline(
                                    landmark.category.get_or_insert_with(String::new),
                                )
                                .desired_width(150.0),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                        ui.small(format!(
                            "{} m from route",
                            landmark.distance_from_route_m.round()
                        ));
                    });
                });
            });
        }
        if let Some(index) = remove {
            self.landmarks.remove(index);
            changed = true;
        }
        if let Some(progress) = jump {
            self.preview_progress = progress;
        }
        if changed {
            self.save_landmarks();
        }
        if let Some(warning) = &self.project_warning {
            ui.colored_label(egui::Color32::YELLOW, warning);
        }
        if self.landmarks.is_empty() {
            ui.small(if english {
                "No route places yet. Right-click the map and search nearby places."
            } else {
                "尚未加入地點；在地圖上按右鍵即可搜尋附近地點。"
            });
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
                    let mut tile = tile;
                    d3d11_renderer::apply_map_color_transform(&mut tile, style);
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
                            format!("map-{style:?}-{}-{}-{}", key.zoom, key.x, key.y),
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

    fn start_nearby_lookup(&mut self, coordinate: SearchCoordinate) {
        self.candidate_place = None;
        self.selected_landmark_id = None;
        self.next_places_request_id = self.next_places_request_id.wrapping_add(1);
        let request_id = self.next_places_request_id;
        self.nearby_dialog = Some(NearbyDialogState {
            coordinate,
            request_id,
            loading: true,
            places: Vec::new(),
            error: None,
            attempts: Vec::new(),
            attribution: Vec::new(),
            degraded: false,
        });
        let tx = self.places_tx.clone();
        let radius_m = places_core::normalize_radius(self.preferences.nearby_radius_m);
        let language = match self.language {
            UiLanguage::TraditionalChinese => PlaceLanguage::TraditionalChinese,
            UiLanguage::English => PlaceLanguage::English,
        };
        let profile = self.preferences.poi_profile;
        let online = self.preferences.nearby_online;
        let gateway_url = self.preferences.gateway_base_url.clone();
        let app_data = poi_data_root();
        std::thread::spawn(move || {
            let request = NearbySearchRequest {
                coordinate,
                radius_m,
                limit: 20,
                language,
            };
            let tomtom_key = crate::secrets::read_tomtom_api_key()
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty());
            let google_key = crate::secrets::read_google_places_api_key()
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty());
            let foursquare_key = crate::secrets::read_foursquare_api_key()
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty());
            let gateway_token = crate::secrets::read_gateway_bearer_token()
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty());
            let local = places_core::LocalPoiCatalog::from_app_data(&app_data).ok();
            let service = PoiService::new(local);
            let credentials = ProviderCredentials {
                tomtom_api_key: tomtom_key,
                foursquare_api_key: foursquare_key,
                google_api_key: google_key,
                gateway_bearer_token: gateway_token,
                gateway: gateway_url.map(|base_url| GatewayConfig {
                    base_url,
                    enabled: true,
                }),
            };
            let result = service
                .search(profile, request, &credentials, online, false)
                .map_err(|error| error.to_string());
            let _ = tx.send(PlacesMessage::Finished { request_id, result });
        });
    }

    fn start_poi_pack_download(&mut self) {
        if self.poi_pack_loading {
            return;
        }
        self.poi_pack_loading = true;
        self.poi_pack_status = Some("Downloading and verifying Overture/OSM data pack…".into());
        let tx = self.places_tx.clone();
        let root = poi_data_root();
        let manifest_url = std::env::var("GPX_ANIMATOR_POI_MANIFEST_URL")
            .unwrap_or_else(|_| DEFAULT_POI_MANIFEST_URL.to_owned());
        let public_key = std::env::var("GPX_ANIMATOR_POI_PACK_PUBLIC_KEY_HEX")
            .ok()
            .or_else(|| Some(DEFAULT_POI_PACK_PUBLIC_KEY_HEX.to_owned()));
        std::thread::spawn(move || {
            let result = DataPackManager::new(root)
                .map(|manager| manager.with_signature_policy(public_key, true))
                .and_then(|manager| manager.download_manifest_and_install(&manifest_url))
                .map(|paths| format!("Installed {} local data pack(s).", paths.len()))
                .map_err(|error| error.to_string());
            let _ = tx.send(PlacesMessage::PackFinished { result });
        });
    }

    fn poll_places(&mut self) {
        while let Ok(message) = self.places_rx.try_recv() {
            match message {
                PlacesMessage::Finished { request_id, result } => {
                    let Some(dialog) = self.nearby_dialog.as_mut() else {
                        continue;
                    };
                    if dialog.request_id != request_id {
                        continue;
                    }
                    dialog.loading = false;
                    match result {
                        Ok(places) => {
                            dialog.places = places.places;
                            dialog.attempts = places.attempts;
                            dialog.attribution = places.attribution;
                            dialog.degraded = places.degraded;
                            dialog.error = None;
                        }
                        Err(error) => dialog.error = Some(error),
                    }
                }
                PlacesMessage::PackFinished { result } => {
                    self.poi_pack_loading = false;
                    self.poi_pack_status = Some(match result {
                        Ok(message) => {
                            format!("{message} {}", offline_poi_pack_summary(self.language))
                        }
                        Err(error) => format!("Data pack: {error}"),
                    });
                }
            }
        }
    }

    fn draw_context_menu(&mut self, ctx: &egui::Context) {
        let Some(menu) = self.context_menu else {
            return;
        };
        let mut search = false;
        let mut custom = false;
        let language = self.language;
        egui::Area::new(egui::Id::new("nearby-context-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(menu.screen_pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.label(match language {
                        UiLanguage::TraditionalChinese => "選取位置",
                        UiLanguage::English => "Selected location",
                    });
                    ui.small(format!(
                        "{:.6}, {:.6}",
                        menu.coordinate.latitude, menu.coordinate.longitude
                    ));
                    if ui
                        .button(match language {
                            UiLanguage::TraditionalChinese => "搜尋附近地點",
                            UiLanguage::English => "Search nearby places",
                        })
                        .clicked()
                    {
                        search = true;
                    }
                    if ui
                        .button(match language {
                            UiLanguage::TraditionalChinese => "新增自訂沿途地點",
                            UiLanguage::English => "Add custom route marker",
                        })
                        .clicked()
                    {
                        custom = true;
                    }
                    if ui
                        .button(match language {
                            UiLanguage::TraditionalChinese => "關閉",
                            UiLanguage::English => "Close",
                        })
                        .clicked()
                    {
                        search = false;
                        self.context_menu = None;
                    }
                });
            });
        if search {
            self.context_menu = None;
            self.start_nearby_lookup(menu.coordinate);
        }
        if custom {
            self.context_menu = None;
            self.custom_landmark = Some(CustomLandmarkState {
                coordinate: menu.coordinate,
                name: String::new(),
                category: String::new(),
            });
        }
    }

    #[allow(dead_code, clippy::collapsible_else_if)]
    fn draw_nearby_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.nearby_dialog.as_mut() else {
            return;
        };
        let language = self.language;
        let mut close = false;
        let mut retry = false;
        let mut open_settings = false;
        let mut add_index = None;
        let landmark_ids: HashSet<String> = self
            .landmarks
            .iter()
            .map(|value| value.id.clone())
            .collect();
        egui::Window::new(match language {
            UiLanguage::TraditionalChinese => "附近地點",
            UiLanguage::English => "Nearby places",
        })
        .id(egui::Id::new("nearby-places-window"))
        .fixed_pos(egui::pos2(0.0, 0.0))
        .default_size(egui::vec2(420.0, 520.0))
        .resizable(true)
        .collapsible(false)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(match language {
                    UiLanguage::TraditionalChinese => "座標",
                    UiLanguage::English => "Coordinate",
                });
                ui.monospace(format!("{:.5}, {:.5}", dialog.coordinate.latitude, dialog.coordinate.longitude));
            });
            ui.horizontal(|ui| {
                ui.label(match language {
                    UiLanguage::TraditionalChinese => "搜尋半徑",
                    UiLanguage::English => "Search radius",
                });
                egui::ComboBox::from_id_salt("nearby-radius")
                    .selected_text(format!("{} m", self.preferences.nearby_radius_m))
                    .show_ui(ui, |ui| {
                        for radius in places_core::ALLOWED_RADII_M {
                            if ui
                                .selectable_label(self.preferences.nearby_radius_m == radius, format!("{} m", radius))
                                .clicked()
                            {
                                self.preferences.nearby_radius_m = radius;
                            }
                        }
                    });
                if ui
                    .button(match language {
                        UiLanguage::TraditionalChinese => "重新搜尋",
                        UiLanguage::English => "Retry",
                    })
                    .clicked()
                {
                    retry = true;
                }
                if ui
                    .button(match language {
                        UiLanguage::TraditionalChinese => "關閉",
                        UiLanguage::English => "Close",
                    })
                    .clicked()
                {
                    close = true;
                }
            });
            ui.separator();
            if dialog.loading {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(match language {
                        UiLanguage::TraditionalChinese => "正在查詢 TomTom / Google / OpenStreetMap…",
                        UiLanguage::English => "Querying TomTom / Google / OpenStreetMap…",
                    });
                });
            }
            if let Some(error) = &dialog.error {
                ui.colored_label(egui::Color32::LIGHT_RED, error);
                ui.small(match language {
                    UiLanguage::TraditionalChinese => "請確認網路、防火牆與 Google API 設定；結果只在目前視窗保存。",
                    UiLanguage::English => "Check the network, TomTom/Google key, or firewall. Results remain in memory only.",
                });
            }
            if !dialog.places.is_empty() {
                let provider = dialog.places.first().map(|place| place.provider);
                ui.small(match provider {
                    Some(PlaceProvider::Google) => "Powered by Google · sorted by review count",
                    Some(PlaceProvider::TomTom) => {
                        "Powered by TomTom · sorted by provider relevance and distance"
                    }
                    Some(PlaceProvider::OpenStreetMap) => "OpenStreetMap · sorted by distance (no review data)",
                    Some(PlaceProvider::Overture) => "Overture local snapshot - sorted by distance",
                    Some(PlaceProvider::Foursquare) => "Foursquare - sorted by popularity/rating",
                    Some(PlaceProvider::Gateway) => "Gateway provider - organisation policy",
                    None => "",
                });
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, place) in dialog.places.iter().enumerate() {
                        ui.group(|ui| {
                            ui.horizontal(|ui| {
                                ui.strong(format!("{}.", index + 1));
                                ui.strong(&place.name);
                                let source_tag = match place.provider {
                                    PlaceProvider::Overture => "overture",
                                    PlaceProvider::OpenStreetMap => "osm",
                                    _ => "provider",
                                };
                                let place_id = format!("{source_tag}:{}", place.id);
                                if matches!(
                                    place.provider,
                                    PlaceProvider::Overture
                                        | PlaceProvider::OpenStreetMap
                                        | PlaceProvider::TomTom
                                        | PlaceProvider::Foursquare
                                ) {
                                    if landmark_ids.contains(&place_id) {
                                        ui.label(if language == UiLanguage::English {
                                            "✓ Added"
                                        } else {
                                            "✓ 已加入"
                                        });
                                    } else if ui
                                        .button(if language == UiLanguage::English {
                                            if matches!(place.provider, PlaceProvider::TomTom | PlaceProvider::Foursquare) {
                                                "Match & add"
                                            } else {
                                                "Add to route"
                                            }
                                        } else {
                                            if matches!(place.provider, PlaceProvider::TomTom | PlaceProvider::Foursquare) {
                                                "匹配後加入"
                                            } else {
                                                "加入路線"
                                            }
                                        })
                                        .clicked()
                                    {
                                        add_index = Some(index);
                                    }
                                } else {
                                    ui.small(if language == UiLanguage::English {
                                        "Use open-data match or custom marker"
                                    } else {
                                        "請使用開放資料匹配或自訂標記"
                                    });
                                }
                            });
                            if let Some(category) = &place.category {
                                ui.small(category);
                            }
                            ui.horizontal(|ui| {
                                if let Some(rating) = place.rating {
                                    ui.label(format!("Rating {rating:.1}/{}", place.rating_scale.unwrap_or(5)));
                                    ui.label(format!("★ {rating:.1}"));
                                    if place.review_count > 0 {
                                        ui.label(format!("{} ratings", place.review_count));
                                    }
                                } else if let Some(score) = place.provider_score {
                                    ui.label(format!("TomTom relevance {score:.2}"));
                                } else {
                                    ui.label("No review data");
                                }
                                if let Some(popularity) = place.popularity {
                                    ui.label(format!("Popularity {:.0}%", popularity * 100.0));
                                }
                                ui.label(format!("{:.0} m", place.distance_m));
                                if let Some(open_now) = place.open_now {
                                    ui.label(if open_now { "Open" } else { "Closed" });
                                }
                            });
                            if let Some(address) = &place.address {
                                ui.small(address);
                            }
                            if let Some(phone) = &place.phone {
                                ui.small(phone);
                            }
                            if let Some(website) = &place.website {
                                ui.hyperlink_to("Open website", website);
                            }
                            ui.hyperlink_to(
                                match place.provider {
                                    PlaceProvider::Google => "Open in Google Maps",
                                    PlaceProvider::TomTom => "Open in Google Maps",
                                    PlaceProvider::OpenStreetMap => "Open in OpenStreetMap",
                                    PlaceProvider::Overture => "Open in Google Maps",
                                    PlaceProvider::Foursquare => "Open in Foursquare",
                                    PlaceProvider::Gateway => "Open in Google Maps",
                                },
                                &place.external_url,
                            );
                        });
                    }
                });
            }
            if !dialog.loading && dialog.places.is_empty() && dialog.error.is_none() {
                ui.separator();
                ui.colored_label(
                    egui::Color32::from_rgb(210, 150, 40),
                    match language {
                        UiLanguage::TraditionalChinese => {
                            "Offline Free 沒有可用的本地 POI 資料包。請先下載離線資料包。"
                        }
                        UiLanguage::English => {
                            "Offline Free has no local POI data pack. Download the offline data pack first."
                        }
                    },
                );
                if ui
                    .button(match language {
                        UiLanguage::TraditionalChinese => "開啟設定並下載資料包",
                        UiLanguage::English => "Open Settings and download the data pack",
                    })
                    .clicked()
                {
                    open_settings = true;
                }
            }
            if dialog.places.iter().any(|place| place.provider == PlaceProvider::Google) {
                ui.small("Google Maps and Places attribution is required when Google data is shown.");
            }
            for attribution in &dialog.attribution {
                ui.small(attribution);
            }
            if dialog.degraded {
                ui.small("Some providers were unavailable; results are from the configured fallback.");
            }
            if !dialog.attempts.is_empty() {
                ui.small(format!("Fallback stages checked: {}", dialog.attempts.len()));
            }
            if dialog.places.iter().any(|place| place.provider == PlaceProvider::TomTom) {
                ui.small("TomTom POI data · free daily allowance applies to your own API key.");
            }
        });
        let coordinate = dialog.coordinate;
        if close {
            self.nearby_dialog = None;
        } else if retry {
            self.start_nearby_lookup(coordinate);
        }
        if let Some(index) = add_index {
            self.add_place_from_dialog(index);
        }
        if open_settings {
            self.show_settings = true;
        }
    }

    fn draw_custom_landmark_dialog(&mut self, ctx: &egui::Context) {
        let Some(state) = self.custom_landmark.as_mut() else {
            return;
        };
        let english = self.language == UiLanguage::English;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new(if english {
            "Add custom route marker"
        } else {
            "新增自訂沿途地點"
        })
        .id(egui::Id::new("custom-route-landmark"))
        .fixed_pos(egui::pos2(460.0, 110.0))
        .default_size(egui::vec2(360.0, 190.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.small(format!(
                "{:.6}, {:.6}",
                state.coordinate.latitude, state.coordinate.longitude
            ));
            ui.label(if english { "Name" } else { "名稱" });
            ui.text_edit_singleline(&mut state.name);
            ui.label(if english {
                "Category (optional)"
            } else {
                "分類（可選）"
            });
            ui.text_edit_singleline(&mut state.category);
            ui.horizontal(|ui| {
                if ui.button(if english { "Add" } else { "加入" }).clicked() {
                    save = true;
                }
                if ui.button(if english { "Cancel" } else { "取消" }).clicked() {
                    cancel = true;
                }
            });
        });
        if save {
            self.add_custom_landmark();
        } else if cancel {
            self.custom_landmark = None;
        }
    }

    /// Render nearby search results in a real right-hand inspector.  The old
    /// floating window is kept only as a compatibility shim for saved UI
    /// state; this panel is the sole active entry point.
    fn draw_nearby_panel(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.nearby_dialog.as_mut() else {
            return;
        };
        let language = self.language;
        let english = language == UiLanguage::English;
        let landmark_ids: HashSet<String> = self
            .landmarks
            .iter()
            .map(|value| value.id.clone())
            .collect();
        let mut close = false;
        let mut retry = false;
        let mut open_settings = false;
        let mut add_index = None;
        let mut preview_index: Option<usize> = None;
        let available_width = ctx.available_rect().width().max(0.0);
        let min_width = 300.0_f32;
        let (preferred_width, max_width) =
            nearby_panel_widths(available_width, self.layout.nearby_panel_width);
        let panel_response = egui::SidePanel::right("nearby-places-inspector")
            .resizable(true)
            .default_width(preferred_width)
            .width_range(min_width..=max_width)
            .show(ctx, |ui| {
                ui.set_max_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.heading(if english {
                        "Nearby places"
                    } else {
                        "附近地點"
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("×").clicked() {
                            close = true;
                        }
                    });
                });
                ui.small(if english {
                    "Select a place to preview a pin, then add it to the route."
                } else {
                    "選取地點可預覽圖針；按加入後才會寫入路線。"
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(if english { "Coordinate" } else { "座標" });
                    ui.monospace(format!(
                        "{:.5}, {:.5}",
                        dialog.coordinate.latitude, dialog.coordinate.longitude
                    ));
                });
                ui.horizontal(|ui| {
                    ui.label(if english { "Radius" } else { "半徑" });
                    egui::ComboBox::from_id_salt("nearby-radius-docked")
                        .selected_text(format!("{} m", self.preferences.nearby_radius_m))
                        .show_ui(ui, |ui| {
                            for radius in places_core::ALLOWED_RADII_M {
                                ui.selectable_value(
                                    &mut self.preferences.nearby_radius_m,
                                    radius,
                                    format!("{} m", radius),
                                );
                            }
                        });
                    if ui
                        .button(if english { "Retry" } else { "重新搜尋" })
                        .clicked()
                    {
                        retry = true;
                    }
                });
                ui.separator();
                if dialog.loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if english {
                            "Searching providers…"
                        } else {
                            "正在搜尋地點…"
                        });
                    });
                }
                if let Some(error) = &dialog.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, error);
                }
                if !dialog.places.is_empty() {
                    ui.small(match dialog.places.first().map(|place| place.provider) {
                        Some(PlaceProvider::Google) => "Google · sorted by review count",
                        Some(PlaceProvider::TomTom) => "TomTom · sorted by relevance/distance",
                        Some(PlaceProvider::Foursquare) => "Foursquare · sorted by popularity",
                        Some(PlaceProvider::OpenStreetMap) => "OpenStreetMap · sorted by distance",
                        Some(PlaceProvider::Overture) => "Offline Overture · sorted by distance",
                        Some(PlaceProvider::Gateway) => "Gateway provider",
                        None => "",
                    });
                }
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, place) in dialog.places.iter().enumerate() {
                            let source_tag = match place.provider {
                                PlaceProvider::Overture => "overture",
                                PlaceProvider::OpenStreetMap => "osm",
                                _ => "provider",
                            };
                            let place_id = format!("{source_tag}:{}", place.id);
                            let added = landmark_ids.contains(&place_id);
                            ui.group(|ui| {
                                ui.set_width(ui.available_width());
                                ui.add(
                                    egui::Label::new(format!("{}  {}", index + 1, place.name))
                                        .truncate(),
                                );
                                ui.horizontal_wrapped(|ui| {
                                    if ui
                                        .button(if english { "Preview" } else { "預覽" })
                                        .clicked()
                                    {
                                        preview_index = Some(index);
                                    }
                                    if added {
                                        ui.label(if english { "Added" } else { "已加入" });
                                    } else if ui
                                        .button(if english { "Add pin" } else { "加入圖針" })
                                        .clicked()
                                    {
                                        add_index = Some(index);
                                    }
                                });
                                if let Some(category) = &place.category {
                                    ui.small(category.replace('_', " "));
                                }
                                ui.horizontal_wrapped(|ui| {
                                    if let Some(rating) = place.rating {
                                        ui.label(format!("★ {rating:.1}"));
                                    }
                                    if place.review_count > 0 {
                                        ui.label(format!("{} reviews", place.review_count));
                                    }
                                    if let Some(popularity) = place.popularity {
                                        ui.label(format!("Popularity {:.0}%", popularity * 100.0));
                                    }
                                    ui.label(format!("{:.0} m", place.distance_m));
                                });
                                if let Some(address) = &place.address {
                                    ui.small(address);
                                }
                                ui.hyperlink_to(
                                    if english { "Open map" } else { "開啟地圖" },
                                    &place.external_url,
                                );
                            });
                            ui.add_space(4.0);
                        }
                    });
                if !dialog.loading && dialog.places.is_empty() && dialog.error.is_none() {
                    ui.colored_label(
                        egui::Color32::from_rgb(210, 150, 40),
                        if english {
                            "No local POI data. Download the offline pack from Settings."
                        } else {
                            "找不到離線 POI；請先到設定下載資料包。"
                        },
                    );
                    if ui
                        .button(if english {
                            "Open Settings"
                        } else {
                            "開啟設定"
                        })
                        .clicked()
                    {
                        open_settings = true;
                    }
                }
                if dialog.degraded {
                    ui.small(if english {
                        "Some providers were unavailable; fallback results are shown."
                    } else {
                        "部分資料來源無法使用，目前顯示 fallback 結果。"
                    });
                }
                for attribution in &dialog.attribution {
                    ui.small(attribution);
                }
            });
        let measured_width = panel_response.response.rect.width();
        if measured_width.is_finite() && measured_width >= min_width {
            self.layout.nearby_panel_width = measured_width.clamp(min_width, max_width);
        }
        let coordinate = dialog.coordinate;
        let candidate = preview_index.and_then(|index| dialog.places.get(index).cloned());
        if close {
            self.nearby_dialog = None;
        } else if retry {
            self.start_nearby_lookup(coordinate);
        }
        if let Some(index) = add_index {
            self.add_place_from_dialog(index);
        }
        if add_index.is_none()
            && let Some(place) = candidate
        {
            self.candidate_place = Some(place);
            self.preview_inspecting = true;
            if let Some(candidate) = &self.candidate_place {
                self.preview_center_mercator = Some(scene_core::geo_to_mercator(
                    candidate.latitude,
                    candidate.longitude,
                ));
            }
        }
        if open_settings {
            self.show_settings = true;
        }
    }

    fn start_export(&mut self) {
        let (Some(track), Some(output)) = (self.track.clone(), self.output_path.clone()) else {
            self.last_error = Some("請先載入 GPX 並選擇輸出檔案".into());
            return;
        };
        self.model.settings = self.validated_settings();
        self.model.settings.scene.render_language = render_language(self.language);
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
            landmarks: self.landmarks.clone(),
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
        value.scene.render_language = render_language(self.language);
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
            });
        });
    }

    fn draw_controls(&mut self, ctx: &egui::Context) {
        // Free camera remains readable for old preference files, but it is no
        // longer a valid export mode. Migrate it immediately to Follow.
        if self.model.settings.scene.camera_mode == CameraMode::Free {
            self.model.settings.scene.camera_mode = CameraMode::Follow;
            self.model.settings.scene.free_camera_center = None;
            self.model.settings.scene.camera_zoom = 1.0;
        }
        self.draw_controls_workflow(ctx);
    }

    #[allow(dead_code)]
    fn draw_controls_legacy_chinese(&mut self, ctx: &egui::Context) {
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
                        });
                    if self.model.settings.scene.camera_mode == CameraMode::Follow {
                        ui.add(
                            egui::Slider::new(
                                &mut self.model.settings.scene.follow_zoom_level,
                                10.0..=20.0,
                            )
                            .step_by(0.25)
                            .text("跟隨地圖縮放"),
                        );
                        ui.small(format!(
                            "Zoom {:.2} · 滾輪保持跟隨模式",
                            self.model.settings.scene.follow_zoom_level
                        ));
                    }
                    ui.add(
                        egui::Slider::new(&mut self.model.settings.scene.line_width_px, 1.0..=16.0)
                            .text("路線寬度 px"),
                    );
                    ui.checkbox(&mut self.model.settings.scene.show_hud, "顯示 HUD");
                    ui.checkbox(&mut self.model.settings.scene.show_elevation, "顯示海拔圖");
                    self.draw_landmark_manager(ui);
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

    /// Compact workflow-oriented controls shared by both languages.  Keeping
    /// one layout prevents the English and Traditional Chinese UIs from
    /// drifting apart as new export options are added.
    fn draw_controls_workflow(&mut self, ctx: &egui::Context) {
        let english = self.language == UiLanguage::English;
        let min_width = 292.0_f32;
        let max_width = 420.0_f32;
        let preferred_width = self.layout.left_panel_width.clamp(min_width, max_width);
        let panel_response = egui::SidePanel::left("controls")
            .resizable(true)
            .default_width(preferred_width)
            .width_range(min_width..=max_width)
            .show(ctx, |ui| {
                ui.set_max_width(ui.available_width());
                egui::TopBottomPanel::bottom("controls-export-footer")
                    .resizable(false)
                    .show_inside(ui, |ui| self.draw_export_footer(ui, english));
                egui::ScrollArea::vertical()
                    .id_salt("controls-workflow-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let Some(track) = self.track.clone() else {
                            ui.heading(if english { "Track" } else { "軌跡" });
                            ui.label(if english {
                                "Load a GPX track to start."
                            } else {
                                "請先載入 GPX 軌跡。"
                            });
                            if ui
                                .button(if english {
                                    "Choose GPX file"
                                } else {
                                    "選擇 GPX 檔案"
                                })
                                .clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter("GPX", &["gpx"])
                                    .pick_file()
                            {
                                self.load_gpx(path);
                            }
                            return;
                        };

                        ui.horizontal(|ui| {
                            ui.heading(if english { "Track" } else { "軌跡" });
                            if ui
                                .small_button(if english { "Change" } else { "更換" })
                                .clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter("GPX", &["gpx"])
                                    .pick_file()
                            {
                                self.load_gpx(path);
                            }
                        });
                        ui.group(|ui| {
                            ui.set_width(ui.available_width());
                            ui.add(egui::Label::new(track.name.clone()).truncate());
                            ui.horizontal_wrapped(|ui| {
                                ui.label(format!("{:.2} km", track.distance_m / 1000.0));
                                ui.label(format!(
                                    "{} {:.0} m",
                                    if english { "Gain" } else { "爬升" },
                                    track.elevation_gain_m
                                ));
                                ui.label(format!(
                                    "{} {}",
                                    if english { "GPS" } else { "GPS 點" },
                                    track.source_point_count
                                ));
                            });
                            ui.small(format!(
                                "{} {}",
                                if english {
                                    "Filtered stops:"
                                } else {
                                    "已移除停留點："
                                },
                                track.removed_stop_points
                            ));
                        });

                        let preview_state =
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                egui::Id::new("controls-preview-section"),
                                self.layout.preview_section_open,
                            );
                        let preview_header = preview_state.show_header(ui, |ui| {
                            ui.strong(if english { "Preview" } else { "預覽" });
                        });
                        let preview_open = preview_header.is_open();
                        preview_header.body(|ui| self.draw_preview_settings(ui, english));
                        self.layout.preview_section_open = preview_open;

                        let places_state =
                            egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                egui::Id::new("controls-landmarks-section"),
                                self.layout.landmarks_section_open,
                            );
                        let count = self.landmarks.len();
                        let places_header = places_state.show_header(ui, |ui| {
                            ui.strong(if english {
                                format!("Route places · {count}")
                            } else {
                                format!("沿途地點 · {count}")
                            });
                        });
                        let places_open = places_header.is_open();
                        places_header.body(|ui| self.draw_landmark_manager(ui));
                        self.layout.landmarks_section_open = places_open;
                    });
            });
        let measured_width = panel_response.response.rect.width();
        if measured_width.is_finite() {
            self.layout.left_panel_width = measured_width.clamp(min_width, max_width);
        }
    }

    fn draw_preview_settings(&mut self, ui: &mut egui::Ui, english: bool) {
        let mut long_edge = current_long_edge(&self.model.settings);
        egui::Grid::new("preview-settings-grid")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(if english { "Aspect" } else { "比例" });
                egui::ComboBox::from_id_salt("preview-aspect")
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
                ui.end_row();

                ui.label(if english { "Resolution" } else { "解析度" });
                egui::ComboBox::from_id_salt("preview-resolution")
                    .selected_text(resolution_label(long_edge, self.language))
                    .show_ui(ui, |ui| {
                        for edge in [3840, 2560, 1920, 1280] {
                            if ui
                                .selectable_label(
                                    long_edge == edge,
                                    resolution_label(edge, self.language),
                                )
                                .clicked()
                            {
                                long_edge = edge;
                            }
                        }
                    });
                ui.end_row();

                ui.label(if english { "Map" } else { "地圖" });
                egui::ComboBox::from_id_salt("preview-map-style")
                    .selected_text(match self.model.settings.scene.map_style {
                        MapStyle::Light => {
                            if english {
                                "Light"
                            } else {
                                "淺色"
                            }
                        }
                        MapStyle::Dark => {
                            if english {
                                "Dark"
                            } else {
                                "深色"
                            }
                        }
                        MapStyle::Satellite => {
                            if english {
                                "Satellite"
                            } else {
                                "衛星圖"
                            }
                        }
                        MapStyle::Transparent => {
                            if english {
                                "Transparent · 35%"
                            } else {
                                "淡化地圖 · 35%"
                            }
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.model.settings.scene.map_style,
                            MapStyle::Light,
                            if english { "Light" } else { "淺色" },
                        );
                        ui.selectable_value(
                            &mut self.model.settings.scene.map_style,
                            MapStyle::Dark,
                            if english { "Dark" } else { "深色" },
                        );
                        ui.selectable_value(
                            &mut self.model.settings.scene.map_style,
                            MapStyle::Satellite,
                            if english { "Satellite" } else { "衛星圖" },
                        );
                        ui.selectable_value(
                            &mut self.model.settings.scene.map_style,
                            MapStyle::Transparent,
                            if english {
                                "Transparent · 35%"
                            } else {
                                "淡化地圖 · 35%"
                            },
                        );
                    });
                ui.end_row();

                ui.label(if english { "Camera" } else { "攝影機" });
                egui::ComboBox::from_id_salt("preview-camera")
                    .selected_text(match self.model.settings.scene.camera_mode {
                        CameraMode::Fit => {
                            if english {
                                "Fit route"
                            } else {
                                "完整路線"
                            }
                        }
                        CameraMode::Follow | CameraMode::Free => {
                            if english {
                                "Follow route"
                            } else {
                                "跟隨路線"
                            }
                        }
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.model.settings.scene.camera_mode,
                            CameraMode::Fit,
                            if english { "Fit route" } else { "完整路線" },
                        );
                        ui.selectable_value(
                            &mut self.model.settings.scene.camera_mode,
                            CameraMode::Follow,
                            if english {
                                "Follow route"
                            } else {
                                "跟隨路線"
                            },
                        );
                    });
                ui.end_row();

                if self.model.settings.scene.camera_mode == CameraMode::Follow {
                    ui.label(if english {
                        "Follow zoom"
                    } else {
                        "跟隨縮放"
                    });
                    ui.add(
                        egui::Slider::new(
                            &mut self.model.settings.scene.follow_zoom_level,
                            10.0..=20.0,
                        )
                        .step_by(0.25),
                    );
                    ui.end_row();
                }
                ui.label(if english {
                    "Route width"
                } else {
                    "路線寬度"
                });
                ui.add(
                    egui::Slider::new(&mut self.model.settings.scene.line_width_px, 1.0..=16.0)
                        .suffix(" px"),
                );
                ui.end_row();
                ui.label(if english { "Overlays" } else { "資訊圖層" });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.model.settings.scene.show_hud, "HUD");
                    ui.checkbox(
                        &mut self.model.settings.scene.show_elevation,
                        if english { "Elevation" } else { "海拔" },
                    );
                });
                ui.end_row();
            });
        apply_long_edge(&mut self.model.settings, long_edge);
    }

    fn draw_export_footer(&mut self, ui: &mut egui::Ui, english: bool) {
        ui.separator();
        ui.strong(if english { "Export" } else { "輸出" });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("export-codec")
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
                    ui.selectable_value(&mut self.model.settings.codec, Codec::H264, "H.264 / AVC");
                });
            egui::ComboBox::from_id_salt("export-quality")
                .selected_text(match self.model.settings.quality {
                    QualityPreset::Balanced => {
                        if english {
                            "Balanced"
                        } else {
                            "平衡"
                        }
                    }
                    QualityPreset::Quality => {
                        if english {
                            "High"
                        } else {
                            "高畫質"
                        }
                    }
                    QualityPreset::Speed => {
                        if english {
                            "Speed"
                        } else {
                            "速度"
                        }
                    }
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut self.model.settings.quality,
                        QualityPreset::Balanced,
                        if english { "Balanced" } else { "平衡" },
                    );
                    ui.selectable_value(
                        &mut self.model.settings.quality,
                        QualityPreset::Quality,
                        if english { "High" } else { "高畫質" },
                    );
                    ui.selectable_value(
                        &mut self.model.settings.quality,
                        QualityPreset::Speed,
                        if english { "Speed" } else { "速度" },
                    );
                });
        });

        let advanced_state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            egui::Id::new("controls-export-advanced"),
            self.layout.export_advanced_open,
        );
        let header = advanced_state.show_header(ui, |ui| {
            ui.small(if english {
                "Advanced output"
            } else {
                "進階輸出"
            });
        });
        let advanced_open = header.is_open();
        header.body(|ui| {
            ui.horizontal(|ui| {
                ui.label(if english { "Seconds" } else { "秒數" });
                ui.add(
                    egui::DragValue::new(&mut self.model.settings.duration_seconds).range(1..=3600),
                );
                ui.label("FPS");
                egui::ComboBox::from_id_salt("export-fps")
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
            });
        });
        self.layout.export_advanced_open = advanced_open;

        if ui
            .button(if english {
                "Choose output MP4"
            } else {
                "選擇輸出 MP4"
            })
            .clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("MP4", &["mp4"])
                .set_file_name("gpx-animation.mp4")
                .save_file()
        {
            self.preferences.last_output_directory = path.parent().map(PathBuf::from);
            self.output_path = Some(path);
        }
        if let Some(path) = &self.output_path {
            ui.add(egui::Label::new(path.display().to_string()).truncate());
        }
        match &self.model.state {
            JobState::Running(value) => {
                let ratio = if value.stage_total == 0 {
                    0.0
                } else {
                    value.stage_completed as f32 / value.stage_total as f32
                };
                ui.add(
                    egui::ProgressBar::new(ratio)
                        .show_percentage()
                        .text(format!("{:?} · {:.1} FPS", value.stage, value.fps)),
                );
                if ui.button(if english { "Cancel" } else { "取消" }).clicked() {
                    if let Some(token) = &self.active_token {
                        token.cancel();
                    }
                    self.model.cancel();
                }
            }
            _ => {
                let long_edge = current_long_edge(&self.model.settings);
                if ui
                    .add_enabled(
                        self.track.is_some() && self.model.can_export(),
                        egui::Button::new(if english {
                            format!("Export {} FPS MP4", self.model.settings.fps)
                        } else {
                            format!("輸出 {} FPS MP4", self.model.settings.fps)
                        }),
                    )
                    .clicked()
                {
                    self.start_export();
                }
                ui.small(format!(
                    "{} · {}",
                    resolution_label(long_edge, self.language),
                    if english { "Ready" } else { "準備完成" }
                ));
            }
        }
        if let Some(error) = &self.last_error {
            ui.colored_label(
                egui::Color32::LIGHT_RED,
                localized_error(self.language, error),
            );
        }
    }

    #[allow(dead_code, unreachable_code)]
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
                            CameraMode::Free => "Follow route",
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
                        });
                    if self.model.settings.scene.camera_mode == CameraMode::Follow {
                        ui.add(
                            egui::Slider::new(
                                &mut self.model.settings.scene.follow_zoom_level,
                                10.0..=20.0,
                            )
                            .step_by(0.25)
                            .text("Follow map zoom"),
                        );
                        ui.small(format!(
                            "Zoom {:.2} · wheel keeps Follow mode",
                            self.model.settings.scene.follow_zoom_level
                        ));
                    }
                    ui.add(
                        egui::Slider::new(&mut self.model.settings.scene.line_width_px, 1.0..=16.0)
                            .text("Route width (px)"),
                    );
                    ui.checkbox(&mut self.model.settings.scene.show_hud, "Show HUD");
                    ui.checkbox(
                        &mut self.model.settings.scene.show_elevation,
                        "Show elevation profile",
                    );
                    self.draw_landmark_manager(ui);
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

    #[allow(clippy::collapsible_if)]
    fn draw_preview(&mut self, ctx: &egui::Context) {
        if self.preview_map_style != self.model.settings.scene.map_style {
            self.preview_map_style = self.model.settings.scene.map_style;
            self.preview_tiles.clear();
            self.pending_tiles.clear();
        }
        let mut context_click = None;
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
            if self.preview_inspecting
                && ui
                    .button(if self.language == UiLanguage::English {
                        "Return to export view"
                    } else {
                        "返回輸出視角"
                    })
                    .clicked()
            {
                self.preview_inspecting = false;
                self.preview_center_mercator = None;
            }
            let available = ui.available_size();
            let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
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
                MapStyle::Transparent => egui::Color32::from_rgb(16, 22, 28),
            };
            painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(28, 34, 40));
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
            let preview_options = self.model.settings.scene.clone();
            // Camera composition uses scene_core's logical canvas.  The
            // widget size only controls how the already-computed frame is
            // letterboxed on screen.
            let initial_scene = Scene {
                track: track.clone(),
                options: preview_options.clone(),
                landmarks: self.landmarks.clone(),
                route_duration_seconds: self.model.settings.duration_seconds as f64,
            };
            let initial_frame = build_frame_with_context(
                &initial_scene,
                self.preview_progress,
                FrameBuildContext {
                    purpose: FramePurpose::EditorPreview,
                    center_override_mercator: self.preview_center_mercator,
                },
            );
            if response.dragged() {
                let delta = ctx.input(|input| input.pointer.delta());
                if !self.preview_inspecting {
                    self.preview_inspecting = true;
                    self.preview_center_mercator = Some(initial_frame.view_center_mercator);
                }
                if let Some(center) = &mut self.preview_center_mercator {
                    *center = pan_camera_center(
                        *center,
                        delta,
                        frame_rect.size(),
                        [initial_frame.view_span, initial_frame.view_span_y],
                    );
                }
            }
            if response.hovered() {
                let scroll = ctx.input(|input| input.smooth_scroll_delta.y);
                if scroll.abs() > 0.0 {
                    if self.model.settings.scene.camera_mode == CameraMode::Follow {
                        self.model.settings.scene.follow_zoom_level = apply_follow_zoom_scroll(
                            self.model.settings.scene.follow_zoom_level,
                            scroll,
                        );
                    }
                }
            }
            let scene = Scene {
                track: track.clone(),
                options: preview_options,
                landmarks: self.landmarks.clone(),
                route_duration_seconds: self.model.settings.duration_seconds as f64,
            };
            let frame = build_frame_with_context(
                &scene,
                self.preview_progress,
                FrameBuildContext {
                    purpose: FramePurpose::EditorPreview,
                    center_override_mercator: self.preview_center_mercator,
                },
            );
            let preview_scale = preview_content_scale(
                frame_rect.width(),
                frame_rect.height(),
                self.model.settings.width,
                self.model.settings.height,
            );
            if response.secondary_clicked()
                && let Some(pointer) = response.interact_pointer_pos()
                && frame_rect.contains(pointer)
                && let Some([latitude, longitude]) = screen_point_to_geo(
                    &frame,
                    [frame_rect.width(), frame_rect.height()],
                    [pointer.x - frame_rect.left(), pointer.y - frame_rect.top()],
                )
            {
                context_click = Some(ContextMenuState {
                    screen_pos: pointer,
                    coordinate: SearchCoordinate {
                        latitude,
                        longitude,
                    },
                });
            }
            let zoom = d3d11_renderer::tile_zoom_rect(
                frame.view_span,
                frame.view_span_y,
                frame_rect.width() as u32,
            );
            let keys = d3d11_renderer::required_view_tiles_rect(
                frame.view_center_mercator,
                frame.view_span,
                frame.view_span_y,
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
                                - (-(y - frame.view_center_mercator[1]) * 2.0 / frame.view_span_y)
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
                        if self.model.settings.scene.map_style == MapStyle::Transparent {
                            egui::Color32::from_white_alpha(89)
                        } else {
                            egui::Color32::WHITE
                        },
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
                    self.model.settings.scene.line_width_px * preview_scale,
                    egui::Color32::from_gray(135),
                ),
            ));
            painter.add(egui::Shape::line(
                completed,
                egui::Stroke::new(
                    self.model.settings.scene.line_width_px * preview_scale,
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
                12.0 * preview_scale,
                egui::Color32::from_rgb(255, 93, 59),
            );
            let landmark_scale = preview_scale;
            for landmark in &frame.landmarks {
                if landmark.pin_opacity <= 0.0
                    || landmark.ndc[0] < -1.25
                    || landmark.ndc[0] > 1.25
                    || landmark.ndc[1] < -1.25
                    || landmark.ndc[1] > 1.25
                {
                    continue;
                }
                let point = to_screen(landmark.ndc);
                let alpha = (landmark.pin_opacity.clamp(0.0, 1.0) * 255.0) as u8;
                let color = egui::Color32::from_rgba_unmultiplied(
                    landmark.color[0],
                    landmark.color[1],
                    landmark.color[2],
                    alpha,
                );
                let selected = self.selected_landmark_id.as_deref() == Some(landmark.id.as_str());
                let selection_scale = if selected { 1.35 } else { 1.0 };
                let radius = 18.0 * landmark_scale * landmark.pin_scale.max(0.2) * selection_scale;
                let body = point + egui::vec2(0.0, -radius * 0.78);
                painter.circle_filled(
                    body + egui::vec2(6.0 * landmark_scale, 9.0 * landmark_scale),
                    radius * 1.08,
                    egui::Color32::from_rgba_unmultiplied(0, 0, 0, (alpha as f32 * 0.45) as u8),
                );
                painter.line_segment(
                    [body + egui::vec2(0.0, radius * 0.65), point],
                    egui::Stroke::new(
                        radius * 1.15,
                        egui::Color32::from_rgba_unmultiplied(255, 246, 218, alpha),
                    ),
                );
                painter.circle_filled(
                    body,
                    radius,
                    egui::Color32::from_rgba_unmultiplied(255, 246, 218, alpha),
                );
                painter.line_segment(
                    [body + egui::vec2(0.0, radius * 0.65), point],
                    egui::Stroke::new(radius * 0.82, color),
                );
                painter.circle_filled(body, radius * 0.66, color);
                if landmark.pulse_progress > 0.0 {
                    painter.circle_stroke(
                        body,
                        radius * (1.25 + landmark.pulse_progress * 0.7),
                        egui::Stroke::new(
                            2.0 * landmark_scale,
                            egui::Color32::from_rgba_unmultiplied(
                                landmark.color[0],
                                landmark.color[1],
                                landmark.color[2],
                                (alpha as f32 * (1.0 - landmark.pulse_progress) * 0.7) as u8,
                            ),
                        ),
                    );
                }
                if selected && landmark.label_opacity > 0.0 && landmark.show_label {
                    let label_alpha = (landmark.label_opacity.clamp(0.0, 1.0) * 255.0) as u8;
                    let label_width = 260.0 * landmark_scale;
                    let label_height = if landmark.category.is_some() {
                        54.0 * landmark_scale
                    } else {
                        34.0 * landmark_scale
                    };
                    let side = if landmark.ndc[0] > 0.55 { -1.0 } else { 1.0 };
                    let left = if side > 0.0 {
                        point.x + 28.0 * landmark_scale
                    } else {
                        point.x - 28.0 * landmark_scale - label_width
                    };
                    let top = body.y - label_height - 18.0 * landmark_scale;
                    let rect = egui::Rect::from_min_size(
                        egui::pos2(left, top),
                        egui::vec2(label_width, label_height),
                    );
                    painter.rect_filled(
                        rect,
                        8.0 * landmark_scale,
                        egui::Color32::from_rgba_unmultiplied(
                            9,
                            14,
                            18,
                            (label_alpha as f32 * 0.86) as u8,
                        ),
                    );
                    let mut label = landmark.name.clone();
                    if let Some(category) = &landmark.category {
                        if !category.trim().is_empty() {
                            label.push('\n');
                            label.push_str(category);
                        }
                    }
                    painter.text(
                        rect.left_top() + egui::vec2(12.0 * landmark_scale, 7.0 * landmark_scale),
                        egui::Align2::LEFT_TOP,
                        label,
                        egui::FontId::proportional(28.0 * landmark_scale),
                        egui::Color32::from_rgba_unmultiplied(255, 248, 230, label_alpha),
                    );
                }
            }
            if let Some(candidate) = &self.candidate_place {
                let projected =
                    scene_core::geo_to_mercator(candidate.latitude, candidate.longitude);
                let ndc = [
                    ((projected[0] - frame.view_center_mercator[0]) * 2.0 / frame.view_span) as f32,
                    (-(projected[1] - frame.view_center_mercator[1]) * 2.0 / frame.view_span_y)
                        as f32,
                ];
                if ndc[0].abs() <= 1.15 && ndc[1].abs() <= 1.15 {
                    let point = to_screen(ndc);
                    let radius = (16.0 * landmark_scale).max(8.0);
                    let blue = egui::Color32::from_rgb(92, 190, 255);
                    painter.line_segment(
                        [point + egui::vec2(0.0, -radius * 0.25), point],
                        egui::Stroke::new(radius * 0.65, blue),
                    );
                    painter.circle_stroke(
                        point + egui::vec2(0.0, -radius * 0.75),
                        radius,
                        egui::Stroke::new(3.0 * landmark_scale, blue),
                    );
                    painter.circle_filled(
                        point + egui::vec2(0.0, -radius * 0.75),
                        radius * 0.30,
                        egui::Color32::from_rgba_unmultiplied(92, 190, 255, 170),
                    );
                }
            }
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
                        2.0 * preview_scale,
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
                frame_rect.right_bottom() - egui::vec2(12.0, 10.0) * preview_scale,
                egui::Align2::RIGHT_BOTTOM,
                match self.model.settings.scene.map_style {
                    MapStyle::Satellite => {
                        "Tiles © Esri — Source: Esri, Maxar, Earthstar Geographics"
                    }
                    _ => "© OpenStreetMap contributors",
                },
                egui::FontId::proportional(12.0 * preview_scale),
                egui::Color32::from_gray(90),
            );
            if self.model.settings.scene.show_hud && self.language == UiLanguage::TraditionalChinese
            {
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 22.0) * preview_scale,
                    egui::Align2::LEFT_TOP,
                    format!("公里數 {:.2} km", frame.distance_m / 1000.0),
                    egui::FontId::proportional(36.0 * preview_scale),
                    egui::Color32::WHITE,
                );
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 54.0) * preview_scale,
                    egui::Align2::LEFT_TOP,
                    format!(
                        "海拔 {}",
                        frame
                            .elevation_m
                            .map(|value| format!("{value:.0} m"))
                            .unwrap_or_else(|| "-- m".to_owned())
                    ),
                    egui::FontId::proportional(36.0 * preview_scale),
                    egui::Color32::WHITE,
                );
            }
            if self.model.settings.scene.show_hud && self.language == UiLanguage::English {
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 22.0) * preview_scale,
                    egui::Align2::LEFT_TOP,
                    format!("Distance {:.2} km", frame.distance_m / 1000.0),
                    egui::FontId::proportional(36.0 * preview_scale),
                    egui::Color32::WHITE,
                );
                painter.text(
                    frame_rect.min + egui::vec2(24.0, 54.0) * preview_scale,
                    egui::Align2::LEFT_TOP,
                    format!(
                        "Elevation {}",
                        frame
                            .elevation_m
                            .map(|value| format!("{value:.0} m"))
                            .unwrap_or_else(|| "-- m".to_owned())
                    ),
                    egui::FontId::proportional(36.0 * preview_scale),
                    egui::Color32::WHITE,
                );
            }
        });
        if let Some(value) = context_click {
            self.context_menu = Some(value);
        }
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
        let english = self.language == UiLanguage::English;
        let mut open = true;
        let mut download_pack = false;
        let mut clear_cache = false;
        let language_before = self.language;
        let available_rect = ctx.content_rect();
        let window_min_size = settings_window_min_size(available_rect);
        let window_max_size = settings_window_max_size(available_rect);
        let window_size = settings_window_size(available_rect);
        let saved_window = self.layout.settings_window.clone();
        let window_rect = settings_window_initial_rect(
            available_rect,
            window_size,
            window_min_size,
            window_max_size,
            &saved_window,
        );
        let settings_window_id = egui::Id::new("settings-window-stable");
        egui::Window::new(if english { "Settings" } else { "設定" })
            .id(settings_window_id)
            .open(&mut open)
            .default_rect(window_rect)
            .min_size(window_min_size)
            .max_size(window_max_size)
            .movable(true)
            .resizable(true)
            .constrain_to(available_rect)
            .collapsible(false)
            .show(ctx, |ui| {
                let content_size = ui.available_size_before_wrap();
                ui.horizontal_top(|ui| {
                    let sidebar_width = 150.0_f32.min(content_size.x);
                    ui.allocate_ui_with_layout(
                        egui::vec2(sidebar_width, content_size.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                        ui.heading(if english { "Settings" } else { "設定" });
                        ui.add_space(8.0);
                        for (page, label) in [
                            (
                                SettingsPage::General,
                                if english { "General" } else { "一般" },
                            ),
                            (
                                SettingsPage::Places,
                                if english { "Places" } else { "附近地點" },
                            ),
                            (
                                SettingsPage::ApiKeys,
                                if english { "API keys" } else { "API 金鑰" },
                            ),
                            (
                                SettingsPage::Storage,
                                if english { "Storage" } else { "儲存空間" },
                            ),
                            (
                                SettingsPage::Advanced,
                                if english { "Advanced" } else { "進階" },
                            ),
                        ] {
                            if ui
                                .selectable_label(self.settings_page == page, label)
                                .clicked()
                            {
                                self.settings_page = page;
                            }
                        }
                        },
                    );
                    ui.separator();
                    let content_width = ui.available_width().max(0.0);
                    ui.allocate_ui_with_layout(
                        egui::vec2(content_width, content_size.y),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(("settings-content-scroll", self.settings_page.scroll_id()))
                            .auto_shrink([false, false])
                            .max_height(content_size.y)
                            .show(ui, |ui| match self.settings_page {
                                SettingsPage::General => {
                                    ui.heading(if english { "General" } else { "一般" });
                                    ui.separator();
                                    ui.label(if english { "Language" } else { "語言" });
                                    egui::ComboBox::from_id_salt("settings-language")
                                        .selected_text(if english { "English" } else { "繁體中文" })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(
                                                &mut self.language,
                                                UiLanguage::TraditionalChinese,
                                                "繁體中文",
                                            );
                                            ui.selectable_value(
                                                &mut self.language,
                                                UiLanguage::English,
                                                "English",
                                            );
                                        });
                                    ui.small(if english {
                                        "The selected language is also used for static text in exported videos."
                                    } else {
                                        "目前語言也會套用到輸出影片的固定文字。"
                                    });
                                    ui.separator();
                                    ui.label(if english { "Current output" } else { "目前輸出" });
                                    ui.label(format!(
                                        "{} · {} FPS · H.265",
                                        resolution_label(current_long_edge(&self.model.settings), self.language),
                                        self.model.settings.fps
                                    ));
                                    if ui
                                        .button(if english { "Reset panel layout" } else { "重設面板版面" })
                                        .clicked()
                                    {
                                        self.layout = UiLayoutPreferences::default();
                                    }
                                }
                                SettingsPage::Places => {
                                    ui.heading(if english { "Nearby places" } else { "附近地點" });
                                    ui.separator();
                                    ui.label(if english { "POI profile" } else { "POI 模式" });
                                    egui::ComboBox::from_id_salt("settings-poi-profile")
                                        .selected_text(poi_profile_label(self.preferences.poi_profile, self.language))
                                        .show_ui(ui, |ui| {
                                            for profile in [
                                                PoiProfile::OfflineFree,
                                                PoiProfile::TomTomLive,
                                                PoiProfile::FoursquareEnhanced,
                                                PoiProfile::GatewayPro,
                                                PoiProfile::GoogleByok,
                                            ] {
                                                ui.selectable_value(
                                                    &mut self.preferences.poi_profile,
                                                    profile,
                                                    poi_profile_label(profile, self.language),
                                                );
                                            }
                                        });
                                    ui.checkbox(
                                        &mut self.preferences.nearby_online,
                                        if english { "Allow online refresh" } else { "允許線上更新" },
                                    );
                                    ui.separator();
                                    ui.label(if english { "Offline data pack" } else { "離線資料包" });
                                    ui.small(offline_poi_pack_summary(self.language));
                                    if ui
                                        .add_enabled(
                                            !self.poi_pack_loading,
                                            egui::Button::new(if self.poi_pack_loading {
                                                "Downloading…"
                                            } else if english {
                                                "Download / update data pack"
                                            } else {
                                                "下載／更新資料包"
                                            }),
                                        )
                                        .clicked()
                                    {
                                        download_pack = true;
                                    }
                                    if let Some(status) = &self.poi_pack_status {
                                        ui.small(status);
                                    }
                                }
                                SettingsPage::ApiKeys => {
                                    ui.heading(if english { "Provider API keys" } else { "供應商 API 金鑰" });
                                    ui.small(if english {
                                        "Keys are stored in Windows Credential Manager and are never written to settings.json."
                                    } else {
                                        "金鑰會儲存在 Windows Credential Manager，不會寫入 settings.json。"
                                    });
                                    ui.separator();
                                    ui.group(|ui| {
                                        ui.label(if english { "TomTom Search API key" } else { "TomTom Search API 金鑰" });
                                        ui.add(egui::TextEdit::singleline(&mut self.tomtom_key_input).password(true).hint_text("TomTom key"));
                                        ui.horizontal(|ui| {
                                            if ui.button(if english { "Save" } else { "儲存" }).clicked() {
                                                self.tomtom_key_status = Some(match crate::secrets::write_tomtom_api_key(&self.tomtom_key_input) {
                                                    Ok(()) => if english { "TomTom key saved.".into() } else { "TomTom 金鑰已儲存。".into() },
                                                    Err(error) => error,
                                                });
                                            }
                                            if ui.button(if english { "Remove" } else { "移除" }).clicked() {
                                                self.tomtom_key_input.clear();
                                                self.tomtom_key_status = crate::secrets::write_tomtom_api_key("").err().or_else(|| Some(if english { "TomTom key removed.".into() } else { "TomTom 金鑰已移除。".into() }));
                                            }
                                        });
                                        if let Some(status) = &self.tomtom_key_status { ui.small(status); }
                                    });
                                    ui.add_space(8.0);
                                    ui.group(|ui| {
                                        ui.label(if english { "Google Places API key" } else { "Google Places API 金鑰" });
                                        ui.add(egui::TextEdit::singleline(&mut self.google_key_input).password(true).hint_text("AIza…"));
                                        ui.horizontal(|ui| {
                                            if ui.button(if english { "Save" } else { "儲存" }).clicked() {
                                                self.google_key_status = Some(match crate::secrets::write_google_places_api_key(&self.google_key_input) {
                                                    Ok(()) => if english { "Google key saved.".into() } else { "Google 金鑰已儲存。".into() },
                                                    Err(error) => error,
                                                });
                                            }
                                            if ui.button(if english { "Remove" } else { "移除" }).clicked() {
                                                self.google_key_input.clear();
                                                self.google_key_status = crate::secrets::write_google_places_api_key("").err().or_else(|| Some(if english { "Google key removed.".into() } else { "Google 金鑰已移除。".into() }));
                                            }
                                        });
                                        if let Some(status) = &self.google_key_status { ui.small(status); }
                                    });
                                    ui.add_space(8.0);
                                    ui.group(|ui| {
                                        ui.label(if english { "Foursquare Places API key" } else { "Foursquare Places API 金鑰" });
                                        ui.add(egui::TextEdit::singleline(&mut self.foursquare_key_input).password(true).hint_text("Foursquare key"));
                                        ui.horizontal(|ui| {
                                            if ui.button(if english { "Save" } else { "儲存" }).clicked() {
                                                self.foursquare_key_status = Some(match crate::secrets::write_foursquare_api_key(&self.foursquare_key_input) {
                                                    Ok(()) => "Foursquare key saved.".into(),
                                                    Err(error) => error,
                                                });
                                            }
                                            if ui.button(if english { "Remove" } else { "移除" }).clicked() {
                                                self.foursquare_key_input.clear();
                                                self.foursquare_key_status = crate::secrets::write_foursquare_api_key("").err().or_else(|| Some("Foursquare key removed.".into()));
                                            }
                                        });
                                        if let Some(status) = &self.foursquare_key_status { ui.small(status); }
                                    });
                                    ui.add_space(8.0);
                                    ui.group(|ui| {
                                        ui.label(if english { "Gateway (optional)" } else { "Gateway（選配）" });
                                        ui.add(egui::TextEdit::singleline(&mut self.gateway_url_input).hint_text("https://gateway.example"));
                                        ui.add(egui::TextEdit::singleline(&mut self.gateway_token_input).password(true).hint_text("Bearer token"));
                                        if ui.button(if english { "Save gateway token" } else { "儲存 Gateway token" }).clicked() {
                                            self.gateway_token_status = Some(match crate::secrets::write_gateway_bearer_token(&self.gateway_token_input) {
                                                Ok(()) => "Gateway token saved.".into(),
                                                Err(error) => error,
                                            });
                                        }
                                        if let Some(status) = &self.gateway_token_status { ui.small(status); }
                                    });
                                }
                                SettingsPage::Storage => {
                                    ui.heading(if english { "Storage and cache" } else { "儲存空間與快取" });
                                    ui.separator();
                                    let mut cache_gb = self.model.settings.cache_limit_bytes as f32 / (1024.0 * 1024.0 * 1024.0);
                                    ui.add(egui::Slider::new(&mut cache_gb, 0.25..=8.0).text(if english { "Map cache (GB)" } else { "地圖快取（GB）" }).step_by(0.25));
                                    self.model.settings.cache_limit_bytes = (cache_gb * 1024.0 * 1024.0 * 1024.0) as u64;
                                    ui.small(if english { "Light, Dark and Transparent share the OSM cache. Satellite uses a separate cache." } else { "淺色、深色與淡化地圖共用 OSM 快取；衛星圖使用獨立快取。" });
                                    if ui.button(if english { "Clear current map cache" } else { "清除目前地圖快取" }).clicked() { clear_cache = true; }
                                    if let Some(status) = &self.poi_pack_status { ui.separator(); ui.label(if english { "POI pack" } else { "POI 資料包" }); ui.small(status); }
                                }
                                SettingsPage::Advanced => {
                                    ui.heading(if english { "Advanced diagnostics" } else { "進階診斷" });
                                    ui.separator();
                                    if let Some(gpu) = &self.model.capabilities {
                                        ui.label(format!("Adapter: {}", gpu.adapter_name));
                                        ui.label(format!("NVENC HEVC: {} · H.264: {} · Async: {}", gpu.hevc, gpu.h264, gpu.async_encode));
                                    } else if self.gpu_receiver.is_some() {
                                        ui.spinner();
                                        ui.label(if english { "Detecting GPU capabilities…" } else { "正在偵測 GPU 能力…" });
                                    } else {
                                        ui.colored_label(egui::Color32::LIGHT_RED, if english { "GPU capability detection failed." } else { "GPU 能力偵測失敗。" });
                                    }
                                    let d = &self.model.diagnostics;
                                    ui.separator();
                                    ui.label(format!("CPU readbacks: {} · encoded: {} · dropped: {}", d.cpu_frame_readbacks, d.encoded_frames, d.dropped_frames));
                                    ui.label(format!("Render p50/p95: {:.2}/{:.2} ms · Encode p95: {:.2} ms", d.render_p50_ms, d.render_p95_ms, d.encode_p95_ms));
                                    if ui.button(if english { "Open detailed diagnostics" } else { "開啟詳細診斷" }).clicked() { self.show_diagnostics = true; }
                                }
                            });
                        },
                    );
                });
            });
        if let Some(rect) = ctx.memory(|memory| memory.area_rect(settings_window_id)) {
            let actual_geometry = settings_window_preferences_from_rect(rect);
            let safe_rect = settings_window_initial_rect(
                available_rect,
                window_size,
                window_min_size,
                window_max_size,
                &actual_geometry,
            );
            let geometry = settings_window_preferences_from_rect(safe_rect);
            if self.layout.settings_window != geometry {
                self.layout.settings_window = geometry;
            }
        }
        self.show_settings = open;
        if self.language != language_before {
            self.model.settings.scene.render_language = render_language(self.language);
            ctx.request_repaint();
        }
        if clear_cache {
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
        if download_pack {
            self.start_poi_pack_download();
        }
    }

    #[allow(dead_code)]
    fn settings_window_legacy(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let title = match self.language {
            UiLanguage::TraditionalChinese => "設定",
            UiLanguage::English => "Settings",
        };
        let mut download_pack = false;
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
                egui::CollapsingHeader::new(match self.language {
                    UiLanguage::TraditionalChinese => "進階 · GPU 與輸出診斷",
                    UiLanguage::English => "Advanced · GPU and export diagnostics",
                })
                .id_salt("settings-advanced-gpu")
                .default_open(false)
                .show(ui, |ui| {
                    if let Some(gpu) = &self.model.capabilities {
                        ui.label(format!("Adapter: {}", gpu.adapter_name));
                        ui.label(format!(
                            "NVENC HEVC: {} · H.264: {} · Async: {}",
                            gpu.hevc, gpu.h264, gpu.async_encode
                        ));
                    } else if self.gpu_receiver.is_some() {
                        ui.spinner();
                        ui.label(if self.language == UiLanguage::English {
                            "Detecting GPU capabilities…"
                        } else {
                            "正在偵測 GPU 能力…"
                        });
                    } else {
                        ui.colored_label(
                            egui::Color32::LIGHT_RED,
                            if self.language == UiLanguage::English {
                                "GPU capability detection failed."
                            } else {
                                "GPU 能力偵測失敗。"
                            },
                        );
                    }
                    let diagnostics = &self.model.diagnostics;
                    ui.separator();
                    ui.label(format!(
                        "CPU readbacks: {} · encoded: {} · dropped: {}",
                        diagnostics.cpu_frame_readbacks,
                        diagnostics.encoded_frames,
                        diagnostics.dropped_frames
                    ));
                    if ui
                        .button(if self.language == UiLanguage::English {
                            "Open detailed diagnostics"
                        } else {
                            "開啟詳細診斷"
                        })
                        .clicked()
                    {
                        self.show_diagnostics = true;
                    }
                });
                ui.separator();
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => "附近地點資料來源",
                    UiLanguage::English => "Nearby places provider",
                });
                egui::ComboBox::from_id_salt("nearby-provider")
                    .selected_text(match self.preferences.nearby_provider {
                        NearbyProviderPreference::TomTomFirst => match self.language {
                            UiLanguage::TraditionalChinese => "TomTom（免費、較新）",
                            UiLanguage::English => "Legacy TomTom-first (migration only)",
                        },
                        NearbyProviderPreference::GoogleFirst => match self.language {
                            UiLanguage::TraditionalChinese => "Google（評分優先）",
                            UiLanguage::English => "Legacy Google-first (migration only)",
                        },
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.preferences.nearby_provider,
                            NearbyProviderPreference::TomTomFirst,
                            match self.language {
                                UiLanguage::TraditionalChinese => "TomTom（免費、較新）",
                                UiLanguage::English => "Legacy TomTom-first (migration only)",
                            },
                        );
                        ui.selectable_value(
                            &mut self.preferences.nearby_provider,
                            NearbyProviderPreference::GoogleFirst,
                            match self.language {
                                UiLanguage::TraditionalChinese => "Google（評分優先）",
                                UiLanguage::English => "Legacy Google-first (migration only)",
                            },
                        );
                    });
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => "POI 模式",
                    UiLanguage::English => "POI profile",
                });
                egui::ComboBox::from_id_salt("poi-profile")
                    .selected_text(self.preferences.poi_profile.label())
                    .show_ui(ui, |ui| {
                        for profile in [
                            PoiProfile::OfflineFree,
                            PoiProfile::TomTomLive,
                            PoiProfile::FoursquareEnhanced,
                            PoiProfile::GatewayPro,
                            PoiProfile::GoogleByok,
                        ] {
                            ui.selectable_value(
                                &mut self.preferences.poi_profile,
                                profile,
                                profile.label(),
                            );
                        }
                    });
                ui.checkbox(
                    &mut self.preferences.nearby_online,
                    match self.language {
                        UiLanguage::TraditionalChinese => "允許線上更新",
                        UiLanguage::English => "Allow online refresh",
                    },
                );
                if ui
                    .button(if self.poi_pack_loading {
                        "Downloading POI data pack…"
                    } else {
                        "Download / update offline POI data pack"
                    })
                    .clicked()
                {
                    download_pack = true;
                }
                if let Some(status) = &self.poi_pack_status {
                    ui.small(status);
                }
                ui.small(match self.language {
                    UiLanguage::TraditionalChinese => {
                        "TomTom 需要使用者自己的免費 API Key；沒有金鑰時會使用 OSM 備援。"
                    }
                    UiLanguage::English => {
                        "The POI profile above is authoritative. Offline Free uses the installed Overture + OSM pack; live profiles use their configured provider and documented fallbacks."
                    }
                });
                ui.hyperlink_to(
                    match self.language {
                        UiLanguage::TraditionalChinese => "申請免費 TomTom API Key",
                        UiLanguage::English => "Create a free TomTom API key",
                    },
                    "https://developer.tomtom.com/user/register",
                );
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => {
                        "TomTom Search API Key（儲存在 Windows Credential Manager）"
                    }
                    UiLanguage::English => {
                        "TomTom Search API key (stored in Windows Credential Manager)"
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.tomtom_key_input)
                        .password(true)
                        .hint_text("TomTom key"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(match self.language {
                            UiLanguage::TraditionalChinese => "儲存 TomTom 金鑰",
                            UiLanguage::English => "Save TomTom key",
                        })
                        .clicked()
                    {
                        self.tomtom_key_status = match crate::secrets::write_tomtom_api_key(
                            &self.tomtom_key_input,
                        ) {
                            Ok(()) => Some(match self.language {
                                UiLanguage::TraditionalChinese => {
                                    "TomTom 金鑰已儲存。預設查詢會優先使用 TomTom。".to_owned()
                                }
                                UiLanguage::English => {
                                    "TomTom key saved. TomTom is now preferred by default.".to_owned()
                                }
                            }),
                            Err(error) => Some(error),
                        };
                    }
                    if ui
                        .button(match self.language {
                            UiLanguage::TraditionalChinese => "移除",
                            UiLanguage::English => "Remove",
                        })
                        .clicked()
                    {
                        self.tomtom_key_input.clear();
                        self.tomtom_key_status =
                            match crate::secrets::write_tomtom_api_key("") {
                                Ok(()) => Some(match self.language {
                                    UiLanguage::TraditionalChinese => {
                                        "TomTom API 金鑰已移除。".to_owned()
                                    }
                                    UiLanguage::English => "TomTom API key removed.".to_owned(),
                                }),
                                Err(error) => Some(error),
                            };
                    }
                });
                if let Some(status) = &self.tomtom_key_status {
                    ui.small(status);
                }
                ui.separator();
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => {
                        "Google Places API 金鑰（儲存在 Windows Credential Manager）"
                    }
                    UiLanguage::English => {
                        "Google Places API key (stored in Windows Credential Manager)"
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.google_key_input)
                        .password(true)
                        .hint_text("AIza…"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(match self.language {
                            UiLanguage::TraditionalChinese => "儲存金鑰",
                            UiLanguage::English => "Save key",
                        })
                        .clicked()
                    {
                        self.google_key_status = match crate::secrets::write_google_places_api_key(
                            &self.google_key_input,
                        ) {
                            Ok(()) => Some(match self.language {
                                UiLanguage::TraditionalChinese => {
                                    "已儲存；查詢時優先使用 Google，失敗會回退 OSM。".to_owned()
                                }
                                UiLanguage::English => {
                                    "Saved. Google is contacted only when the explicit Google Places (BYOK) profile is selected."
                                        .to_owned()
                                }
                            }),
                            Err(error) => Some(error),
                        };
                    }
                    if ui
                        .button(match self.language {
                            UiLanguage::TraditionalChinese => "移除",
                            UiLanguage::English => "Remove",
                        })
                        .clicked()
                    {
                        self.google_key_input.clear();
                        self.google_key_status =
                            match crate::secrets::write_google_places_api_key("") {
                                Ok(()) => Some(match self.language {
                                    UiLanguage::TraditionalChinese => {
                                        "已移除 Google API 金鑰。".to_owned()
                                    }
                                    UiLanguage::English => "Google API key removed; TomTom remains available.".to_owned(),
                                }),
                                Err(error) => Some(error),
                            };
                    }
                });
                if let Some(status) = &self.google_key_status {
                    ui.small(status);
                }
                ui.separator();
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => "Foursquare Places Premium API Key",
                    UiLanguage::English => "Foursquare Places Premium API key (Credential Manager)",
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.foursquare_key_input)
                        .password(true)
                        .hint_text("Foursquare key"),
                );
                ui.horizontal(|ui| {
                    if ui
                        .button(match self.language {
                            UiLanguage::TraditionalChinese => "儲存 Foursquare key",
                            UiLanguage::English => "Save Foursquare key",
                        })
                        .clicked()
                    {
                        self.foursquare_key_status =
                            match crate::secrets::write_foursquare_api_key(&self.foursquare_key_input) {
                                Ok(()) => Some("Foursquare key saved; Premium metrics are optional.".into()),
                                Err(error) => Some(error),
                            };
                    }
                    if ui.button("Remove").clicked() {
                        self.foursquare_key_input.clear();
                        self.foursquare_key_status =
                            match crate::secrets::write_foursquare_api_key("") {
                                Ok(()) => Some("Foursquare key removed.".into()),
                                Err(error) => Some(error),
                            };
                    }
                });
                if let Some(status) = &self.foursquare_key_status {
                    ui.small(status);
                }
                ui.label(match self.language {
                    UiLanguage::TraditionalChinese => "Gateway Base URL / Bearer Token（選配）",
                    UiLanguage::English => "Gateway Base URL / bearer token (optional)",
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.gateway_url_input)
                        .hint_text("https://gateway.example")
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.gateway_token_input)
                        .password(true)
                        .hint_text("Gateway token"),
                );
                if ui
                    .button(match self.language {
                        UiLanguage::TraditionalChinese => "儲存 Gateway token",
                        UiLanguage::English => "Save Gateway token",
                    })
                    .clicked()
                {
                    self.gateway_token_status =
                        match crate::secrets::write_gateway_bearer_token(&self.gateway_token_input) {
                            Ok(()) => Some("Gateway token saved.".into()),
                            Err(error) => Some(error),
                        };
                }
                if let Some(status) = &self.gateway_token_status {
                    ui.small(status);
                }
                ui.small(match self.language {
                    UiLanguage::TraditionalChinese => {
                        "Google 評分與評論欄位可能產生 API 費用；未選擇 Google-first 時不會使用。"
                    }
                    UiLanguage::English => {
                        "Google rating/review fields may incur API charges; they are used only in the explicit Google Places (BYOK) profile."
                    }
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
        if download_pack {
            self.start_poi_pack_download();
        }
    }

    fn persist_preferences(&mut self) {
        let mut current = self.preferences.clone();
        current.language = self.language;
        current.settings = self.model.settings.clone();
        current.settings.scene.render_language = render_language(self.language);
        current.cache_limit_bytes = current.settings.cache_limit_bytes;
        current.ui_layout = self.layout.clone();
        current.gateway_base_url = (!self.gateway_url_input.trim().is_empty())
            .then(|| self.gateway_url_input.trim().to_owned());
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
        self.poll_places();
        self.poll_tiles(ctx);
        self.draw_header(ctx);
        self.draw_controls(ctx);
        self.draw_nearby_panel(ctx);
        self.draw_preview(ctx);
        self.draw_context_menu(ctx);
        self.draw_custom_landmark_dialog(ctx);
        self.diagnostics_window(ctx);
        self.settings_window(ctx);
        self.persist_preferences();
        if matches!(self.model.state, JobState::Running(_)) || self.gpu_receiver.is_some() {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }
}

fn apply_follow_zoom_scroll(current: f64, scroll_y: f32) -> f64 {
    (current + f64::from(scroll_y) / 60.0).clamp(10.0, 20.0)
}

fn pan_camera_center(
    center: [f64; 2],
    pointer_delta: egui::Vec2,
    viewport: egui::Vec2,
    span: [f64; 2],
) -> [f64; 2] {
    if viewport.x <= 1.0 || viewport.y <= 1.0 {
        return center;
    }
    [
        center[0] - pointer_delta.x as f64 / viewport.x as f64 * span[0],
        center[1] - pointer_delta.y as f64 / viewport.y as f64 * span[1],
    ]
}

fn reset_camera_for_new_track(options: &mut scene_core::SceneOptions) {
    options.camera_mode = CameraMode::Follow;
    options.free_camera_center = None;
    options.follow_zoom_level = scene_core::default_follow_zoom_level();
    options.camera_zoom = 1.0;
}

fn nearby_panel_widths(available_width: f32, preferred_width: f32) -> (f32, f32) {
    let min_width = 300.0_f32;
    let max_width = (available_width - 640.0).max(min_width).min(720.0);
    (preferred_width.clamp(min_width, max_width), max_width)
}

fn preview_content_scale(
    display_width: f32,
    display_height: f32,
    output_width: u32,
    output_height: u32,
) -> f32 {
    if display_width <= 0.0 || display_height <= 0.0 || output_width == 0 || output_height == 0 {
        return 1.0;
    }
    (display_width / output_width as f32)
        .min(display_height / output_height as f32)
        .max(0.01)
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
    #[test]
    fn pan_follows_pointer_and_preserves_export_zoom() {
        let center = pan_camera_center(
            [0.5, 0.5],
            egui::vec2(100.0, 50.0),
            egui::vec2(1000.0, 500.0),
            [0.1, 0.05],
        );
        assert!(center[0] < 0.5);
        assert!(center[1] < 0.5);
        assert_eq!(apply_follow_zoom_scroll(15.0, 60.0), 16.0);
        assert_eq!(apply_follow_zoom_scroll(10.0, -600.0), 10.0);
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

    #[test]
    fn nearby_panel_width_is_clamped_without_content_growth() {
        assert_eq!(nearby_panel_widths(1800.0, 900.0), (720.0, 720.0));
        assert_eq!(nearby_panel_widths(1200.0, 410.0), (410.0, 560.0));
        assert_eq!(nearby_panel_widths(800.0, 600.0), (300.0, 300.0));
    }

    #[test]
    fn settings_window_uses_standard_size_when_screen_has_room() {
        let available = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1440.0, 900.0));
        assert_eq!(settings_window_size(available), egui::vec2(760.0, 620.0));
    }

    #[test]
    fn settings_window_stays_inside_small_or_high_dpi_available_rect() {
        let small = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(956.0, 764.0));
        let small_size = settings_window_size(small);
        assert!(small_size.x <= small.width() - SETTINGS_WINDOW_OUTER_MARGIN * 2.0);
        assert!(small_size.y <= small.height() - SETTINGS_WINDOW_OUTER_MARGIN * 2.0);

        let high_dpi = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 600.0));
        let high_dpi_size = settings_window_size(high_dpi);
        assert_eq!(high_dpi_size, egui::vec2(760.0, 552.0));
        assert!(high_dpi_size.x <= high_dpi.width() - SETTINGS_WINDOW_OUTER_MARGIN * 2.0);
        assert!(high_dpi_size.y <= high_dpi.height() - SETTINGS_WINDOW_OUTER_MARGIN * 2.0);
    }

    #[test]
    fn settings_window_defaults_are_centered_and_bounded() {
        let available =
            egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(1440.0, 900.0));
        let min_size = settings_window_min_size(available);
        let max_size = settings_window_max_size(available);
        let default_size = settings_window_size(available);
        let rect = settings_window_initial_rect(
            available,
            default_size,
            min_size,
            max_size,
            &SettingsWindowPreferences::default(),
        );
        assert_eq!(rect.size(), egui::vec2(760.0, 620.0));
        assert!((rect.center().x - available.center().x).abs() < f32::EPSILON);
        assert!((rect.center().y - available.center().y).abs() < f32::EPSILON);
        assert!(rect.left() >= available.left() + SETTINGS_WINDOW_OUTER_MARGIN);
        assert!(rect.right() <= available.right() - SETTINGS_WINDOW_OUTER_MARGIN);
        assert!(rect.top() >= available.top() + SETTINGS_WINDOW_OUTER_MARGIN);
        assert!(rect.bottom() <= available.bottom() - SETTINGS_WINDOW_OUTER_MARGIN);
    }

    #[test]
    fn settings_window_restores_and_clamps_persisted_geometry() {
        let available = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(956.0, 764.0));
        let min_size = settings_window_min_size(available);
        let max_size = settings_window_max_size(available);
        let saved = SettingsWindowPreferences {
            position: Some([-500.0, -300.0]),
            size: Some([1500.0, 1200.0]),
        };
        let rect = settings_window_initial_rect(
            available,
            settings_window_size(available),
            min_size,
            max_size,
            &saved,
        );
        assert_eq!(rect.size(), max_size);
        assert_eq!(rect.left(), SETTINGS_WINDOW_OUTER_MARGIN);
        assert_eq!(rect.top(), SETTINGS_WINDOW_OUTER_MARGIN);

        let tiny = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 450.0));
        assert_eq!(
            settings_window_min_size(tiny),
            settings_window_max_size(tiny)
        );
        assert_eq!(settings_window_size(tiny), settings_window_max_size(tiny));
    }

    #[test]
    fn preview_scale_matches_selected_output_density() {
        assert!((preview_content_scale(960.0, 540.0, 3840, 2160) - 0.25).abs() < 1e-6);
        assert!((preview_content_scale(960.0, 540.0, 1920, 1080) - 0.5).abs() < 1e-6);
        assert_eq!(preview_content_scale(0.0, 540.0, 3840, 2160), 1.0);
    }
}
