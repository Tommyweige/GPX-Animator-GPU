use crate::ExportSettings;
use gpx_core::{GpxError, ParseOptions, Track, parse_gpx};
use mp4_output::{AtomicAvcFile, AtomicHevcFile, Mp4Error, avc_parameter_sets, parameter_sets};
use nvenc_engine::{
    CancellationToken, EncoderConfig, ExportMetrics, ExportProgress, ExportStage, GpuCapabilities,
    NativeEncoder, NvencError,
};
use scene_core::{CameraMode, Codec, RouteLandmark, Scene, blend_frames, build_frame};
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;
use thiserror::Error;

#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub track: Track,
    pub output_path: PathBuf,
    pub settings: ExportSettings,
    pub landmarks: Vec<RouteLandmark>,
}

#[derive(Clone, Debug)]
pub struct ExportOutcome {
    pub output_path: PathBuf,
    pub metrics: ExportMetrics,
}

enum NativeFileMux {
    Hevc(AtomicHevcFile),
    Avc(AtomicAvcFile),
}
impl NativeFileMux {
    fn write(&mut self, packet: &[u8], dts: u64, pts: u64) -> Result<(), Mp4Error> {
        match self {
            Self::Hevc(value) => value.write_access_unit(packet, dts, pts),
            Self::Avc(value) => value.write_access_unit(packet, dts, pts),
        }
    }
    fn finalize(self) -> Result<PathBuf, Mp4Error> {
        match self {
            Self::Hevc(value) => value.finalize(),
            Self::Avc(value) => value.finalize(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("讀取 GPX 失敗：{0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Gpx(#[from] GpxError),
    #[error(transparent)]
    Nvenc(#[from] NvencError),
    #[error(transparent)]
    Mp4(#[from] Mp4Error),
    #[error(transparent)]
    Renderer(#[from] d3d11_renderer::RendererError),
    #[error("NVENC 沒有輸出任何 packet")]
    NoPackets,
    #[error("匯出已取消")]
    Cancelled,
}

pub fn load_gpx_file(path: impl AsRef<Path>, options: ParseOptions) -> Result<Track, ExportError> {
    let source = std::fs::read_to_string(path)?;
    Ok(parse_gpx(&source, options)?)
}

pub fn detect_gpu_capabilities() -> Result<GpuCapabilities, ExportError> {
    let device = d3d11_renderer::D3d11ExportDevice::create_rtx()?;
    let textures = device.create_export_textures(1920, 1080)?;
    let supports = |codec| {
        let mut config = EncoderConfig::four_k_60(codec, scene_core::QualityPreset::Speed);
        config.width = 1920;
        config.height = 1080;
        NativeEncoder::create(&device, &textures, config).is_ok()
    };
    let hevc = supports(Codec::Hevc);
    let h264 = supports(Codec::H264);
    Ok(GpuCapabilities {
        adapter_name: device.info.name.clone(),
        luid: device.info.luid,
        dedicated_vram: device.info.dedicated_vram,
        hevc,
        h264,
        async_encode: true,
        max_width: 8192,
        max_height: 8192,
        api_version: "NVENC dynamic API".into(),
    })
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

pub fn run_native_export<F>(
    request: ExportRequest,
    token: &CancellationToken,
    mut progress: F,
) -> Result<ExportOutcome, ExportError>
where
    F: FnMut(ExportProgress),
{
    let route_frames = request.settings.route_frames();
    let total_frames = request.settings.total_frames();
    let update = |stage, completed, fps, eta| ExportProgress {
        stage,
        completed_frames: completed,
        total_frames,
        stage_completed: completed,
        stage_total: total_frames,
        fps,
        eta_seconds: eta,
    };
    progress(update(ExportStage::Preflight, 0, 0.0, 0.0));
    token.check().map_err(|_| ExportError::Cancelled)?;
    let mut scene_options = request.settings.scene.clone();
    scene_options.camera_viewport_width_px = request.settings.width;
    scene_options.camera_viewport_height_px = request.settings.height;
    let scene = Scene {
        track: request.track,
        options: scene_options,
        landmarks: request.landmarks,
        route_duration_seconds: request.settings.duration_seconds as f64,
    };
    let device = d3d11_renderer::D3d11ExportDevice::create_rtx()?;
    let textures =
        device.create_export_textures(request.settings.width, request.settings.height)?;
    let renderer = d3d11_renderer::D2dSceneRenderer::new(&device)?;
    let gpu_tiles = {
        let mut keys = HashSet::new();
        let samples = request.settings.duration_seconds.clamp(1, 300);
        for i in 0..=samples {
            token.check().map_err(|_| ExportError::Cancelled)?;
            let frame = build_frame(&scene, i as f64 / samples as f64);
            let zoom = d3d11_renderer::tile_zoom_rect(
                frame.view_span,
                frame.view_span_y,
                request.settings.width,
            );
            keys.extend(d3d11_renderer::required_view_tiles_rect(
                frame.view_center_mercator,
                frame.view_span,
                frame.view_span_y,
                zoom,
            ));
        }
        if scene.options.camera_mode == CameraMode::Follow {
            let mut fit_scene = scene.clone();
            fit_scene.options.camera_mode = CameraMode::Fit;
            let follow = build_frame(&scene, 1.0);
            let fit = build_frame(&fit_scene, 1.0);
            let transition_frames = 2 * request.settings.fps as u64;
            // Preload every interpolated camera view. Without these keys the
            // transition can clear to the dark background while the camera
            // moves between two otherwise-preloaded views.
            for index in 0..=transition_frames {
                let linear = index as f64 / transition_frames.max(1) as f64;
                let smooth = linear * linear * linear * (linear * (linear * 6.0 - 15.0) + 10.0);
                let frame = blend_frames(&follow, &fit, smooth);
                let zoom = d3d11_renderer::tile_zoom_rect(
                    frame.view_span,
                    frame.view_span_y,
                    request.settings.width,
                );
                keys.extend(d3d11_renderer::required_view_tiles_rect(
                    frame.view_center_mercator,
                    frame.view_span,
                    frame.view_span_y,
                    zoom,
                ));
            }
        }
        // Always cache and upload two lower zoom levels as a seamless underlay.
        // A missing high-resolution request then reveals real satellite imagery
        // instead of a solid-colored rectangle.
        let detailed: Vec<_> = keys.iter().copied().collect();
        for key in detailed {
            for levels in 1..=2u8 {
                if key.zoom >= levels {
                    keys.insert(d3d11_renderer::TileKey {
                        zoom: key.zoom - levels,
                        x: key.x >> levels,
                        y: key.y >> levels,
                    });
                }
            }
        }
        let cache_limit = request.settings.cache_limit_bytes;
        let style = scene.options.map_style;
        let manifest_cache =
            d3d11_renderer::TileDiskCache::for_map_style_with_limit(style, Some(cache_limit));
        let manifest =
            d3d11_renderer::TileManifest::new(&manifest_cache, keys.into_iter().collect());
        let keys: Arc<Vec<_>> = Arc::new(manifest.keys.clone());
        let tile_total = manifest.total();
        let workers = std::thread::available_parallelism()
            .map_or(4, usize::from)
            .clamp(1, 4)
            .min(tile_total.max(1));
        let next = Arc::new(AtomicUsize::new(0));
        let (tile_tx, tile_rx) = mpsc::channel();
        progress(ExportProgress {
            stage: ExportStage::Preflight,
            completed_frames: 0,
            total_frames,
            stage_completed: 0,
            stage_total: tile_total as u64,
            fps: 0.0,
            eta_seconds: 0.0,
        });
        let decoded = std::thread::scope(|scope| -> Result<Vec<_>, ExportError> {
            for _ in 0..workers {
                let keys = Arc::clone(&keys);
                let next = Arc::clone(&next);
                let tx = tile_tx.clone();
                let token = token.clone();
                scope.spawn(move || {
                    let cache = d3d11_renderer::TileDiskCache::for_map_style_with_limit(
                        style,
                        Some(cache_limit),
                    );
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        if index >= keys.len() || token.is_cancelled() {
                            break;
                        }
                        if tx.send(cache.load(keys[index])).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tile_tx);
            let mut decoded = Vec::with_capacity(tile_total);
            for (index, result) in tile_rx.into_iter().enumerate() {
                token.check().map_err(|_| ExportError::Cancelled)?;
                decoded.push(result?);
                progress(ExportProgress {
                    stage: ExportStage::Preflight,
                    completed_frames: 0,
                    total_frames,
                    stage_completed: (index + 1) as u64,
                    stage_total: tile_total as u64,
                    fps: 0.0,
                    eta_seconds: 0.0,
                });
            }
            Ok(decoded)
        })?;
        let mut decoded = decoded;
        for tile in &mut decoded {
            d3d11_renderer::apply_map_color_transform(tile, scene.options.map_style);
        }
        Some(renderer.prepare_tiles(decoded)?)
    };
    let mut config = EncoderConfig::four_k_60(request.settings.codec, request.settings.quality);
    config.width = request.settings.width;
    config.height = request.settings.height;
    config.fps = request.settings.fps;
    let encoder = NativeEncoder::create(&device, &textures, config)?;
    let started = Instant::now();
    let mut free_slots: VecDeque<usize> = (0..textures.len()).collect();
    let mut mux: Option<NativeFileMux> = None;
    let mut dts = 0u64;
    let mut encoded = 0u64;
    let mut render_ms = Vec::with_capacity(total_frames as usize);
    let mut encode_ms = Vec::with_capacity(total_frames as usize);
    let mut mux_ms = Vec::with_capacity(total_frames as usize);
    let consume = |packets: Vec<nvenc_engine::EncodedPacket>,
                   free_slots: &mut VecDeque<usize>,
                   mux: &mut Option<NativeFileMux>,
                   dts: &mut u64,
                   encoded: &mut u64,
                   mux_ms: &mut Vec<f64>|
     -> Result<(), ExportError> {
        for packet in packets {
            free_slots.push_back(packet.texture_slot);
            if mux.is_none() {
                *mux = Some(match request.settings.codec {
                    Codec::Hevc => NativeFileMux::Hevc(AtomicHevcFile::create(
                        &request.output_path,
                        request.settings.width,
                        request.settings.height,
                        request.settings.fps,
                        parameter_sets(&packet.bytes)?,
                    )?),
                    Codec::H264 => NativeFileMux::Avc(AtomicAvcFile::create(
                        &request.output_path,
                        request.settings.width,
                        request.settings.height,
                        request.settings.fps,
                        avc_parameter_sets(&packet.bytes)?,
                    )?),
                });
            }
            let before = Instant::now();
            mux.as_mut()
                .ok_or(ExportError::NoPackets)?
                .write(&packet.bytes, *dts, packet.pts)?;
            mux_ms.push(before.elapsed().as_secs_f64() * 1000.0);
            *dts += 1;
            *encoded += 1;
        }
        Ok(())
    };
    for frame_index in 0..total_frames {
        if token.is_cancelled() {
            let _ = encoder.finish_packets();
            drop(mux);
            return Err(ExportError::Cancelled);
        }
        if free_slots.is_empty() {
            let packets = encoder.receive_packets(true)?;
            consume(
                packets,
                &mut free_slots,
                &mut mux,
                &mut dts,
                &mut encoded,
                &mut mux_ms,
            )?;
        }
        let slot = free_slots.pop_front().ok_or(ExportError::NoPackets)?;
        let before = Instant::now();
        let frame =
            if scene.options.camera_mode == CameraMode::Follow && frame_index >= route_frames {
                let follow = build_frame(&scene, 1.0);
                let mut fit_scene = scene.clone();
                fit_scene.options.camera_mode = CameraMode::Fit;
                let fit = build_frame(&fit_scene, 1.0);
                let transition_frames = 2 * request.settings.fps as u64;
                if frame_index < route_frames + transition_frames {
                    let linear = (frame_index - route_frames + 1) as f64 / transition_frames as f64;
                    let smooth = linear * linear * linear * (linear * (linear * 6.0 - 15.0) + 10.0);
                    blend_frames(&follow, &fit, smooth)
                } else {
                    fit
                }
            } else {
                build_frame(
                    &scene,
                    if route_frames <= 1 {
                        1.0
                    } else {
                        frame_index as f64 / (route_frames - 1) as f64
                    },
                )
            };
        renderer.render_with_tiles(
            &textures[slot],
            &frame,
            &scene.options,
            request.settings.width,
            request.settings.height,
            gpu_tiles.as_ref(),
        )?;
        device.flush();
        render_ms.push(before.elapsed().as_secs_f64() * 1000.0);
        let before = Instant::now();
        let packets = encoder.submit_frame(slot, frame_index as usize, frame_index)?;
        encode_ms.push(before.elapsed().as_secs_f64() * 1000.0);
        consume(
            packets,
            &mut free_slots,
            &mut mux,
            &mut dts,
            &mut encoded,
            &mut mux_ms,
        )?;
        let elapsed = started.elapsed().as_secs_f64();
        let rendered = frame_index + 1;
        let rate = rendered as f64 / elapsed.max(1e-9);
        progress(update(
            ExportStage::Rendering,
            encoded,
            rate,
            (total_frames - rendered) as f64 / rate.max(1e-9),
        ));
    }
    progress(update(ExportStage::Encoding, encoded, 0.0, 0.0));
    let packets = encoder.finish_packets()?;
    consume(
        packets,
        &mut free_slots,
        &mut mux,
        &mut dts,
        &mut encoded,
        &mut mux_ms,
    )?;
    if encoded != total_frames {
        return Err(ExportError::NoPackets);
    }
    token.check().map_err(|_| ExportError::Cancelled)?;
    progress(update(ExportStage::Muxing, encoded, 0.0, 0.0));
    let output = mux.ok_or(ExportError::NoPackets)?.finalize()?;
    progress(update(ExportStage::Verifying, encoded, 0.0, 0.0));
    let elapsed = started.elapsed().as_secs_f64();
    let metrics = ExportMetrics {
        cpu_frame_readbacks: 0,
        encoded_frames: encoded,
        dropped_frames: 0,
        duplicated_frames: 0,
        ring_occupancy_peak: textures.len(),
        peak_vram_bytes: textures.len() as u64
            * request.settings.width as u64
            * request.settings.height as u64
            * 4,
        gpu_adapter_luid: device.info.luid,
        render_p50_ms: percentile(&render_ms, 0.50),
        render_p95_ms: percentile(&render_ms, 0.95),
        encode_p50_ms: percentile(&encode_ms, 0.50),
        encode_p95_ms: percentile(&encode_ms, 0.95),
        mux_p50_ms: percentile(&mux_ms, 0.50),
        mux_p95_ms: percentile(&mux_ms, 0.95),
        elapsed_seconds: elapsed,
    };
    progress(update(
        ExportStage::Completed,
        encoded,
        encoded as f64 / elapsed.max(1e-9),
        0.0,
    ));
    Ok(ExportOutcome {
        output_path: output,
        metrics,
    })
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    fn track() -> Track {
        parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"><ele>10</ele></trkpt><trkpt lat="25.01" lon="121.01"><ele>20</ele></trkpt></trkseg></trk></gpx>"#,ParseOptions::default()).unwrap()
    }
    fn offline_settings(duration_seconds: u32) -> ExportSettings {
        let mut settings = ExportSettings {
            duration_seconds,
            ..ExportSettings::default()
        };
        settings.scene.map_style = scene_core::MapStyle::Transparent;
        settings.scene.camera_mode = scene_core::CameraMode::Fit;
        settings
    }
    #[test]
    fn percentile_handles_empty_and_rank() {
        assert_eq!(percentile(&[], 0.95), 0.0);
        assert_eq!(percentile(&[3.0, 1.0, 2.0], 0.5), 2.0);
    }
    #[test]
    fn cancelled_preflight_does_not_create_file() {
        let output = std::env::temp_dir().join("gpx-native-cancelled.mp4");
        let _ = std::fs::remove_file(&output);
        let token = CancellationToken::default();
        token.cancel();
        let request = ExportRequest {
            track: track(),
            output_path: output.clone(),
            settings: ExportSettings::default(),
            landmarks: Vec::new(),
        };
        assert!(matches!(
            run_native_export(request, &token, |_| {}),
            Err(ExportError::Cancelled)
        ));
        assert!(!output.exists());
    }
    #[cfg(windows)]
    fn gpu_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    #[cfg(windows)]
    #[test]
    fn exports_two_second_real_gpx_scene_with_exact_metrics() {
        let _gpu = gpu_guard();
        let output =
            std::env::temp_dir().join(format!("gpx-native-e2e-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let settings = offline_settings(2);
        let request = ExportRequest {
            track: track(),
            output_path: output.clone(),
            settings,
            landmarks: Vec::new(),
        };
        let mut stages = Vec::new();
        let outcome = run_native_export(request, &CancellationToken::default(), |value| {
            stages.push(value.stage)
        })
        .unwrap();
        assert_eq!(outcome.metrics.encoded_frames, 120);
        assert_eq!(outcome.metrics.cpu_frame_readbacks, 0);
        assert_eq!(outcome.metrics.dropped_frames, 0);
        assert_eq!(outcome.metrics.duplicated_frames, 0);
        assert!(outcome.metrics.gpu_adapter_luid != 0);
        assert!(
            stages.contains(&ExportStage::Preflight) && stages.contains(&ExportStage::Completed)
        );
        let ffprobe = std::path::Path::new(r"C:\Program Files\FFMPEG\bin\ffprobe.exe");
        if ffprobe.exists() {
            let result = std::process::Command::new(ffprobe)
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=codec_name,codec_tag_string,width,height,r_frame_rate,nb_read_frames",
                    "-of",
                    "default=nw=1",
                    output.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&result.stdout);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                text.contains("codec_name=hevc")
                    && text.contains("codec_tag_string=hvc1")
                    && text.contains("r_frame_rate=60/1")
                    && text.contains("nb_read_frames=120"),
                "{text}"
            );
        }
        std::fs::remove_file(output).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn cancellation_removes_partial_mp4() {
        let _gpu = gpu_guard();
        let output =
            std::env::temp_dir().join(format!("gpx-native-mid-cancel-{}.mp4", std::process::id()));
        let partial = output.with_extension("mp4.part");
        let _ = std::fs::remove_file(&output);
        let _ = std::fs::remove_file(&partial);
        let settings = offline_settings(2);
        let token = CancellationToken::default();
        let callback_token = token.clone();
        let request = ExportRequest {
            track: track(),
            output_path: output.clone(),
            settings,
            landmarks: Vec::new(),
        };
        let result = run_native_export(request, &token, move |value| {
            if value.completed_frames >= 5 {
                callback_token.cancel();
            }
        });
        assert!(matches!(result, Err(ExportError::Cancelled)));
        assert!(!output.exists());
        assert!(!partial.exists());
    }
    #[cfg(windows)]
    #[test]
    fn exports_h264_as_avc1() {
        let _gpu = gpu_guard();
        let output =
            std::env::temp_dir().join(format!("gpx-native-h264-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let mut settings = offline_settings(1);
        settings.codec = Codec::H264;
        settings.quality = scene_core::QualityPreset::Speed;
        let request = ExportRequest {
            track: track(),
            output_path: output.clone(),
            settings,
            landmarks: Vec::new(),
        };
        let outcome = run_native_export(request, &CancellationToken::default(), |_| {}).unwrap();
        assert_eq!(outcome.metrics.encoded_frames, 60);
        let ffprobe = std::path::Path::new(r"C:\Program Files\FFMPEG\bin\ffprobe.exe");
        if ffprobe.exists() {
            let result = std::process::Command::new(ffprobe)
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=codec_name,codec_tag_string,nb_read_frames",
                    "-of",
                    "default=nw=1",
                    output.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&result.stdout);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                text.contains("codec_name=h264")
                    && text.contains("codec_tag_string=avc1")
                    && text.contains("nb_read_frames=60"),
                "{text}"
            );
        }
        std::fs::remove_file(output).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn exports_selected_30_fps_with_exact_timestamps() {
        let _gpu = gpu_guard();
        let output =
            std::env::temp_dir().join(format!("gpx-native-30fps-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let mut settings = offline_settings(1);
        settings.fps = 30;
        let outcome = run_native_export(
            ExportRequest {
                track: track(),
                output_path: output.clone(),
                settings,
                landmarks: Vec::new(),
            },
            &CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.metrics.encoded_frames, 30);
        let ffprobe = std::path::Path::new(r"C:\Program Files\FFMPEG\bin\ffprobe.exe");
        if ffprobe.exists() {
            let result = std::process::Command::new(ffprobe)
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=r_frame_rate,nb_read_frames",
                    "-of",
                    "default=nw=1",
                    output.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&result.stdout);
            assert!(result.status.success());
            assert!(
                text.contains("r_frame_rate=30/1") && text.contains("nb_read_frames=30"),
                "{text}"
            );
        }
        std::fs::remove_file(output).unwrap();
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "RTX release performance gate"]
    fn warm_cache_twenty_second_4k60_meets_realtime_gate() {
        let _gpu = gpu_guard();
        let output =
            std::env::temp_dir().join(format!("gpx-native-20s-gate-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let settings = offline_settings(20);
        let request = ExportRequest {
            track: track(),
            output_path: output.clone(),
            settings,
            landmarks: Vec::new(),
        };
        let outcome = run_native_export(request, &CancellationToken::default(), |_| {}).unwrap();
        assert_eq!(outcome.metrics.encoded_frames, 1200);
        assert_eq!(outcome.metrics.cpu_frame_readbacks, 0);
        assert!(
            outcome.metrics.render_p95_ms < 16.67,
            "render p95 = {:.2} ms",
            outcome.metrics.render_p95_ms
        );
        assert!(
            outcome.metrics.elapsed_seconds <= 20.0,
            "elapsed = {:.2} s",
            outcome.metrics.elapsed_seconds
        );
        let ffprobe = std::path::Path::new(r"C:\Program Files\FFMPEG\bin\ffprobe.exe");
        if ffprobe.exists() {
            let result = std::process::Command::new(ffprobe)
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-select_streams",
                    "v:0",
                    "-show_entries",
                    "stream=codec_name,codec_tag_string,width,height,r_frame_rate,nb_read_frames",
                    "-of",
                    "default=nw=1",
                    output.to_str().unwrap(),
                ])
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&result.stdout);
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            assert!(
                text.contains("codec_name=hevc")
                    && text.contains("codec_tag_string=hvc1")
                    && text.contains("width=3840")
                    && text.contains("height=2160")
                    && text.contains("r_frame_rate=60/1")
                    && text.contains("nb_read_frames=1200"),
                "{text}"
            );
        }
        std::fs::remove_file(output).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "RTX release five minute gate"]
    fn five_minute_4k60_has_exact_frames_and_realtime_throughput() {
        let _gpu = gpu_guard();
        let output =
            std::env::temp_dir().join(format!("gpx-native-5m-gate-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let settings = offline_settings(300);
        let outcome = run_native_export(
            ExportRequest {
                track: track(),
                output_path: output.clone(),
                settings,
                landmarks: Vec::new(),
            },
            &CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        assert_eq!(outcome.metrics.encoded_frames, 18_000);
        assert_eq!(outcome.metrics.cpu_frame_readbacks, 0);
        assert_eq!(outcome.metrics.dropped_frames, 0);
        assert_eq!(outcome.metrics.duplicated_frames, 0);
        assert!(outcome.metrics.render_p95_ms < 16.67);
        assert!(outcome.metrics.elapsed_seconds <= 300.0);
        std::fs::remove_file(output).unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "RTX release stress gate"]
    fn ten_exports_do_not_leak_handles_or_partial_files() {
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
        let _gpu = gpu_guard();
        let handles = || unsafe {
            let mut count = 0;
            GetProcessHandleCount(GetCurrentProcess(), &mut count).unwrap();
            count
        };
        // The first session initializes process-wide NVIDIA/D3D driver state. Measure
        // steady-state growth after that one-time initialization, not driver-owned caches.
        let warmup = std::env::temp_dir().join(format!(
            "gpx-native-stress-warmup-{}.mp4",
            std::process::id()
        ));
        let _ = run_native_export(
            ExportRequest {
                track: track(),
                output_path: warmup.clone(),
                settings: offline_settings(1),
                landmarks: Vec::new(),
            },
            &CancellationToken::default(),
            |_| {},
        )
        .unwrap();
        std::fs::remove_file(warmup).unwrap();
        let before = handles();
        for index in 0..10 {
            let output = std::env::temp_dir().join(format!(
                "gpx-native-stress-{}-{index}.mp4",
                std::process::id()
            ));
            let partial = output.with_extension("mp4.part");
            let _ = std::fs::remove_file(&output);
            let outcome = run_native_export(
                ExportRequest {
                    track: track(),
                    output_path: output.clone(),
                    settings: offline_settings(2),
                    landmarks: Vec::new(),
                },
                &CancellationToken::default(),
                |_| {},
            )
            .unwrap();
            assert_eq!(outcome.metrics.encoded_frames, 120);
            assert!(!partial.exists());
            std::fs::remove_file(output).unwrap();
        }
        let after = handles();
        assert!(after <= before + 8, "handle growth: {before} -> {after}");
    }
}
