use gpx_core::{Track, sample_distance};
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub camera_zoom: f64,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            aspect: Aspect::Landscape,
            map_style: MapStyle::Satellite,
            camera_mode: CameraMode::Follow,
            route_color: [255, 93, 59, 255],
            marker_color: [255, 243, 214, 255],
            line_width_px: 6.0,
            show_hud: true,
            show_elevation: true,
            free_camera_center: None,
            camera_zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub track: Track,
    pub options: SceneOptions,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FramePlan {
    pub view_center_mercator: [f64; 2],
    pub view_span: f64,
    pub route_ndc: Vec<[f32; 2]>,
    pub completed_points: usize,
    pub marker_ndc: [f32; 2],
    pub elevation_line: Vec<[f32; 2]>,
    pub progress: f32,
    pub distance_m: f64,
    pub elevation_m: Option<f64>,
}

fn mercator(lon: f64, lat: f64) -> [f64; 2] {
    let x = (lon + 180.0) / 360.0;
    let sin = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians().sin();
    let y = 0.5 - ((1.0 + sin) / (1.0 - sin)).ln() / (4.0 * std::f64::consts::PI);
    [x, y]
}

pub fn build_frame(scene: &Scene, progress: f64) -> FramePlan {
    let progress = progress.clamp(0.0, 1.0);
    let projected: Vec<_> = scene
        .track
        .points
        .iter()
        .map(|point| mercator(point.lon, point.lat))
        .collect();
    let min_x = projected
        .iter()
        .map(|value| value[0])
        .fold(f64::INFINITY, f64::min);
    let max_x = projected
        .iter()
        .map(|value| value[0])
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = projected
        .iter()
        .map(|value| value[1])
        .fold(f64::INFINITY, f64::min);
    let max_y = projected
        .iter()
        .map(|value| value[1])
        .fold(f64::NEG_INFINITY, f64::max);
    let fit_center = [(min_x + max_x) * 0.5, (min_y + max_y) * 0.5];
    let fit_span = (max_x - min_x).max(max_y - min_y).max(1e-12) * 1.20;
    let sample = sample_distance(&scene.track, progress);
    let (center, span) = match scene.options.camera_mode {
        CameraMode::Fit => (fit_center, fit_span),
        CameraMode::Follow => (
            mercator(sample.lon, sample.lat),
            fit_span / scene.options.camera_zoom.max(2.5),
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
            )
        }
    };
    let to_ndc = |value: [f64; 2]| {
        [
            ((value[0] - center[0]) * 2.0 / span) as f32,
            (-(value[1] - center[1]) * 2.0 / span) as f32,
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
    FramePlan {
        view_center_mercator: center,
        view_span: span,
        route_ndc,
        completed_points,
        marker_ndc,
        elevation_line,
        progress: progress as f32,
        distance_m: sample.distance_m,
        elevation_m: sample.elevation_m,
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
    let convert = |point: [f32; 2], source: &FramePlan| {
        let world = [
            source.view_center_mercator[0] + point[0] as f64 * source.view_span * 0.5,
            source.view_center_mercator[1] - point[1] as f64 * source.view_span * 0.5,
        ];
        [
            ((world[0] - center[0]) * 2.0 / span) as f32,
            (-(world[1] - center[1]) * 2.0 / span) as f32,
        ]
    };
    FramePlan {
        view_center_mercator: center,
        view_span: span,
        route_ndc: from.route_ndc.iter().map(|&p| convert(p, from)).collect(),
        completed_points: to.completed_points,
        marker_ndc: convert(from.marker_ndc, from),
        elevation_line: to.elevation_line.clone(),
        progress: to.progress,
        distance_m: to.distance_m,
        elevation_m: to.elevation_m,
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
        let mut options = SceneOptions::default();
        options.camera_mode = CameraMode::Fit;
        Scene { track: parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"><ele>10</ele></trkpt><trkpt lat="25.01" lon="121.01"><ele>20</ele></trkpt><trkpt lat="25.02" lon="121.03"><ele>15</ele></trkpt></trkseg></trk></gpx>"#, ParseOptions::default()).unwrap(), options }
    }
    #[test]
    fn defaults_match_product_contract() {
        let value = SceneOptions::default();
        assert_eq!(value.map_style, MapStyle::Satellite);
        assert_eq!(value.camera_mode, CameraMode::Follow);
        assert_eq!(value.line_width_px, 6.0);
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
}
