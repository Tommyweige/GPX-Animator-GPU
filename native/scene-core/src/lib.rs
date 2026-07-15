use gpx_core::{EARTH_RADIUS_M, Track, haversine_m, sample_distance};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Aspect {
    Landscape,
    Square,
    Portrait,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MapStyle {
    Light,
    Dark,
    Satellite,
    Transparent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CameraMode {
    Fit,
    Follow,
    Free,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    Hevc,
    H264,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityPreset {
    Balanced,
    Quality,
    Speed,
}

/// The dataset that supplied a route landmark.  Open-data landmarks can be
/// persisted in a project; provider-specific results are resolved to open data
/// or converted to a user-authored `Manual` landmark before export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LandmarkSource {
    Overture,
    OpenStreetMap,
    TomTom,
    Foursquare,
    Google,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandmarkStyle {
    pub pin_color: [u8; 4],
    pub show_label: bool,
}

impl Default for LandmarkStyle {
    fn default() -> Self {
        Self {
            pin_color: [255, 93, 59, 255],
            show_label: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteLandmark {
    pub id: String,
    pub source: LandmarkSource,
    pub source_id: Option<String>,
    pub name: String,
    pub category: Option<String>,
    pub latitude: f64,
    pub longitude: f64,
    pub anchor_distance_m: f64,
    pub anchor_progress: f64,
    pub distance_from_route_m: f64,
    pub enabled: bool,
    pub style: LandmarkStyle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteAnchor {
    pub anchor_distance_m: f64,
    pub anchor_progress: f64,
    pub distance_from_route_m: f64,
    pub nearest_latitude: f64,
    pub nearest_longitude: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandmarkFrame {
    pub id: String,
    pub ndc: [f32; 2],
    pub pin_opacity: f32,
    pub pin_scale: f32,
    pub pulse_progress: f32,
    pub label_opacity: f32,
    pub label_side: i8,
    pub name: String,
    pub category: Option<String>,
    pub color: [u8; 4],
    pub show_label: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneOptions {
    pub aspect: Aspect,
    pub map_style: MapStyle,
    pub camera_mode: CameraMode,
    pub route_color: [u8; 4],
    pub marker_color: [u8; 4],
    pub line_width_px: f32,
    pub show_hud: bool,
    pub show_elevation: bool,
    pub free_camera_center: Option<[f64; 2]>,
    /// Web Mercator zoom level used by the Follow export camera.  This is a
    /// tile zoom, not the old relative fit multiplier.
    #[serde(default = "default_follow_zoom_level")]
    pub follow_zoom_level: f64,
    /// Pixel dimensions used to turn a Web Mercator zoom into a visible world
    /// span.  Export sets these to the output dimensions; the preview sets
    /// them to its aspect-correct frame dimensions.
    #[serde(default = "default_camera_viewport_width")]
    pub camera_viewport_width_px: u32,
    #[serde(default = "default_camera_viewport_height")]
    pub camera_viewport_height_px: u32,
    /// Temporary preview-only camera overrides.  Export code always leaves
    /// these as `None`, so desktop inspection cannot leak into the MP4.
    #[serde(default)]
    pub preview_center_mercator: Option<[f64; 2]>,
    #[serde(default)]
    pub preview_zoom_level: Option<f64>,
    /// Legacy relative zoom retained only for migration of old project files.
    /// New Follow/preview code does not use it.
    pub camera_zoom: f64,
}

pub const fn default_follow_zoom_level() -> f64 {
    15.0
}

pub const fn default_camera_viewport_width() -> u32 {
    3840
}

pub const fn default_camera_viewport_height() -> u32 {
    2160
}

/// The composition canvas is intentionally independent from the physical
/// output resolution and the size of the egui preview widget.  This keeps a
/// Follow camera at the same map scale when a user switches between 1080p and
/// 4K, or when the side panels are resized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicalCanvas {
    pub width: f64,
    pub height: f64,
}

pub const fn logical_canvas(aspect: Aspect) -> LogicalCanvas {
    match aspect {
        Aspect::Landscape => LogicalCanvas {
            width: 1920.0,
            height: 1080.0,
        },
        Aspect::Square => LogicalCanvas {
            width: 1080.0,
            height: 1080.0,
        },
        Aspect::Portrait => LogicalCanvas {
            width: 1080.0,
            height: 1920.0,
        },
    }
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            aspect: Aspect::Landscape,
            map_style: MapStyle::Satellite,
            camera_mode: CameraMode::Follow,
            route_color: [255, 93, 59, 255],
            marker_color: [255, 243, 214, 255],
            line_width_px: 8.0,
            show_hud: true,
            show_elevation: true,
            free_camera_center: None,
            follow_zoom_level: default_follow_zoom_level(),
            camera_viewport_width_px: default_camera_viewport_width(),
            camera_viewport_height_px: default_camera_viewport_height(),
            preview_center_mercator: None,
            preview_zoom_level: None,
            camera_zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub track: Track,
    pub options: SceneOptions,
    pub landmarks: Vec<RouteLandmark>,
    pub route_duration_seconds: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    pub view_center_mercator: [f64; 2],
    /// Horizontal and vertical world spans.  Keeping them separate prevents
    /// the route from stretching when the output is not square.
    pub view_span: f64,
    pub view_span_y: f64,
    pub route_ndc: Vec<[f32; 2]>,
    pub completed_points: usize,
    pub marker_ndc: [f32; 2],
    pub elevation_line: Vec<[f32; 2]>,
    pub progress: f32,
    pub distance_m: f64,
    pub elevation_m: Option<f64>,
    pub landmarks: Vec<LandmarkFrame>,
}

fn wrapped_longitude_delta(from: f64, to: f64) -> f64 {
    (to - from + 180.0).rem_euclid(360.0) - 180.0
}

fn local_meters(
    latitude: f64,
    longitude: f64,
    origin_latitude: f64,
    origin_longitude: f64,
) -> [f64; 2] {
    let latitude_scale = origin_latitude.to_radians().cos().abs().max(0.1);
    [
        wrapped_longitude_delta(origin_longitude, longitude).to_radians()
            * EARTH_RADIUS_M
            * latitude_scale,
        (latitude - origin_latitude).to_radians() * EARTH_RADIUS_M,
    ]
}

/// Find the closest point on the GPX polyline to a real-world POI.  The POI is
/// deliberately not moved: the returned anchor only controls when its marker
/// is revealed during the animation.
pub fn anchor_landmark_to_route(
    track: &Track,
    latitude: f64,
    longitude: f64,
) -> Option<RouteAnchor> {
    if track.points.len() < 2 || !latitude.is_finite() || !longitude.is_finite() {
        return None;
    }
    let mut best: Option<(f64, usize, f64, [f64; 2])> = None;
    for (index, pair) in track.points.windows(2).enumerate() {
        let a = pair[0];
        let b = pair[1];
        let origin = [latitude, longitude];
        let p0 = local_meters(a.lat, a.lon, origin[0], origin[1]);
        let p1 = local_meters(b.lat, b.lon, origin[0], origin[1]);
        let dx = p1[0] - p0[0];
        let dy = p1[1] - p0[1];
        let length_squared = dx * dx + dy * dy;
        let t = if length_squared > f64::EPSILON {
            (-(p0[0] * dx + p0[1] * dy) / length_squared).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let closest = [p0[0] + dx * t, p0[1] + dy * t];
        let distance_squared = closest[0] * closest[0] + closest[1] * closest[1];
        if best.is_none_or(|value| distance_squared < value.0) {
            let delta_lon = wrapped_longitude_delta(a.lon, b.lon);
            let nearest_latitude = a.lat + (b.lat - a.lat) * t;
            let nearest_longitude = a.lon + delta_lon * t;
            best = Some((
                distance_squared,
                index,
                t,
                [nearest_latitude, nearest_longitude],
            ));
        }
    }
    let (distance_squared, index, t, nearest) = best?;
    let segment_distance = haversine_m(&track.points[index], &track.points[index + 1]);
    let anchor_distance_m = (track.points[index].distance_m + segment_distance * t)
        .clamp(0.0, track.distance_m.max(0.0));
    Some(RouteAnchor {
        anchor_distance_m,
        anchor_progress: if track.distance_m > f64::EPSILON {
            (anchor_distance_m / track.distance_m).clamp(0.0, 1.0)
        } else {
            0.0
        },
        distance_from_route_m: distance_squared.max(0.0).sqrt(),
        nearest_latitude: nearest[0],
        nearest_longitude: nearest[1],
    })
}

fn ease_out_back(value: f64) -> f32 {
    let t = value.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    (1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)) as f32
}

fn friendly_category(value: &str) -> String {
    let normalized = value.trim().replace('_', " ").to_ascii_lowercase();
    let label = match normalized.as_str() {
        "travel services" => "Travel services",
        "sporting goods" => "Sporting goods",
        "restaurant" => "Restaurant",
        "cafe" | "coffee shop" => "Cafe",
        "accommodation" | "hotel" => "Accommodation",
        "park" => "Park",
        "religious organization" => "Religious place",
        "fuel" | "gas station" => "Fuel station",
        "supermarket" | "grocery" => "Grocery",
        "museum" => "Museum",
        "viewpoint" => "Viewpoint",
        _ => value.trim(),
    };
    label.to_owned()
}

fn landmark_frame(
    landmark: &RouteLandmark,
    ndc: [f32; 2],
    progress: f64,
    duration: f64,
) -> LandmarkFrame {
    let duration = duration.max(1.0);
    let elapsed = progress.clamp(0.0, 1.0) * duration;
    let activation = landmark.anchor_progress.clamp(0.0, 1.0) * duration;
    let reveal = ((elapsed - activation) / 0.65).clamp(0.0, 1.0);
    let label_in = ((elapsed - activation - 0.10) / 0.35).clamp(0.0, 1.0);
    let label_out = ((elapsed - activation - 2.2) / 0.30).clamp(0.0, 1.0);
    let label_opacity = if reveal <= 0.0 {
        0.0
    } else {
        (label_in * (1.0 - label_out)).clamp(0.0, 1.0)
    } as f32;
    LandmarkFrame {
        id: landmark.id.clone(),
        ndc,
        pin_opacity: reveal as f32,
        pin_scale: 0.2 + ease_out_back(reveal) * 0.8,
        pulse_progress: if reveal < 1.0 { reveal as f32 } else { 0.0 },
        label_opacity: if landmark.style.show_label {
            label_opacity
        } else {
            0.0
        },
        label_side: 1,
        name: landmark.name.clone(),
        category: landmark.category.as_deref().map(friendly_category),
        color: landmark.style.pin_color,
        show_label: landmark.style.show_label,
    }
}

fn mercator(lon: f64, lat: f64) -> [f64; 2] {
    let x = (lon + 180.0) / 360.0;
    let sin = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians().sin();
    let y = 0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * std::f64::consts::PI);
    [x, y]
}

pub fn geo_to_mercator(latitude: f64, longitude: f64) -> [f64; 2] {
    mercator(longitude, latitude)
}

/// Convert normalized Web Mercator coordinates back to WGS84.  This is
/// public so the desktop preview can keep temporary pan state without
/// changing the persisted export camera.
pub fn mercator_to_geo(point: [f64; 2]) -> [f64; 2] {
    let longitude = point[0] * 360.0 - 180.0;
    let latitude = ((std::f64::consts::PI * (1.0 - 2.0 * point[1])).sinh())
        .atan()
        .to_degrees()
        .clamp(-85.051_128_78, 85.051_128_78);
    [latitude, longitude]
}

/// Convert a pixel in the preview frame back to a WGS84 coordinate.  The
/// point is relative to the actual aspect-correct frame (not the letterboxed
/// central panel), with `(0, 0)` at the top-left.  This is used by the native
/// context menu and is intentionally independent from any map provider.
pub fn screen_point_to_geo(
    frame: &FramePlan,
    frame_size: [f32; 2],
    point_px: [f32; 2],
) -> Option<[f64; 2]> {
    let [width, height] = frame_size;
    if !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
        || !point_px.iter().all(|value| value.is_finite())
        || !frame.view_span.is_finite()
        || frame.view_span <= 0.0
        || !frame.view_span_y.is_finite()
        || frame.view_span_y <= 0.0
    {
        return None;
    }
    let ndc_x = point_px[0] as f64 / width as f64 * 2.0 - 1.0;
    let ndc_y = 1.0 - point_px[1] as f64 / height as f64 * 2.0;
    let x = frame.view_center_mercator[0] + ndc_x * frame.view_span * 0.5;
    let y = frame.view_center_mercator[1] - ndc_y * frame.view_span_y * 0.5;
    if !x.is_finite() || !y.is_finite() || !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
        return None;
    }
    let longitude = x * 360.0 - 180.0;
    let latitude = ((std::f64::consts::PI * (1.0 - 2.0 * y)).sinh())
        .atan()
        .to_degrees()
        .clamp(-85.051_128_78, 85.051_128_78);
    Some([latitude, longitude])
}

pub fn build_frame(scene: &Scene, progress: f64) -> FramePlan {
    let progress = progress.clamp(0.0, 1.0);
    let projected: Vec<_> = scene
        .track
        .points
        .iter()
        .map(|point| mercator(point.lon, point.lat))
        .collect();
    let landmark_projected: Vec<_> = scene
        .landmarks
        .iter()
        .filter(|landmark| landmark.enabled)
        .map(|landmark| mercator(landmark.longitude, landmark.latitude))
        .collect();
    let mut bounds = projected.iter().chain(landmark_projected.iter());
    let first = bounds.next().copied().unwrap_or([0.5, 0.5]);
    let (min_x, max_x, min_y, max_y) = bounds.fold(
        (first[0], first[0], first[1], first[1]),
        |(min_x, max_x, min_y, max_y), value| {
            (
                min_x.min(value[0]),
                max_x.max(value[0]),
                min_y.min(value[1]),
                max_y.max(value[1]),
            )
        },
    );
    let fit_center = [(min_x + max_x) * 0.5, (min_y + max_y) * 0.5];
    let canvas = logical_canvas(scene.options.aspect);
    let viewport_width = canvas.width;
    let viewport_height = canvas.height;
    let aspect_height_over_width = viewport_height / viewport_width;
    let fit_span = (max_x - min_x)
        .max((max_y - min_y) / aspect_height_over_width.max(1e-9))
        .max(1e-12)
        * 1.20;
    let fit_span_y = fit_span * aspect_height_over_width;
    let follow_span =
        viewport_width / (256.0 * 2.0_f64.powf(scene.options.follow_zoom_level.clamp(2.0, 20.0)));
    let follow_span = follow_span.clamp(1e-9, 1.0);
    let follow_span_y = follow_span * aspect_height_over_width;
    let sample = sample_distance(&scene.track, progress);
    let (center, span, span_y) = match scene.options.camera_mode {
        CameraMode::Fit => (fit_center, fit_span, fit_span_y),
        CameraMode::Follow => (
            scene
                .options
                .preview_center_mercator
                .unwrap_or_else(|| mercator(sample.lon, sample.lat)),
            scene
                .options
                .preview_zoom_level
                .map_or(follow_span, |zoom| {
                    (viewport_width / (256.0 * 2.0_f64.powf(zoom.clamp(2.0, 20.0))))
                        .clamp(1e-9, 1.0)
                }),
            scene
                .options
                .preview_zoom_level
                .map_or(follow_span_y, |zoom| {
                    (viewport_width / (256.0 * 2.0_f64.powf(zoom.clamp(2.0, 20.0))))
                        .clamp(1e-9, 1.0)
                        * aspect_height_over_width
                }),
        ),
        CameraMode::Free => {
            let center = scene
                .options
                .free_camera_center
                .map(|value| mercator(value[0], value[1]))
                .unwrap_or(fit_center);
            (
                center,
                fit_span / scene.options.camera_zoom.clamp(0.25, 64.0),
                fit_span_y / scene.options.camera_zoom.clamp(0.25, 64.0),
            )
        }
    };
    let to_ndc = |value: [f64; 2]| {
        [
            ((value[0] - center[0]) * 2.0 / span) as f32,
            (-(value[1] - center[1]) * 2.0 / span_y) as f32,
        ]
    };
    let route_ndc: Vec<_> = projected.into_iter().map(to_ndc).collect();
    let marker_ndc = to_ndc(mercator(sample.lon, sample.lat));
    let completed_points = scene
        .track
        .points
        .partition_point(|point| point.distance_m <= sample.distance_m)
        .max(1);
    let elevations: Vec<_> = scene
        .track
        .points
        .iter()
        .filter_map(|point| point.elevation_m)
        .collect();
    let min_elevation = elevations.iter().copied().fold(f64::INFINITY, f64::min);
    let max_elevation = elevations.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let elevation_span = (max_elevation - min_elevation).max(1.0);
    let elevation_line = scene
        .track
        .points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            point.elevation_m.map(|elevation| {
                [
                    index as f32 / (scene.track.points.len() - 1) as f32 * 2.0 - 1.0,
                    ((elevation - min_elevation) / elevation_span * 2.0 - 1.0) as f32,
                ]
            })
        })
        .collect();
    let landmarks = scene
        .landmarks
        .iter()
        .filter(|landmark| landmark.enabled)
        .map(|landmark| {
            landmark_frame(
                landmark,
                to_ndc(mercator(landmark.longitude, landmark.latitude)),
                progress,
                scene.route_duration_seconds,
            )
        })
        .collect();
    FramePlan {
        view_center_mercator: center,
        view_span: span,
        view_span_y: span_y,
        route_ndc,
        completed_points,
        marker_ndc,
        elevation_line,
        progress: progress as f32,
        distance_m: sample.distance_m,
        elevation_m: sample.elevation_m,
        landmarks,
    }
}

pub fn blend_frames(from: &FramePlan, to: &FramePlan, amount: f64) -> FramePlan {
    let t = amount.clamp(0.0, 1.0);
    let lerp = |a: f64, b: f64| a + (b - a) * t;
    let center = [
        lerp(from.view_center_mercator[0], to.view_center_mercator[0]),
        lerp(from.view_center_mercator[1], to.view_center_mercator[1]),
    ];
    let span = lerp(from.view_span, to.view_span);
    let span_y = lerp(from.view_span_y, to.view_span_y);
    let convert = |point: [f32; 2], source: &FramePlan| {
        let world = [
            source.view_center_mercator[0] + point[0] as f64 * source.view_span * 0.5,
            source.view_center_mercator[1] - point[1] as f64 * source.view_span_y * 0.5,
        ];
        [
            ((world[0] - center[0]) * 2.0 / span) as f32,
            (-(world[1] - center[1]) * 2.0 / span_y) as f32,
        ]
    };
    FramePlan {
        view_center_mercator: center,
        view_span: span,
        view_span_y: span_y,
        route_ndc: from.route_ndc.iter().map(|&p| convert(p, from)).collect(),
        completed_points: to.completed_points,
        marker_ndc: convert(from.marker_ndc, from),
        elevation_line: to.elevation_line.clone(),
        progress: to.progress,
        distance_m: to.distance_m,
        elevation_m: to.elevation_m,
        landmarks: from
            .landmarks
            .iter()
            .map(|landmark| LandmarkFrame {
                ndc: convert(landmark.ndc, from),
                ..landmark.clone()
            })
            .collect(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub hud: [f32; 4],
    pub elevation: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OverlayLayout {
    /// Normalized left, top, width and height values.
    pub hud: [f32; 4],
    pub elevation: [f32; 4],
    pub safe_margin: f32,
    pub reference_height: f32,
}

pub fn overlay_layout(_aspect: Aspect) -> OverlayLayout {
    OverlayLayout {
        hud: [0.04, 0.04, 0.46, 0.12],
        elevation: [0.73, 0.05, 0.23, 0.11],
        safe_margin: 0.04,
        reference_height: 2160.0,
    }
}

pub fn layout(aspect: Aspect, long_edge: u32) -> Layout {
    let (width, height) = match aspect {
        Aspect::Landscape => (long_edge, long_edge * 9 / 16),
        Aspect::Square => (long_edge, long_edge),
        Aspect::Portrait => (long_edge * 9 / 16, long_edge),
    };
    Layout {
        width,
        height,
        hud: [
            width as f32 * 0.04,
            height as f32 * 0.04,
            width as f32 * 0.46,
            height as f32 * 0.12,
        ],
        elevation: [
            width as f32 * 0.73,
            height as f32 * 0.05,
            width as f32 * 0.23,
            height as f32 * 0.11,
        ],
    }
}

/// Compute the centered preview rectangle used by the desktop UI. The export
/// renderer always produces the exact aspect dimensions; the preview uses
/// this same math and letterboxes unused space instead of stretching it.
pub fn fit_aspect_rect(available_width: f32, available_height: f32, aspect: Aspect) -> [f32; 4] {
    let width = available_width.max(0.0);
    let height = available_height.max(0.0);
    let ratio = match aspect {
        Aspect::Landscape => 16.0 / 9.0,
        Aspect::Square => 1.0,
        Aspect::Portrait => 9.0 / 16.0,
    };
    if width <= 0.0 || height <= 0.0 {
        return [0.0, 0.0, 0.0, 0.0];
    }
    let (frame_width, frame_height) = if width / height > ratio {
        (height * ratio, height)
    } else {
        (width, width / ratio)
    };
    [
        (width - frame_width) * 0.5,
        (height - frame_height) * 0.5,
        frame_width,
        frame_height,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpx_core::{ParseOptions, parse_gpx};
    fn scene() -> Scene {
        let options = SceneOptions {
            camera_mode: CameraMode::Fit,
            ..SceneOptions::default()
        };
        Scene { track: parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"><ele>10</ele></trkpt><trkpt lat="25.01" lon="121.01"><ele>20</ele></trkpt><trkpt lat="25.02" lon="121.03"><ele>15</ele></trkpt></trkseg></trk></gpx>"#, ParseOptions::default()).unwrap(), options, landmarks: vec![], route_duration_seconds: 20.0 }
    }
    #[test]
    fn defaults_match_product_contract() {
        let value = SceneOptions::default();
        assert_eq!(value.map_style, MapStyle::Satellite);
        assert_eq!(value.camera_mode, CameraMode::Follow);
        assert_eq!(value.line_width_px, 8.0);
        assert_eq!(value.follow_zoom_level, 15.0);
        assert_eq!(
            (
                value.camera_viewport_width_px,
                value.camera_viewport_height_px
            ),
            (3840, 2160)
        );
    }
    #[test]
    fn web_mercator_matches_known_taiwan_coordinate() {
        let point = mercator(121.0, 25.0);
        assert!((point[0] - 0.836_111_111).abs() < 1e-8);
        assert!((point[1] - 0.428_240_963).abs() < 1e-8);
    }
    #[test]
    fn aspect_dimensions_are_exact() {
        assert_eq!(
            (
                layout(Aspect::Landscape, 3840).width,
                layout(Aspect::Landscape, 3840).height
            ),
            (3840, 2160)
        );
        assert_eq!(
            (
                layout(Aspect::Portrait, 3840).width,
                layout(Aspect::Portrait, 3840).height
            ),
            (2160, 3840)
        );
        assert_eq!(
            (
                layout(Aspect::Square, 2160).width,
                layout(Aspect::Square, 2160).height
            ),
            (2160, 2160)
        );
    }
    #[test]
    fn overlays_stay_inside_frame() {
        for aspect in [Aspect::Landscape, Aspect::Square, Aspect::Portrait] {
            let value = layout(aspect, 3840);
            assert!(value.hud[0] + value.hud[2] <= value.width as f32);
            assert!(value.elevation[1] + value.elevation[3] <= value.height as f32);
        }
    }

    #[test]
    fn overlay_layout_has_stable_safe_rects() {
        for aspect in [Aspect::Landscape, Aspect::Square, Aspect::Portrait] {
            let value = overlay_layout(aspect);
            for rect in [value.hud, value.elevation] {
                assert!(rect[0] >= value.safe_margin);
                assert!(rect[1] >= value.safe_margin);
                assert!(rect[0] + rect[2] <= 1.0);
                assert!(rect[1] + rect[3] <= 1.0);
            }
        }
    }
    #[test]
    fn preview_rect_preserves_selected_aspect_and_is_centered() {
        let landscape = fit_aspect_rect(1000.0, 800.0, Aspect::Landscape);
        assert!((landscape[2] / landscape[3] - 16.0 / 9.0).abs() < 1e-5);
        assert!((landscape[0] - 0.0).abs() < 1e-5);
        assert!((landscape[1] - 118.75).abs() < 1e-3);
        let portrait = fit_aspect_rect(1000.0, 800.0, Aspect::Portrait);
        assert!((portrait[2] / portrait[3] - 9.0 / 16.0).abs() < 1e-5);
        assert!((portrait[0] - 275.0).abs() < 1e-3);
    }
    #[test]
    fn frame_plan_keeps_route_and_marker_in_ndc() {
        let frame = build_frame(&scene(), 0.5);
        assert!(
            frame
                .route_ndc
                .iter()
                .flatten()
                .all(|value| (-1.0..=1.0).contains(value))
        );
        assert!(
            frame
                .marker_ndc
                .iter()
                .all(|value| (-1.0..=1.0).contains(value))
        );
        assert_eq!(frame.elevation_line.len(), 3);
    }
    #[test]
    fn frame_plan_progress_uses_route_distance() {
        let scene = scene();
        let frame = build_frame(&scene, 0.5);
        assert!((frame.distance_m - scene.track.distance_m * 0.5).abs() < 0.01);
        assert!(frame.completed_points >= 1);
    }
    #[test]
    fn follow_camera_centers_marker() {
        let mut scene = scene();
        scene.options.camera_mode = CameraMode::Follow;
        let frame = build_frame(&scene, 0.6);
        assert!(frame.marker_ndc[0].abs() < 1e-5 && frame.marker_ndc[1].abs() < 1e-5);
    }

    #[test]
    fn follow_zoom_is_aspect_correct_and_preview_center_is_temporary() {
        let mut value = scene();
        value.options.camera_mode = CameraMode::Follow;
        value.options.follow_zoom_level = 15.0;
        value.options.camera_viewport_width_px = 3840;
        value.options.camera_viewport_height_px = 2160;
        let landscape = build_frame(&value, 0.5);
        assert!((landscape.view_span_y / landscape.view_span - 2160.0 / 3840.0).abs() < 1e-9);
        let center = landscape.view_center_mercator;
        value.options.preview_center_mercator = Some([center[0] + 0.01, center[1] + 0.01]);
        value.options.preview_zoom_level = Some(14.0);
        let preview = build_frame(&value, 0.5);
        assert_ne!(preview.view_center_mercator, landscape.view_center_mercator);
        assert!(preview.view_span > landscape.view_span);
    }

    #[test]
    fn camera_composition_is_independent_of_physical_preview_or_export_size() {
        let mut value = scene();
        value.options.camera_mode = CameraMode::Follow;
        value.options.camera_viewport_width_px = 640;
        value.options.camera_viewport_height_px = 360;
        let preview = build_frame(&value, 0.42);
        value.options.camera_viewport_width_px = 3840;
        value.options.camera_viewport_height_px = 2160;
        let export = build_frame(&value, 0.42);
        assert_eq!(preview.view_center_mercator, export.view_center_mercator);
        assert_eq!(preview.view_span, export.view_span);
        assert_eq!(preview.view_span_y, export.view_span_y);
        assert_eq!(preview.route_ndc, export.route_ndc);
    }

    #[test]
    fn logical_canvas_has_expected_orientation_and_dimensions() {
        assert_eq!(
            logical_canvas(Aspect::Landscape),
            LogicalCanvas {
                width: 1920.0,
                height: 1080.0
            }
        );
        assert_eq!(
            logical_canvas(Aspect::Square),
            LogicalCanvas {
                width: 1080.0,
                height: 1080.0
            }
        );
        assert_eq!(
            logical_canvas(Aspect::Portrait),
            LogicalCanvas {
                width: 1080.0,
                height: 1920.0
            }
        );
    }

    #[test]
    fn landmark_anchor_keeps_real_coordinate_and_finds_route_progress() {
        let value = scene();
        let anchor = anchor_landmark_to_route(&value.track, 25.005, 121.005).unwrap();
        assert!(anchor.anchor_progress > 0.1 && anchor.anchor_progress < 0.6);
        assert!(anchor.distance_from_route_m < 100.0);
        assert!((anchor.nearest_latitude - 25.005).abs() < 0.002);
    }

    #[test]
    fn landmark_reveal_is_hidden_then_persistent() {
        let mut value = scene();
        let anchor = anchor_landmark_to_route(&value.track, 25.01, 121.01).unwrap();
        value.landmarks.push(RouteLandmark {
            id: "overture:test".into(),
            source: LandmarkSource::Overture,
            source_id: Some("test".into()),
            name: "Test place".into(),
            category: Some("park".into()),
            latitude: 25.01,
            longitude: 121.01,
            anchor_distance_m: anchor.anchor_distance_m,
            anchor_progress: anchor.anchor_progress,
            distance_from_route_m: anchor.distance_from_route_m,
            enabled: true,
            style: LandmarkStyle::default(),
        });
        let before = build_frame(&value, (anchor.anchor_progress - 0.05).max(0.0));
        assert_eq!(before.landmarks[0].pin_opacity, 0.0);
        let visible = build_frame(&value, anchor.anchor_progress + 0.02);
        assert!(visible.landmarks[0].pin_opacity > 0.0);
        assert_eq!(visible.landmarks[0].category.as_deref(), Some("Park"));
        let final_frame = build_frame(&value, 1.0);
        assert_eq!(final_frame.landmarks[0].pin_opacity, 1.0);
        assert_eq!(final_frame.landmarks[0].label_opacity, 0.0);
    }

    #[test]
    fn fit_camera_includes_enabled_landmarks() {
        let mut value = scene();
        value.options.camera_mode = CameraMode::Fit;
        let route_only = build_frame(&value, 1.0);
        let anchor = anchor_landmark_to_route(&value.track, 25.10, 121.30).unwrap();
        value.landmarks.push(RouteLandmark {
            id: "manual:far".into(),
            source: LandmarkSource::Manual,
            source_id: None,
            name: "Far marker".into(),
            category: None,
            latitude: 25.10,
            longitude: 121.30,
            anchor_distance_m: anchor.anchor_distance_m,
            anchor_progress: anchor.anchor_progress,
            distance_from_route_m: anchor.distance_from_route_m,
            enabled: true,
            style: LandmarkStyle::default(),
        });
        let with_landmark = build_frame(&value, 1.0);
        assert!(with_landmark.view_span > route_only.view_span);
    }
    #[test]
    fn follow_to_fit_blend_has_exact_camera_endpoints() {
        let mut value = scene();
        value.options.camera_mode = CameraMode::Follow;
        let follow = build_frame(&value, 1.0);
        value.options.camera_mode = CameraMode::Fit;
        let fit = build_frame(&value, 1.0);
        assert_eq!(blend_frames(&follow, &fit, 0.0).view_span, follow.view_span);
        assert_eq!(blend_frames(&follow, &fit, 1.0).view_span, fit.view_span);
        assert_eq!(
            blend_frames(&follow, &fit, 1.0).view_center_mercator,
            fit.view_center_mercator
        );
    }
    #[test]
    fn free_camera_uses_requested_center_and_zoom() {
        let mut scene = scene();
        scene.options.camera_mode = CameraMode::Free;
        scene.options.free_camera_center = Some([121.01, 25.01]);
        scene.options.camera_zoom = 4.0;
        let frame = build_frame(&scene, 0.5);
        assert!(
            frame
                .route_ndc
                .iter()
                .any(|point| point[0].abs() < 0.05 && point[1].abs() < 0.05)
        );
    }

    #[test]
    fn screen_point_to_geo_maps_frame_center_and_rejects_invalid_input() {
        let frame = build_frame(&scene(), 0.5);
        let center = screen_point_to_geo(&frame, [1_600.0, 900.0], [800.0, 450.0]).unwrap();
        assert!((center[0] - 25.01).abs() < 0.02);
        assert!((center[1] - 121.01).abs() < 0.03);
        assert!(screen_point_to_geo(&frame, [0.0, 900.0], [0.0, 0.0]).is_none());
        assert!(screen_point_to_geo(&frame, [1_600.0, 900.0], [f32::NAN, 0.0]).is_none());
    }

    #[test]
    fn screen_point_to_geo_round_trips_fit_corners_within_web_mercator_bounds() {
        let frame = build_frame(&scene(), 0.0);
        let top_left = screen_point_to_geo(&frame, [100.0, 100.0], [0.0, 0.0]).unwrap();
        let bottom_right = screen_point_to_geo(&frame, [100.0, 100.0], [100.0, 100.0]).unwrap();
        assert!(top_left[0] > bottom_right[0]);
        assert!(top_left[1] < bottom_right[1]);
        assert!(top_left[0].abs() <= 85.051_128_78);
        assert!(bottom_right[0].abs() <= 85.051_128_78);
    }
}
