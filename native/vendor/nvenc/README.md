### If you like my work and want to support what I do, support me on Ko-Fi 💜!
[![Ko-Fi](https://img.shields.io/badge/Ko--fi-F16061?style=for-the-badge&logo=ko-fi&logoColor=white)](https://ko-fi.com/cyberite)
---

![Crates.io MSRV](https://img.shields.io/crates/msrv/nvenc?style=for-the-badge) ![Crates.io License](https://img.shields.io/crates/l/nvenc?style=for-the-badge) ![GitHub Repo stars](https://img.shields.io/github/stars/AlsoSylv/nvenc?style=for-the-badge)

The following example is for Linux GLX

```no_run
use nvenc::{encoder::Encoder, session::{InitParams, NeedsConfig, Session}};
use nvenc::sys::enums::{NVencBufferFormat, NVencTuningInfo};
use nvenc::sys::guids::{NV_ENC_CODEC_H264_GUID, NV_ENC_PRESET_P3_GUID};

fn main() -> Result<(), nvenc::sys::result::NVencError> {
    // Setup GLX context
    let session = Session::<NeedsConfig>::open_gl()?;
    let (session, mut config) = session.get_encode_preset_config_ex(
            NV_ENC_CODEC_H264_GUID,
            NV_ENC_PRESET_P3_GUID,
            NVencTuningInfo::LowLatency
        )?;
    let init_params = InitParams {
        encode_guid: NV_ENC_CODEC_H264_GUID,
        preset_guid: NV_ENC_PRESET_P3_GUID,
        resolution: [1920, 1080],
        aspect_ratio: [16, 9],
        frame_rate: [30, 1],
        tuning_info: NVencTuningInfo::LowLatency,
        buffer_format: NVencBufferFormat::ARGB,
        encode_config: &mut config.preset_cfg,
        enable_ptd: true,
        enable_async: false,
        max_encoder_resolution: [1920, 1080],
    };
    let _encoder: Encoder = session.init_encoder(init_params)?;
    Ok(())
}
```
