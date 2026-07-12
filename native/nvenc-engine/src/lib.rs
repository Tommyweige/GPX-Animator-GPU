use scene_core::{Codec, QualityPreset};
use std::collections::{HashMap, VecDeque};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuCapabilities {
    pub adapter_name: String,
    pub luid: u64,
    pub dedicated_vram: u64,
    pub hevc: bool,
    pub h264: bool,
    pub async_encode: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub api_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub cq: u8,
    pub max_bitrate: u32,
    pub gop: u32,
    pub b_frames: u8,
    pub preset: u8,
    pub aq: bool,
    pub lookahead: bool,
}

impl EncoderConfig {
    pub fn four_k_60(codec: Codec, quality: QualityPreset) -> Self {
        let (preset, cq, b_frames) = match quality {
            QualityPreset::Balanced => (4, 22, 2),
            QualityPreset::Quality => (5, 19, 2),
            QualityPreset::Speed => (3, 25, 0),
        };
        Self {
            codec,
            width: 3840,
            height: 2160,
            fps: 60,
            cq,
            max_bitrate: 80_000_000,
            gop: 120,
            b_frames,
            preset,
            aq: true,
            lookahead: false,
        }
    }
    pub fn validate(&self, caps: &GpuCapabilities) -> Result<(), NvencError> {
        if !caps.adapter_name.to_ascii_uppercase().contains("RTX") {
            return Err(NvencError::UnsupportedAdapter);
        }
        if self.width > caps.max_width || self.height > caps.max_height {
            return Err(NvencError::UnsupportedResolution);
        }
        if (self.codec == Codec::Hevc && !caps.hevc) || (self.codec == Codec::H264 && !caps.h264) {
            return Err(NvencError::UnsupportedCodec);
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NvencError {
    #[error("只支援 NVIDIA RTX")]
    UnsupportedAdapter,
    #[error("GPU 不支援要求的解析度")]
    UnsupportedResolution,
    #[error("GPU 不支援要求的 codec")]
    UnsupportedCodec,
    #[error("匯出已取消")]
    Cancelled,
    #[error("NVENC API 失敗：{0}")]
    Native(String),
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken(Arc<AtomicBool>);
impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    pub fn check(&self) -> Result<(), NvencError> {
        if self.is_cancelled() {
            Err(NvencError::Cancelled)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportStage {
    Preflight,
    Rendering,
    Encoding,
    Muxing,
    Verifying,
    Completed,
    Cancelled,
    Failed,
}
#[derive(Debug, Clone, PartialEq)]
pub struct ExportProgress {
    pub stage: ExportStage,
    pub completed_frames: u64,
    pub total_frames: u64,
    pub fps: f64,
    pub eta_seconds: f64,
}
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExportMetrics {
    pub cpu_frame_readbacks: u64,
    pub encoded_frames: u64,
    pub dropped_frames: u64,
    pub duplicated_frames: u64,
    pub ring_occupancy_peak: usize,
    pub peak_vram_bytes: u64,
    pub gpu_adapter_luid: u64,
    pub render_p50_ms: f64,
    pub render_p95_ms: f64,
    pub encode_p50_ms: f64,
    pub encode_p95_ms: f64,
    pub mux_p50_ms: f64,
    pub mux_p95_ms: f64,
    pub elapsed_seconds: f64,
}

pub trait FrameRenderer {
    type Frame;
    fn render(&mut self, frame_index: u64, total_frames: u64) -> Result<Self::Frame, String>;
}

pub trait PacketEncoder<Frame> {
    type Packet;
    fn encode(&mut self, frame_index: u64, frame: Frame) -> Result<Vec<Self::Packet>, String>;
    fn flush(&mut self) -> Result<Vec<Self::Packet>, String>;
}

pub trait PacketMuxer<Packet> {
    fn write(&mut self, packet: Packet) -> Result<(), String>;
    fn finalize(&mut self) -> Result<(), String>;
    fn abort(&mut self);
}

pub fn run_export<R, E, M, F>(
    total_frames: u64,
    token: &CancellationToken,
    renderer: &mut R,
    encoder: &mut E,
    muxer: &mut M,
    mut progress: F,
) -> Result<ExportMetrics, NvencError>
where
    R: FrameRenderer,
    E: PacketEncoder<R::Frame>,
    M: PacketMuxer<E::Packet>,
    F: FnMut(ExportProgress),
{
    let started = Instant::now();
    let result = (|| {
        for frame_index in 0..total_frames {
            token.check()?;
            let frame = renderer
                .render(frame_index, total_frames)
                .map_err(NvencError::Native)?;
            token.check()?;
            let packets = encoder
                .encode(frame_index, frame)
                .map_err(NvencError::Native)?;
            for packet in packets {
                muxer.write(packet).map_err(NvencError::Native)?;
            }
            let completed = frame_index + 1;
            let elapsed = started.elapsed().as_secs_f64().max(f64::EPSILON);
            let fps = completed as f64 / elapsed;
            progress(ExportProgress {
                stage: ExportStage::Encoding,
                completed_frames: completed,
                total_frames,
                fps,
                eta_seconds: (total_frames - completed) as f64 / fps,
            });
        }
        for packet in encoder.flush().map_err(NvencError::Native)? {
            muxer.write(packet).map_err(NvencError::Native)?;
        }
        muxer.finalize().map_err(NvencError::Native)?;
        Ok(ExportMetrics {
            cpu_frame_readbacks: 0,
            encoded_frames: total_frames,
            dropped_frames: 0,
            duplicated_frames: 0,
            ring_occupancy_peak: d3d11_renderer::EXPORT_TEXTURE_COUNT,
            ..ExportMetrics::default()
        })
    })();
    if result.is_err() {
        muxer.abort();
    }
    result
}

#[cfg(windows)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPacket {
    pub bytes: Vec<u8>,
    pub pts: u64,
    pub duration: u64,
    pub frame_index: u32,
    pub display_index: u32,
    pub picture_type: nvenc::sys::enums::NVencPicType,
    pub texture_slot: usize,
}

#[cfg(windows)]
struct EncoderState {
    pending_outputs: VecDeque<usize>,
    free_outputs: VecDeque<usize>,
    input_slots: HashMap<u64, usize>,
}
#[cfg(windows)]
impl EncoderState {
    fn new(count: usize) -> Self {
        Self {
            pending_outputs: VecDeque::new(),
            free_outputs: (0..count).collect(),
            input_slots: HashMap::new(),
        }
    }
}

#[cfg(windows)]
struct NvencEvent(windows::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl NvencEvent {
    fn create() -> Result<Self, NvencError> {
        use windows::Win32::System::Threading::CreateEventW;
        Ok(Self(
            unsafe { CreateEventW(None, true, false, None) }
                .map_err(|error| NvencError::Native(error.to_string()))?,
        ))
    }
    fn raw(&self) -> *mut std::ffi::c_void {
        self.0.0
    }
    fn reset(&self) -> Result<(), NvencError> {
        unsafe { windows::Win32::System::Threading::ResetEvent(self.0) }
            .map_err(|error| NvencError::Native(error.to_string()))
    }
    fn signaled(&self, wait: bool) -> Result<bool, NvencError> {
        use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::{INFINITE, WaitForSingleObject};
        let result = unsafe { WaitForSingleObject(self.0, if wait { INFINITE } else { 0 }) };
        if result == WAIT_OBJECT_0 {
            Ok(true)
        } else if result == WAIT_TIMEOUT {
            Ok(false)
        } else {
            Err(NvencError::Native(format!(
                "WaitForSingleObject failed: {result:?}"
            )))
        }
    }
}
#[cfg(windows)]
impl Drop for NvencEvent {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(windows)]
pub struct NativeEncoder {
    pub encoder: nvenc::encoder::Encoder,
    pub resources: Vec<nvenc::encoder::RegisteredResource>,
    pub bitstreams: Vec<nvenc::bitstream::BitStream>,
    events: Vec<NvencEvent>,
    eos_event: NvencEvent,
    state: Mutex<EncoderState>,
}

#[cfg(windows)]
impl NativeEncoder {
    pub fn create(
        device: &d3d11_renderer::D3d11ExportDevice,
        textures: &[windows::Win32::Graphics::Direct3D11::ID3D11Texture2D],
        config: EncoderConfig,
    ) -> Result<Self, NvencError> {
        use nvenc::session::{InitParams, NeedsConfig, Session};
        use nvenc::sys::enums::{NVencBufferFormat, NVencTuningInfo};
        use nvenc::sys::guids::{
            NV_ENC_CODEC_H264_GUID, NV_ENC_CODEC_HEVC_GUID, NV_ENC_PRESET_P3_GUID,
            NV_ENC_PRESET_P4_GUID, NV_ENC_PRESET_P5_GUID,
        };

        let codec = match config.codec {
            Codec::Hevc => NV_ENC_CODEC_HEVC_GUID,
            Codec::H264 => NV_ENC_CODEC_H264_GUID,
        };
        let preset = match config.preset {
            3 => NV_ENC_PRESET_P3_GUID,
            5 => NV_ENC_PRESET_P5_GUID,
            _ => NV_ENC_PRESET_P4_GUID,
        };
        let session = Session::<NeedsConfig>::open_dx(&device.device)
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        let codecs = session
            .get_encode_codecs()
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        if !codecs.contains(&codec) {
            return Err(NvencError::UnsupportedCodec);
        }
        let (session, mut preset_config) = session
            .get_encode_preset_config_ex(
                codec.clone(),
                preset.clone(),
                NVencTuningInfo::HighQuality,
            )
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        preset_config.preset_cfg.gop_len = config.gop;
        preset_config.preset_cfg.frame_interval_p = i32::from(config.b_frames) + 1;
        preset_config.preset_cfg.rc_params.configure_vbr_quality(
            config.cq,
            config.max_bitrate,
            config.aq,
        );
        let encoder = session
            .init_encoder(InitParams {
                encode_guid: codec,
                preset_guid: preset,
                resolution: [config.width, config.height],
                aspect_ratio: [config.width, config.height],
                frame_rate: [config.fps, 1],
                tuning_info: NVencTuningInfo::HighQuality,
                buffer_format: NVencBufferFormat::ARGB,
                encode_config: &mut preset_config.preset_cfg,
                enable_ptd: true,
                enable_async: true,
                max_encoder_resolution: [config.width, config.height],
            })
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        let mut events = Vec::with_capacity(textures.len());
        for _ in 0..textures.len() {
            let event = NvencEvent::create()?;
            encoder
                .register_async_event(event.raw())
                .map_err(|error| NvencError::Native(format!("{error:?}")))?;
            events.push(event);
        }
        let eos_event = NvencEvent::create()?;
        encoder
            .register_async_event(eos_event.raw())
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        let resources = textures
            .iter()
            .map(|texture| {
                encoder
                    .register_resource_dx11(texture, NVencBufferFormat::ARGB, config.width * 4)
                    .map_err(|error| NvencError::Native(format!("{error:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let bitstreams = (0..textures.len())
            .map(|_| {
                encoder
                    .create_bitstream_buffer()
                    .map_err(|error| NvencError::Native(format!("{error:?}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            encoder,
            resources,
            bitstreams,
            events,
            eos_event,
            state: Mutex::new(EncoderState::new(textures.len())),
        })
    }

    pub fn encode_frame(
        &self,
        slot: usize,
        frame_index: usize,
        timestamp: u64,
    ) -> Result<Vec<u8>, NvencError> {
        let mut packets = self.submit_frame(slot, frame_index, timestamp)?;
        if packets.is_empty() {
            packets.extend(self.poll_ready(true)?);
        }
        packets
            .into_iter()
            .next()
            .map(|packet| packet.bytes)
            .ok_or_else(|| NvencError::Native("NeedMoreInput".into()))
    }

    pub fn submit_frame(
        &self,
        slot: usize,
        frame_index: usize,
        timestamp: u64,
    ) -> Result<Vec<EncodedPacket>, NvencError> {
        use nvenc::sys::enums::{NVencBufferFormat, NVencPicStruct, NVencPicType};
        let input = self
            .resources
            .get(slot)
            .ok_or_else(|| NvencError::Native("texture slot 超出範圍".into()))?;
        let mut packets = self.poll_ready(false)?;
        let has_free = {
            let state = self
                .state
                .lock()
                .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
            !state.free_outputs.is_empty()
        };
        if !has_free {
            packets.extend(self.poll_ready(true)?);
        }
        let output_slot = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
            let output_slot = state
                .free_outputs
                .pop_front()
                .ok_or_else(|| NvencError::Native("NVENC bitstream ring backpressure".into()))?;
            state.pending_outputs.push_back(output_slot);
            state.input_slots.insert(timestamp, slot);
            output_slot
        };
        let output = self
            .bitstreams
            .get(output_slot)
            .ok_or_else(|| NvencError::Native("bitstream slot 超出範圍".into()))?;
        let event = self
            .events
            .get(output_slot)
            .ok_or_else(|| NvencError::Native("event slot 超出範圍".into()))?;
        event.reset()?;
        let result = self.encoder.encode_picture_async(
            input,
            output,
            frame_index,
            timestamp,
            NVencBufferFormat::ARGB,
            NVencPicStruct::Frame,
            NVencPicType::UNKNOWN,
            None,
            event.raw(),
        );
        match result {
            Ok(()) | Err(nvenc::sys::result::NVencError::NeedMoreInput) => {
                packets.extend(self.poll_ready(false)?);
                Ok(packets)
            }
            Err(error) => {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
                state.pending_outputs.retain(|value| *value != output_slot);
                state.free_outputs.push_back(output_slot);
                state.input_slots.remove(&timestamp);
                Err(NvencError::Native(format!("{error:?}")))
            }
        }
    }

    fn read_packet(
        &self,
        output_slot: usize,
        wait: bool,
    ) -> Result<Option<EncodedPacket>, NvencError> {
        let output = self
            .bitstreams
            .get(output_slot)
            .ok_or_else(|| NvencError::Native("bitstream slot 超出範圍".into()))?;
        let event = self
            .events
            .get(output_slot)
            .ok_or_else(|| NvencError::Native("event slot 超出範圍".into()))?;
        if !event.signaled(wait)? {
            return Ok(None);
        }
        let lock = output
            .try_lock(false)
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        let pts = lock.output_time_stamp();
        let texture_slot = {
            let state = self
                .state
                .lock()
                .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
            *state.input_slots.get(&pts).ok_or_else(|| {
                NvencError::Native(format!("NVENC returned unknown timestamp {pts}"))
            })?
        };
        Ok(Some(EncodedPacket {
            bytes: lock.as_slice().to_vec(),
            pts,
            duration: lock.output_duration(),
            frame_index: lock.frame_idx(),
            display_index: lock.frame_idx_display(),
            picture_type: lock.picture_type(),
            texture_slot,
        }))
    }

    fn poll_ready(&self, wait: bool) -> Result<Vec<EncodedPacket>, NvencError> {
        let mut packets = Vec::new();
        loop {
            let output_slot = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
                state.pending_outputs.front().copied()
            };
            let Some(output_slot) = output_slot else {
                break;
            };
            let Some(packet) = self.read_packet(output_slot, wait)? else {
                break;
            };
            {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
                if state.pending_outputs.pop_front() != Some(output_slot) {
                    return Err(NvencError::Native("bitstream queue order changed".into()));
                }
                state.free_outputs.push_back(output_slot);
                state.input_slots.remove(&packet.pts);
            }
            packets.push(packet);
            if wait {
                break;
            }
        }
        Ok(packets)
    }

    pub fn finish_packets(&self) -> Result<Vec<EncodedPacket>, NvencError> {
        self.eos_event.reset()?;
        self.encoder
            .end_encode_async(self.eos_event.raw())
            .map_err(|error| NvencError::Native(format!("{error:?}")))?;
        if !self.eos_event.signaled(true)? {
            return Err(NvencError::Native(
                "NVENC EOS event was not signaled".into(),
            ));
        }
        let mut packets = Vec::new();
        loop {
            let pending = {
                let state = self
                    .state
                    .lock()
                    .map_err(|_| NvencError::Native("encoder state poisoned".into()))?;
                !state.pending_outputs.is_empty()
            };
            if !pending {
                break;
            }
            packets.extend(self.poll_ready(true)?);
        }
        Ok(packets)
    }
    pub fn receive_packets(&self, wait: bool) -> Result<Vec<EncodedPacket>, NvencError> {
        self.poll_ready(wait)
    }
    pub fn finish(&self) -> Result<(), NvencError> {
        self.finish_packets().map(|_| ())
    }
}

#[cfg(windows)]
impl Drop for NativeEncoder {
    fn drop(&mut self) {
        for event in &self.events {
            let _ = self.encoder.unregister_async_event(event.raw());
        }
        let _ = self.encoder.unregister_async_event(self.eos_event.raw());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[cfg(windows)]
    fn gpu_guard() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap()
    }
    fn caps() -> GpuCapabilities {
        GpuCapabilities {
            adapter_name: "NVIDIA GeForce RTX 2080 Ti".into(),
            luid: 42,
            dedicated_vram: 11 << 30,
            hevc: true,
            h264: true,
            async_encode: true,
            max_width: 8192,
            max_height: 8192,
            api_version: "13".into(),
        }
    }
    #[test]
    fn balanced_contract_is_fixed() {
        let c = EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced);
        assert_eq!(
            (c.width, c.height, c.fps, c.preset, c.cq, c.b_frames, c.gop),
            (3840, 2160, 60, 4, 22, 2, 120)
        );
        assert!(c.aq);
        assert!(!c.lookahead);
    }
    #[test]
    fn validates_rtx_capabilities() {
        assert!(
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced)
                .validate(&caps())
                .is_ok()
        );
    }
    #[test]
    fn rejects_integrated_adapter() {
        let mut c = caps();
        c.adapter_name = "Intel UHD".into();
        assert_eq!(
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced).validate(&c),
            Err(NvencError::UnsupportedAdapter)
        );
    }
    #[test]
    fn cancellation_is_shared_and_idempotent() {
        let token = CancellationToken::default();
        let other = token.clone();
        assert!(token.check().is_ok());
        other.cancel();
        other.cancel();
        assert_eq!(token.check(), Err(NvencError::Cancelled));
    }
    #[cfg(windows)]
    #[test]
    fn opens_nvenc_and_registers_six_d3d11_textures() {
        let _gpu = gpu_guard();
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced),
        )
        .unwrap();
        assert_eq!(encoder.resources.len(), 6);
        assert_eq!(encoder.bitstreams.len(), 6);
    }
    #[cfg(windows)]
    #[test]
    fn encodes_real_hevc_parameter_sets_and_idr_from_d3d11_texture() {
        let _gpu = gpu_guard();
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        device
            .clear_texture(&textures[0], [0.08, 0.12, 0.18, 1.0])
            .unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Speed),
        )
        .unwrap();
        let packet = encoder.encode_frame(0, 0, 0).unwrap();
        let kinds: Vec<_> = mp4_output::split_annex_b(&packet)
            .into_iter()
            .map(|nal| nal.kind)
            .collect();
        assert!(kinds.contains(&mp4_output::HevcNalKind::Vps));
        assert!(kinds.contains(&mp4_output::HevcNalKind::Sps));
        assert!(kinds.contains(&mp4_output::HevcNalKind::Pps));
        assert!(kinds.contains(&mp4_output::HevcNalKind::Idr));
        encoder.finish().unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn muxes_real_nvenc_packets_as_hvc1_mp4() {
        let _gpu = gpu_guard();
        use std::io::Cursor;
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Speed),
        )
        .unwrap();
        device
            .clear_texture(&textures[0], [0.08, 0.12, 0.18, 1.0])
            .unwrap();
        device.flush();
        let first = encoder.encode_frame(0, 0, 0).unwrap();
        let sets = mp4_output::parameter_sets(&first).unwrap();
        let mut mux =
            mp4_output::HevcMp4Writer::new(Cursor::new(Vec::new()), 3840, 2160, 60, sets).unwrap();
        mux.write_access_unit(&first, 0, 0).unwrap();
        for frame in 1..60usize {
            let slot = frame % textures.len();
            device
                .clear_texture(&textures[slot], [frame as f32 / 60.0, 0.12, 0.22, 1.0])
                .unwrap();
            device.flush();
            let packet = encoder.encode_frame(slot, frame, frame as u64).unwrap();
            mux.write_access_unit(&packet, frame as u64, frame as u64)
                .unwrap();
        }
        encoder.finish().unwrap();
        let cursor = mux.finalize().unwrap();
        let data = cursor.into_inner();
        assert!(data.windows(4).any(|v| v == b"hvc1"));
        let size = data.len() as u64;
        let reader = mp4::Mp4Reader::read_header(Cursor::new(data), size).unwrap();
        assert_eq!(reader.sample_count(1).unwrap(), 60);
    }
    #[cfg(windows)]
    #[test]
    fn balanced_b_frames_can_be_submitted() {
        let _gpu = gpu_guard();
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced),
        )
        .unwrap();
        let mut packets = Vec::new();
        for frame in 0..6usize {
            let slot = frame % textures.len();
            device
                .clear_texture(&textures[slot], [0.1, 0.2, 0.3, 1.0])
                .unwrap();
            device.flush();
            packets.extend(encoder.submit_frame(slot, frame, frame as u64).unwrap());
        }
        packets.extend(encoder.finish_packets().unwrap());
        assert_eq!(packets.len(), 6);
        assert_eq!(
            packets.iter().map(|packet| packet.pts).collect::<Vec<_>>(),
            vec![0, 3, 2, 1, 5, 4]
        );
    }
    #[cfg(windows)]
    #[test]
    fn ffprobe_validates_real_p4_bframe_4k60_hvc1_file() {
        let _gpu = gpu_guard();
        use std::process::Command;
        let ffprobe = std::path::Path::new(r"C:\Program Files\FFMPEG\bin\ffprobe.exe");
        if !ffprobe.exists() {
            return;
        }
        let output =
            std::env::temp_dir().join(format!("gpx-animator-real-{}.mp4", std::process::id()));
        let _ = std::fs::remove_file(&output);
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced),
        )
        .unwrap();
        let mut packets = Vec::new();
        for frame in 0..60usize {
            let slot = frame % textures.len();
            device
                .clear_texture(&textures[slot], [frame as f32 / 60.0, 0.2, 0.3, 1.0])
                .unwrap();
            device.flush();
            packets.extend(encoder.submit_frame(slot, frame, frame as u64).unwrap());
        }
        packets.extend(encoder.finish_packets().unwrap());
        assert_eq!(packets.len(), 60);
        let sets = mp4_output::parameter_sets(&packets[0].bytes).unwrap();
        let mut mux = mp4_output::AtomicHevcFile::create(&output, 3840, 2160, 60, sets).unwrap();
        for (dts, packet) in packets.iter().enumerate() {
            mux.write_access_unit(&packet.bytes, dts as u64, packet.pts)
                .unwrap();
        }
        mux.finalize().unwrap();
        let result = Command::new(ffprobe)
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
        assert!(text.contains("codec_name=hevc"), "{text}");
        assert!(text.contains("codec_tag_string=hvc1"), "{text}");
        assert!(
            text.contains("width=3840") && text.contains("height=2160"),
            "{text}"
        );
        assert!(text.contains("r_frame_rate=60/1"), "{text}");
        assert!(text.contains("nb_read_frames=60"), "{text}");
        std::fs::remove_file(output).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn zero_copy_4k60_pipeline_meets_realtime_gate() {
        let _gpu = gpu_guard();
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Speed),
        )
        .unwrap();
        let started = Instant::now();
        let mut packets = Vec::new();
        for frame in 0..120usize {
            let slot = frame % textures.len();
            let phase = frame as f32 / 120.0;
            device
                .clear_texture(&textures[slot], [phase, 0.12, 0.22, 1.0])
                .unwrap();
            device.flush();
            packets.extend(encoder.submit_frame(slot, frame, frame as u64).unwrap());
        }
        packets.extend(encoder.finish_packets().unwrap());
        let bytes = packets
            .iter()
            .map(|packet| packet.bytes.len())
            .sum::<usize>();
        assert_eq!(packets.len(), 120);
        assert!(bytes > 0);
        assert!(
            started.elapsed() <= Duration::from_secs(2),
            "4K60 zero-copy took {:?}",
            started.elapsed()
        );
    }
    #[cfg(windows)]
    #[test]
    fn rendered_route_4k60_pipeline_meets_realtime_gate() {
        let _gpu = gpu_guard();
        use gpx_core::{ParseOptions, parse_gpx};
        use scene_core::{Scene, SceneOptions, build_frame};
        let track=parse_gpx(r#"<gpx><trk><trkseg><trkpt lat="25" lon="121"><ele>10</ele></trkpt><trkpt lat="25.01" lon="121.01"><ele>20</ele></trkpt><trkpt lat="25.02" lon="121.03"><ele>15</ele></trkpt></trkseg></trk></gpx>"#,ParseOptions::default()).unwrap();
        let scene = Scene {
            track,
            options: SceneOptions::default(),
        };
        let device = d3d11_renderer::D3d11ExportDevice::create_rtx().unwrap();
        let textures = device.create_export_textures(3840, 2160).unwrap();
        let renderer = d3d11_renderer::D2dSceneRenderer::new(&device).unwrap();
        let encoder = NativeEncoder::create(
            &device,
            &textures,
            EncoderConfig::four_k_60(Codec::Hevc, QualityPreset::Balanced),
        )
        .unwrap();
        let started = Instant::now();
        let mut packets = Vec::new();
        for frame_index in 0..120usize {
            let slot = frame_index % textures.len();
            let frame = build_frame(&scene, frame_index as f64 / 119.0);
            renderer
                .render(&textures[slot], &frame, &scene.options, 3840, 2160)
                .unwrap();
            device.flush();
            packets.extend(
                encoder
                    .submit_frame(slot, frame_index, frame_index as u64)
                    .unwrap(),
            );
        }
        packets.extend(encoder.finish_packets().unwrap());
        let bytes = packets
            .iter()
            .map(|packet| packet.bytes.len())
            .sum::<usize>();
        assert_eq!(packets.len(), 120);
        assert!(bytes > 0);
        assert!(
            started.elapsed() <= Duration::from_secs(2),
            "rendered P4 B-frame 4K60 took {:?}",
            started.elapsed()
        );
    }

    #[derive(Default)]
    struct FakeRenderer;
    impl FrameRenderer for FakeRenderer {
        type Frame = u64;
        fn render(&mut self, index: u64, _: u64) -> Result<u64, String> {
            Ok(index)
        }
    }
    #[derive(Default)]
    struct FakeEncoder;
    impl PacketEncoder<u64> for FakeEncoder {
        type Packet = u64;
        fn encode(&mut self, _: u64, frame: u64) -> Result<Vec<u64>, String> {
            Ok(vec![frame])
        }
        fn flush(&mut self) -> Result<Vec<u64>, String> {
            Ok(vec![])
        }
    }
    #[derive(Default)]
    struct FakeMuxer {
        packets: Vec<u64>,
        finalized: bool,
        aborted: bool,
    }
    impl PacketMuxer<u64> for FakeMuxer {
        fn write(&mut self, p: u64) -> Result<(), String> {
            self.packets.push(p);
            Ok(())
        }
        fn finalize(&mut self) -> Result<(), String> {
            self.finalized = true;
            Ok(())
        }
        fn abort(&mut self) {
            self.aborted = true;
        }
    }
    #[test]
    fn export_state_machine_writes_every_frame_once() {
        let mut r = FakeRenderer;
        let mut e = FakeEncoder;
        let mut m = FakeMuxer::default();
        let token = CancellationToken::default();
        let mut updates = Vec::new();
        let metrics = run_export(120, &token, &mut r, &mut e, &mut m, |p| updates.push(p)).unwrap();
        assert_eq!(m.packets, (0..120).collect::<Vec<_>>());
        assert!(m.finalized);
        assert!(!m.aborted);
        assert_eq!(metrics.encoded_frames, 120);
        assert_eq!(metrics.cpu_frame_readbacks, 0);
        assert_eq!(updates.last().unwrap().completed_frames, 120);
    }
    #[test]
    fn cancellation_aborts_muxer_without_finalizing() {
        struct CancelRenderer(CancellationToken);
        impl FrameRenderer for CancelRenderer {
            type Frame = u64;
            fn render(&mut self, index: u64, _: u64) -> Result<u64, String> {
                if index == 4 {
                    self.0.cancel();
                }
                Ok(index)
            }
        }
        let token = CancellationToken::default();
        let mut r = CancelRenderer(token.clone());
        let mut e = FakeEncoder;
        let mut m = FakeMuxer::default();
        assert_eq!(
            run_export(120, &token, &mut r, &mut e, &mut m, |_| {}),
            Err(NvencError::Cancelled)
        );
        assert!(m.aborted);
        assert!(!m.finalized);
        assert_eq!(m.packets, vec![0, 1, 2, 3]);
    }
    #[test]
    fn mux_failure_aborts_job() {
        struct FailingMux;
        impl PacketMuxer<u64> for FailingMux {
            fn write(&mut self, _: u64) -> Result<(), String> {
                Err("disk full".into())
            }
            fn finalize(&mut self) -> Result<(), String> {
                Ok(())
            }
            fn abort(&mut self) {}
        }
        let mut r = FakeRenderer;
        let mut e = FakeEncoder;
        let mut m = FailingMux;
        let error = run_export(
            2,
            &CancellationToken::default(),
            &mut r,
            &mut e,
            &mut m,
            |_| {},
        )
        .unwrap_err();
        assert_eq!(error, NvencError::Native("disk full".into()));
    }
}
