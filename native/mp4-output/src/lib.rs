use mp4::{
    AvcConfig, Bytes, FourCC, HevcConfig, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer,
    TrackConfig, TrackType,
};
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HevcNalKind {
    Vps,
    Sps,
    Pps,
    Idr,
    Slice,
    Other(u8),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NalUnit<'a> {
    pub kind: HevcNalKind,
    pub bytes: &'a [u8],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HevcParameterSets {
    pub vps: Vec<u8>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcParameterSets {
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

fn raw_annex_b(input: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= input.len() {
        let length = if input[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if input[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        starts.push((index, length));
        index += length;
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, (start, prefix))| {
            let from = start + prefix;
            let to = starts
                .get(position + 1)
                .map(|next| next.0)
                .unwrap_or(input.len());
            (from < to).then_some(&input[from..to])
        })
        .collect()
}

fn kind(bytes: &[u8]) -> HevcNalKind {
    let value = bytes.first().map(|byte| (byte >> 1) & 0x3f).unwrap_or(63);
    match value {
        32 => HevcNalKind::Vps,
        33 => HevcNalKind::Sps,
        34 => HevcNalKind::Pps,
        19 | 20 => HevcNalKind::Idr,
        0..=31 => HevcNalKind::Slice,
        other => HevcNalKind::Other(other),
    }
}

pub fn split_annex_b(input: &[u8]) -> Vec<NalUnit<'_>> {
    let mut starts = Vec::new();
    let mut index = 0;
    while index + 3 <= input.len() {
        let length = if input[index..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if input[index..].starts_with(&[0, 0, 1]) {
            3
        } else {
            index += 1;
            continue;
        };
        starts.push((index, length));
        index += length;
    }
    starts
        .iter()
        .enumerate()
        .filter_map(|(position, (start, prefix))| {
            let from = start + prefix;
            let to = starts
                .get(position + 1)
                .map(|next| next.0)
                .unwrap_or(input.len());
            (from < to).then(|| NalUnit {
                kind: kind(&input[from..to]),
                bytes: &input[from..to],
            })
        })
        .collect()
}

pub fn parameter_sets(input: &[u8]) -> Result<HevcParameterSets, Mp4Error> {
    let units = split_annex_b(input);
    let find = |wanted| {
        units
            .iter()
            .find(|unit| unit.kind == wanted)
            .map(|unit| unit.bytes.to_vec())
    };
    Ok(HevcParameterSets {
        vps: find(HevcNalKind::Vps).ok_or(Mp4Error::MissingParameterSet("VPS"))?,
        sps: find(HevcNalKind::Sps).ok_or(Mp4Error::MissingParameterSet("SPS"))?,
        pps: find(HevcNalKind::Pps).ok_or(Mp4Error::MissingParameterSet("PPS"))?,
    })
}

pub fn avc_parameter_sets(input: &[u8]) -> Result<AvcParameterSets, Mp4Error> {
    let units = raw_annex_b(input);
    let find = |wanted: u8| {
        units
            .iter()
            .find(|unit| unit.first().map(|byte| byte & 0x1f) == Some(wanted))
            .map(|unit| unit.to_vec())
    };
    Ok(AvcParameterSets {
        sps: find(7).ok_or(Mp4Error::MissingParameterSet("SPS"))?,
        pps: find(8).ok_or(Mp4Error::MissingParameterSet("PPS"))?,
    })
}

fn convert_avc_access_unit(
    input: &[u8],
    include_parameter_sets: bool,
) -> Result<(Vec<u8>, bool), Mp4Error> {
    let units = raw_annex_b(input);
    if units.is_empty() {
        return Err(Mp4Error::MissingStartCode);
    }
    let mut output = Vec::with_capacity(input.len());
    let mut sync = false;
    for unit in units {
        let kind = unit[0] & 0x1f;
        if kind == 5 {
            sync = true
        }
        if !include_parameter_sets && matches!(kind, 7 | 8) {
            continue;
        }
        let length = u32::try_from(unit.len()).map_err(|_| Mp4Error::NalTooLarge)?;
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(unit);
    }
    Ok((output, sync))
}

pub fn annex_b_to_length_prefixed(input: &[u8]) -> Result<Vec<u8>, Mp4Error> {
    convert_access_unit(input, true)
}

fn convert_access_unit(input: &[u8], include_parameter_sets: bool) -> Result<Vec<u8>, Mp4Error> {
    let units = split_annex_b(input);
    if units.is_empty() {
        return Err(Mp4Error::MissingStartCode);
    }
    let mut output = Vec::with_capacity(input.len());
    for unit in units {
        if !include_parameter_sets
            && matches!(
                unit.kind,
                HevcNalKind::Vps | HevcNalKind::Sps | HevcNalKind::Pps
            )
        {
            continue;
        }
        let length = u32::try_from(unit.bytes.len()).map_err(|_| Mp4Error::NalTooLarge)?;
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(unit.bytes);
    }
    Ok(output)
}

pub struct HevcMp4Writer<W: Write + Seek> {
    writer: Mp4Writer<W>,
    fps: u32,
    frame_duration: u32,
    frames: u64,
}

pub struct AvcMp4Writer<W: Write + Seek> {
    writer: Mp4Writer<W>,
    frame_duration: u32,
    frames: u64,
}
impl<W: Write + Seek> AvcMp4Writer<W> {
    pub fn new(
        output: W,
        width: u32,
        height: u32,
        fps: u32,
        sets: AvcParameterSets,
    ) -> Result<Self, Mp4Error> {
        if fps == 0 {
            return Err(Mp4Error::InvalidFps);
        }
        let width = u16::try_from(width).map_err(|_| Mp4Error::InvalidDimensions)?;
        let height = u16::try_from(height).map_err(|_| Mp4Error::InvalidDimensions)?;
        let timescale = fps.checked_mul(1000).ok_or(Mp4Error::InvalidFps)?;
        let config = Mp4Config {
            major_brand: FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![
                FourCC::from(*b"isom"),
                FourCC::from(*b"iso6"),
                FourCC::from(*b"avc1"),
                FourCC::from(*b"mp41"),
            ],
            timescale,
        };
        let mut writer = Mp4Writer::write_start(output, &config)?;
        writer.add_track(&TrackConfig {
            track_type: TrackType::Video,
            timescale,
            language: "und".into(),
            media_conf: MediaConfig::AvcConfig(AvcConfig {
                width,
                height,
                seq_param_set: sets.sps,
                pic_param_set: sets.pps,
            }),
        })?;
        Ok(Self {
            writer,
            frame_duration: 1000,
            frames: 0,
        })
    }
    pub fn write_access_unit(
        &mut self,
        annex_b: &[u8],
        dts_frame: u64,
        pts_frame: u64,
    ) -> Result<(), Mp4Error> {
        let (bytes, is_sync) = convert_avc_access_unit(annex_b, false)?;
        let offset = i64::try_from(pts_frame).map_err(|_| Mp4Error::TimestampOverflow)?
            - i64::try_from(dts_frame).map_err(|_| Mp4Error::TimestampOverflow)?;
        let rendering_offset = i32::try_from(
            offset
                .checked_mul(self.frame_duration as i64)
                .ok_or(Mp4Error::TimestampOverflow)?,
        )
        .map_err(|_| Mp4Error::TimestampOverflow)?;
        self.writer.write_sample(
            1,
            &Mp4Sample {
                start_time: dts_frame
                    .checked_mul(self.frame_duration as u64)
                    .ok_or(Mp4Error::TimestampOverflow)?,
                duration: self.frame_duration,
                rendering_offset,
                is_sync,
                bytes: Bytes::from(bytes),
            },
        )?;
        self.frames += 1;
        Ok(())
    }
    pub fn finalize(mut self) -> Result<W, Mp4Error> {
        self.writer.write_end()?;
        Ok(self.writer.into_writer())
    }
}

impl<W: Write + Seek> HevcMp4Writer<W> {
    pub fn new(
        output: W,
        width: u32,
        height: u32,
        fps: u32,
        sets: HevcParameterSets,
    ) -> Result<Self, Mp4Error> {
        if fps == 0 {
            return Err(Mp4Error::InvalidFps);
        }
        let width = u16::try_from(width).map_err(|_| Mp4Error::InvalidDimensions)?;
        let height = u16::try_from(height).map_err(|_| Mp4Error::InvalidDimensions)?;
        let timescale = fps.checked_mul(1000).ok_or(Mp4Error::InvalidFps)?;
        let config = Mp4Config {
            major_brand: FourCC::from(*b"isom"),
            minor_version: 512,
            compatible_brands: vec![
                FourCC::from(*b"isom"),
                FourCC::from(*b"iso6"),
                FourCC::from(*b"hvc1"),
                FourCC::from(*b"mp41"),
            ],
            timescale,
        };
        let mut writer = Mp4Writer::write_start(output, &config)?;
        writer.add_track(&TrackConfig {
            track_type: TrackType::Video,
            timescale,
            language: "und".into(),
            media_conf: MediaConfig::HevcConfig(HevcConfig {
                width,
                height,
                vps: sets.vps,
                sps: sets.sps,
                pps: sets.pps,
            }),
        })?;
        Ok(Self {
            writer,
            fps,
            frame_duration: 1000,
            frames: 0,
        })
    }

    pub fn write_access_unit(
        &mut self,
        annex_b: &[u8],
        dts_frame: u64,
        pts_frame: u64,
    ) -> Result<(), Mp4Error> {
        let units = split_annex_b(annex_b);
        let is_sync = units.iter().any(|unit| unit.kind == HevcNalKind::Idr);
        let bytes = convert_access_unit(annex_b, false)?;
        let offset_frames = i64::try_from(pts_frame).map_err(|_| Mp4Error::TimestampOverflow)?
            - i64::try_from(dts_frame).map_err(|_| Mp4Error::TimestampOverflow)?;
        let rendering_offset = i32::try_from(
            offset_frames
                .checked_mul(self.frame_duration as i64)
                .ok_or(Mp4Error::TimestampOverflow)?,
        )
        .map_err(|_| Mp4Error::TimestampOverflow)?;
        self.writer.write_sample(
            1,
            &Mp4Sample {
                start_time: dts_frame
                    .checked_mul(self.frame_duration as u64)
                    .ok_or(Mp4Error::TimestampOverflow)?,
                duration: self.frame_duration,
                rendering_offset,
                is_sync,
                bytes: Bytes::from(bytes),
            },
        )?;
        self.frames += 1;
        Ok(())
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }
    pub fn fps(&self) -> u32 {
        self.fps
    }
    pub fn finalize(mut self) -> Result<W, Mp4Error> {
        self.writer.write_end()?;
        Ok(self.writer.into_writer())
    }
}

pub struct AtomicHevcFile {
    muxer: Option<HevcMp4Writer<BufWriter<File>>>,
    final_path: PathBuf,
    partial_path: PathBuf,
}

impl AtomicHevcFile {
    pub fn create(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        fps: u32,
        sets: HevcParameterSets,
    ) -> Result<Self, Mp4Error> {
        let final_path = path.as_ref().to_path_buf();
        let partial_path = final_path.with_extension(format!(
            "{}.part",
            final_path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("mp4")
        ));
        if let Some(parent) = partial_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(&partial_path)?;
        let muxer = HevcMp4Writer::new(BufWriter::new(file), width, height, fps, sets)?;
        Ok(Self {
            muxer: Some(muxer),
            final_path,
            partial_path,
        })
    }
    pub fn write_access_unit(
        &mut self,
        annex_b: &[u8],
        dts_frame: u64,
        pts_frame: u64,
    ) -> Result<(), Mp4Error> {
        self.muxer
            .as_mut()
            .ok_or(Mp4Error::AlreadyFinalized)?
            .write_access_unit(annex_b, dts_frame, pts_frame)
    }
    pub fn finalize(mut self) -> Result<PathBuf, Mp4Error> {
        let muxer = self.muxer.take().ok_or(Mp4Error::AlreadyFinalized)?;
        let mut output = muxer.finalize()?;
        output.flush()?;
        drop(output);
        if self.final_path.exists() {
            std::fs::remove_file(&self.final_path)?;
        }
        std::fs::rename(&self.partial_path, &self.final_path)?;
        Ok(self.final_path.clone())
    }
    pub fn abort(mut self) {
        self.muxer.take();
        let _ = std::fs::remove_file(&self.partial_path);
    }
}

impl Drop for AtomicHevcFile {
    fn drop(&mut self) {
        if self.muxer.is_some() {
            self.muxer.take();
            let _ = std::fs::remove_file(&self.partial_path);
        }
    }
}

pub struct AtomicAvcFile {
    muxer: Option<AvcMp4Writer<BufWriter<File>>>,
    final_path: PathBuf,
    partial_path: PathBuf,
}
impl AtomicAvcFile {
    pub fn create(
        path: impl AsRef<Path>,
        width: u32,
        height: u32,
        fps: u32,
        sets: AvcParameterSets,
    ) -> Result<Self, Mp4Error> {
        let final_path = path.as_ref().to_path_buf();
        let partial_path = final_path.with_extension(format!(
            "{}.part",
            final_path
                .extension()
                .and_then(|v| v.to_str())
                .unwrap_or("mp4")
        ));
        if let Some(parent) = partial_path.parent() {
            std::fs::create_dir_all(parent)?
        }
        let file = File::create(&partial_path)?;
        let muxer = AvcMp4Writer::new(BufWriter::new(file), width, height, fps, sets)?;
        Ok(Self {
            muxer: Some(muxer),
            final_path,
            partial_path,
        })
    }
    pub fn write_access_unit(
        &mut self,
        annex_b: &[u8],
        dts: u64,
        pts: u64,
    ) -> Result<(), Mp4Error> {
        self.muxer
            .as_mut()
            .ok_or(Mp4Error::AlreadyFinalized)?
            .write_access_unit(annex_b, dts, pts)
    }
    pub fn finalize(mut self) -> Result<PathBuf, Mp4Error> {
        let muxer = self.muxer.take().ok_or(Mp4Error::AlreadyFinalized)?;
        let mut output = muxer.finalize()?;
        output.flush()?;
        drop(output);
        if self.final_path.exists() {
            std::fs::remove_file(&self.final_path)?
        }
        std::fs::rename(&self.partial_path, &self.final_path)?;
        Ok(self.final_path.clone())
    }
    pub fn abort(mut self) {
        self.muxer.take();
        let _ = std::fs::remove_file(&self.partial_path);
    }
}
impl Drop for AtomicAvcFile {
    fn drop(&mut self) {
        if self.muxer.is_some() {
            self.muxer.take();
            let _ = std::fs::remove_file(&self.partial_path);
        }
    }
}

#[derive(Debug, Error)]
pub enum Mp4Error {
    #[error("HEVC packet 缺少 Annex-B start code")]
    MissingStartCode,
    #[error("NAL unit 過大")]
    NalTooLarge,
    #[error("缺少 HEVC {0}")]
    MissingParameterSet(&'static str),
    #[error("解析度不合法")]
    InvalidDimensions,
    #[error("FPS 不合法")]
    InvalidFps,
    #[error("時間戳溢位")]
    TimestampOverflow,
    #[error("輸出已完成")]
    AlreadyFinalized,
    #[error("檔案系統錯誤：{0}")]
    Io(#[from] std::io::Error),
    #[error("MP4 封裝失敗：{0}")]
    Backend(#[from] mp4::Error),
}

impl PartialEq for Mp4Error {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
impl Eq for Mp4Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn access_unit() -> Vec<u8> {
        vec![
            0, 0, 0, 1, 64, 1, 2, 0, 0, 1, 66, 3, 4, 0, 0, 1, 68, 5, 6, 0, 0, 1, 38, 7, 8,
        ]
    }
    #[test]
    fn identifies_parameter_sets_and_idr() {
        let kinds: Vec<_> = split_annex_b(&access_unit())
            .into_iter()
            .map(|n| n.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                HevcNalKind::Vps,
                HevcNalKind::Sps,
                HevcNalKind::Pps,
                HevcNalKind::Idr
            ]
        );
    }
    #[test]
    fn converts_to_mp4_length_prefix() {
        let value = annex_b_to_length_prefixed(&[0, 0, 1, 64, 1, 2]).unwrap();
        assert_eq!(value, &[0, 0, 0, 3, 64, 1, 2]);
    }
    #[test]
    fn rejects_raw_packet() {
        assert_eq!(
            annex_b_to_length_prefixed(&[1, 2, 3]),
            Err(Mp4Error::MissingStartCode)
        );
    }
    #[test]
    fn requires_all_parameter_sets() {
        assert!(matches!(
            parameter_sets(&[0, 0, 1, 64, 1]),
            Err(Mp4Error::MissingParameterSet("SPS"))
        ));
    }
    #[test]
    fn writes_hvc1_with_hvcc_and_exact_samples() {
        let packet = access_unit();
        let sets = parameter_sets(&packet).unwrap();
        let cursor = Cursor::new(Vec::new());
        let mut writer = HevcMp4Writer::new(cursor, 3840, 2160, 60, sets).unwrap();
        writer.write_access_unit(&packet, 0, 0).unwrap();
        writer.write_access_unit(&[0, 0, 1, 2, 9, 9], 1, 3).unwrap();
        assert_eq!(writer.frames(), 2);
        let cursor = writer.finalize().unwrap();
        let bytes = cursor.into_inner();
        assert!(bytes.windows(4).any(|v| v == b"hvc1"));
        assert!(bytes.windows(4).any(|v| v == b"hvcC"));
        let size = bytes.len() as u64;
        let reader = mp4::Mp4Reader::read_header(Cursor::new(bytes), size).unwrap();
        assert_eq!(reader.sample_count(1).unwrap(), 2);
    }
    #[test]
    fn aborted_file_removes_partial_output() {
        let base =
            std::env::temp_dir().join(format!("gpx-animator-abort-{}.mp4", std::process::id()));
        let packet = access_unit();
        let sets = parameter_sets(&packet).unwrap();
        let output = AtomicHevcFile::create(&base, 3840, 2160, 60, sets).unwrap();
        let partial = output.partial_path.clone();
        assert!(partial.exists());
        output.abort();
        assert!(!partial.exists());
        assert!(!base.exists());
    }
    #[test]
    fn writes_avc1_with_parameter_sets() {
        let packet = [
            0, 0, 1, 0x67, 0x64, 0, 0x1f, 0, 0, 1, 0x68, 0xee, 0x3c, 0x80, 0, 0, 1, 0x65, 5, 6,
        ];
        let sets = avc_parameter_sets(&packet).unwrap();
        let mut writer = AvcMp4Writer::new(Cursor::new(Vec::new()), 1920, 1080, 60, sets).unwrap();
        writer.write_access_unit(&packet, 0, 0).unwrap();
        let bytes = writer.finalize().unwrap().into_inner();
        assert!(bytes.windows(4).any(|value| value == b"avc1"));
        let size = bytes.len() as u64;
        let reader = mp4::Mp4Reader::read_header(Cursor::new(bytes), size).unwrap();
        assert_eq!(reader.sample_count(1).unwrap(), 1);
    }
}
