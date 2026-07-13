use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use serde::Serialize;
use std::io::{Read, Seek, Write};

use crate::mp4box::*;

/// ISO-BMFF `colr/nclx` metadata for SDR BT.709 video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ColrBox {
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    pub full_range: bool,
}

impl Default for ColrBox {
    fn default() -> Self {
        Self {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        }
    }
}

impl Mp4Box for ColrBox {
    fn box_type(&self) -> BoxType {
        BoxType::ColrBox
    }

    fn box_size(&self) -> u64 {
        HEADER_SIZE + 4 + 2 + 2 + 2 + 1
    }

    fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self).unwrap())
    }

    fn summary(&self) -> Result<String> {
        Ok(format!(
            "primaries={} transfer={} matrix={} full_range={}",
            self.primaries, self.transfer, self.matrix, self.full_range
        ))
    }
}

impl<R: Read + Seek> ReadBox<&mut R> for ColrBox {
    fn read_box(reader: &mut R, size: u64) -> Result<Self> {
        let start = box_start(reader)?;
        let color_type = reader.read_u32::<BigEndian>()?;
        if color_type != u32::from_be_bytes(*b"nclx") {
            return Err(Error::InvalidData("unsupported colr colour type"));
        }
        let primaries = reader.read_u16::<BigEndian>()?;
        let transfer = reader.read_u16::<BigEndian>()?;
        let matrix = reader.read_u16::<BigEndian>()?;
        let full_range = reader.read_u8()? & 0x80 != 0;
        skip_bytes_to(reader, start + size)?;
        Ok(Self {
            primaries,
            transfer,
            matrix,
            full_range,
        })
    }
}

impl<W: Write> WriteBox<&mut W> for ColrBox {
    fn write_box(&self, writer: &mut W) -> Result<u64> {
        let size = self.box_size();
        BoxHeader::new(self.box_type(), size).write(writer)?;
        writer.write_all(b"nclx")?;
        writer.write_u16::<BigEndian>(self.primaries)?;
        writer.write_u16::<BigEndian>(self.transfer)?;
        writer.write_u16::<BigEndian>(self.matrix)?;
        writer.write_u8(if self.full_range { 0x80 } else { 0 })?;
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trips_bt709_metadata() {
        let source = ColrBox::default();
        let mut bytes = Vec::new();
        source.write_box(&mut bytes).unwrap();
        let mut reader = Cursor::new(bytes);
        let header = BoxHeader::read(&mut reader).unwrap();
        let decoded = ColrBox::read_box(&mut reader, header.size).unwrap();
        assert_eq!(source, decoded);
    }
}
