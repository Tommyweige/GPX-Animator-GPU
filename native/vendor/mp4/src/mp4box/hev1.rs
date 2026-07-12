use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::convert::TryFrom;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hev1Box {
    pub data_reference_index: u16,
    pub width: u16,
    pub height: u16,

    #[serde(with = "value_u32")]
    pub horizresolution: FixedPointU16,

    #[serde(with = "value_u32")]
    pub vertresolution: FixedPointU16,
    pub frame_count: u16,
    pub depth: u16,
    pub hvcc: HvcCBox,
}

impl Default for Hev1Box {
    fn default() -> Self {
        Hev1Box {
            data_reference_index: 0,
            width: 0,
            height: 0,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 0x0018,
            hvcc: HvcCBox::default(),
        }
    }
}

impl Hev1Box {
    pub fn new(config: &HevcConfig) -> Self {
        Hev1Box {
            data_reference_index: 1,
            width: config.width,
            height: config.height,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 0x0018,
            hvcc: HvcCBox::new(config),
        }
    }

    pub fn get_type(&self) -> BoxType {
        BoxType::Hev1Box
    }

    pub fn get_size(&self) -> u64 {
        HEADER_SIZE + 8 + 70 + self.hvcc.box_size()
    }
}

impl Mp4Box for Hev1Box {
    fn box_type(&self) -> BoxType {
        self.get_type()
    }

    fn box_size(&self) -> u64 {
        self.get_size()
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!(
            "data_reference_index={} width={} height={} frame_count={}",
            self.data_reference_index, self.width, self.height, self.frame_count
        );
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for Hev1Box {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        reader.read_u32::<BigEndian>()?; // reserved
        reader.read_u16::<BigEndian>()?; // reserved
        let data_reference_index = reader.read_u16::<BigEndian>()?;

        reader.read_u32::<BigEndian>()?; // pre-defined, reserved
        reader.read_u64::<BigEndian>()?; // pre-defined
        reader.read_u32::<BigEndian>()?; // pre-defined
        let width = reader.read_u16::<BigEndian>()?;
        let height = reader.read_u16::<BigEndian>()?;
        let horizresolution = FixedPointU16::new_raw(reader.read_u32::<BigEndian>()?);
        let vertresolution = FixedPointU16::new_raw(reader.read_u32::<BigEndian>()?);
        reader.read_u32::<BigEndian>()?; // reserved
        let frame_count = reader.read_u16::<BigEndian>()?;
        skip_bytes(reader, 32)?; // compressorname
        let depth = reader.read_u16::<BigEndian>()?;
        reader.read_i16::<BigEndian>()?; // pre-defined

        let header = BoxHeader::read(reader)?;
        let BoxHeader { name, size: s } = header;
        if s > size {
            return Err(Error::InvalidData(
                "hev1 box contains a box with a larger size than it",
            ));
        }
        if name == BoxType::HvcCBox {
            let hvcc = HvcCBox::read_box(reader, s)?;

            skip_bytes_to(reader, start + size)?;

            Ok(Hev1Box {
                data_reference_index,
                width,
                height,
                horizresolution,
                vertresolution,
                frame_count,
                depth,
                hvcc,
            })
        } else {
            Err(Error::InvalidData("hvcc not found"))
        }
    }
}

impl<W: Write> WriteBox<&mut W> for Hev1Box {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        writer.write_u32::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(self.data_reference_index)?;

        writer.write_u32::<BigEndian>(0)?; // pre-defined, reserved
        writer.write_u64::<BigEndian>(0)?; // pre-defined
        writer.write_u32::<BigEndian>(0)?; // pre-defined
        writer.write_u16::<BigEndian>(self.width)?;
        writer.write_u16::<BigEndian>(self.height)?;
        writer.write_u32::<BigEndian>(self.horizresolution.raw_value())?;
        writer.write_u32::<BigEndian>(self.vertresolution.raw_value())?;
        writer.write_u32::<BigEndian>(0)?; // reserved
        writer.write_u16::<BigEndian>(self.frame_count)?;
        // skip compressorname
        write_zeros(writer, 32)?;
        writer.write_u16::<BigEndian>(self.depth)?;
        writer.write_i16::<BigEndian>(-1)?; // pre-defined

        self.hvcc.write_box(writer)?;

        Ok(size)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct HvcCBox {
    pub configuration_version: u8,
    pub vps: Vec<u8>,
    pub sps: Vec<u8>,
    pub pps: Vec<u8>,
}

impl HvcCBox {
    pub fn new(config: &HevcConfig) -> Self {
        Self {
            configuration_version: 1,
            vps: config.vps.clone(),
            sps: config.sps.clone(),
            pps: config.pps.clone(),
        }
    }
}

impl Mp4Box for HvcCBox {
    fn box_type(&self) -> BoxType {
        BoxType::HvcCBox
    }

    fn box_size(&self) -> u64 {
        HEADER_SIZE
            + 23
            + [self.vps.len(), self.sps.len(), self.pps.len()]
                .iter()
                .map(|size| 5 + *size as u64)
                .sum::<u64>()
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(&self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        let s = format!("configuration_version={}", self.configuration_version);
        Ok(s)
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for HvcCBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;

        let configuration_version = reader.read_u8()?;
        let mut fixed = [0u8; 21];
        reader.read_exact(&mut fixed)?;
        let array_count = reader.read_u8()?;
        let mut vps = Vec::new();
        let mut sps = Vec::new();
        let mut pps = Vec::new();
        for _ in 0..array_count {
            let nal_type = reader.read_u8()? & 0x3f;
            let count = reader.read_u16::<BigEndian>()?;
            for _ in 0..count {
                let length = reader.read_u16::<BigEndian>()? as usize;
                let mut data = vec![0; length];
                reader.read_exact(&mut data)?;
                match nal_type {
                    32 => vps = data,
                    33 => sps = data,
                    34 => pps = data,
                    _ => {}
                }
            }
        }

        skip_bytes_to(reader, start + size)?;

        Ok(HvcCBox {
            configuration_version,
            vps,
            sps,
            pps,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for HvcCBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;

        writer.write_u8(self.configuration_version)?;
        writer.write_u8(1)?;
        writer.write_u32::<BigEndian>(0x6000_0000)?;
        writer.write_u32::<BigEndian>(0x9000_0000)?;
        writer.write_u16::<BigEndian>(0)?;
        writer.write_u8(153)?;
        writer.write_u16::<BigEndian>(0xf000)?;
        writer.write_u8(0xfc)?;
        writer.write_u8(0xfd)?;
        writer.write_u8(0xf8)?;
        writer.write_u8(0xf8)?;
        writer.write_u16::<BigEndian>(0)?;
        writer.write_u8(0x0f)?;
        writer.write_u8(3)?;
        for (nal_type, data) in [(32, &self.vps), (33, &self.sps), (34, &self.pps)] {
            writer.write_u8(0x80 | nal_type)?;
            writer.write_u16::<BigEndian>(1)?;
            writer.write_u16::<BigEndian>(
                u16::try_from(data.len())
                    .map_err(|_| Error::InvalidData("HEVC parameter set too large"))?,
            )?;
            writer.write_all(data)?;
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mp4box::BoxHeader;
    use std::io::Cursor;

    #[test]
    fn test_hev1() {
        let src_box = Hev1Box {
            data_reference_index: 1,
            width: 320,
            height: 240,
            horizresolution: FixedPointU16::new(0x48),
            vertresolution: FixedPointU16::new(0x48),
            frame_count: 1,
            depth: 24,
            hvcc: HvcCBox {
                configuration_version: 1,
                vps: vec![],
                sps: vec![],
                pps: vec![],
            },
        };
        let mut buf = Vec::new();
        src_box.write_box(&mut buf).unwrap();
        assert_eq!(buf.len(), src_box.box_size() as usize);

        let mut reader = Cursor::new(&buf);
        let header = BoxHeader::read(&mut reader).unwrap();
        assert_eq!(header.name, BoxType::Hev1Box);
        assert_eq!(src_box.box_size(), header.size);

        let dst_box = Hev1Box::read_box(&mut reader, header.size).unwrap();
        assert_eq!(src_box, dst_box);
    }
}
