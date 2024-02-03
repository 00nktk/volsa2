//! Messages for interacting with volca's sample storage.

use std::io;
use std::mem;

use arrayref::{array_ref, array_refs};
use bytemuck::cast_slice;

use crate::seven_bit::{FromKorgData, IntoKorgData, U7ToU8, U8ToU7, U7};
use crate::util::array_type_refs;

use super::header::ExtendedKorgSysEx;
use super::{read_u8, write_u8, Incoming, Message, Outgoing, ParseError};

// ===== Sample Space =====

/// Request [`SampleSpaceDump`].
#[derive(Debug, Clone, Copy)]
pub struct SampleSpaceDumpRequest;

impl Message for SampleSpaceDumpRequest {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 1];

    const ID: [u8; 1] = [0x1B];
    const LEN: Option<usize> = Some(0);
}

impl Outgoing for SampleSpaceDumpRequest {
    fn encode_data(&self, _: impl io::Write) -> io::Result<()> {
        Ok(())
    }
}

/// Info about used and available storage.
#[derive(Debug, Clone)]
pub struct SampleSpaceDump {
    pub all_sector_size: u16,
    pub used_sector_size: u16,
}

impl SampleSpaceDump {
    pub fn occupied(&self) -> f64 {
        self.used_sector_size as f64 / self.all_sector_size as f64
    }
}

impl Message for SampleSpaceDump {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 1];

    const ID: [u8; 1] = [0x4B];
    const LEN: Option<usize> = Some(4);
}

impl Incoming for SampleSpaceDump {
    fn parse_data(slice: &[u8]) -> Result<Self, ParseError> {
        let slice = array_ref!(slice, 0, 4);
        // Field order are likely messed up in the documentation
        let (&[used_lsb, used_msb], &[all_lsb, all_msb]) = array_refs![slice, 2, 2];

        let mut all_sector_size = all_lsb as u16;
        all_sector_size |= (all_msb as u16) << 7;

        let mut used_sector_size = used_lsb as u16;
        used_sector_size |= (used_msb as u16) << 7;

        Ok(Self {
            all_sector_size,
            used_sector_size,
        })
    }
}

// ===== Sample Header =====

/// Request [`SampleHeader`].
#[derive(Debug, Clone)]
pub struct SampleHeaderDumpRequest {
    pub sample_no: u8,
}

impl Message for SampleHeaderDumpRequest {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 1];

    const ID: [u8; 1] = [0x1E];
    const LEN: Option<usize> = Some(2);
}

impl Outgoing for SampleHeaderDumpRequest {
    fn encode_data(&self, dest: impl io::Write) -> io::Result<()> {
        write_u8(dest, self.sample_no)
    }
}

/// Meta information about sample.
#[derive(Debug, Clone)]
pub struct SampleHeader {
    pub sample_no: u8,
    pub name: String,
    pub length: u32,
    pub level: u16,
    pub speed: u16,
}

impl SampleHeader {
    const DATA_SIZE_7BIT: usize = 37;
    const NAME_LEN: usize = 24;
    const DEFAULT_SPEED: u16 = 16384;
    const DEFAULT_LEVEL: u16 = 65535;

    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.length == 0 && self.level == 0 && self.speed == 0
    }

    pub fn empty(sample_no: u8) -> Self {
        Self {
            sample_no,
            name: String::new(),
            length: 0,
            level: 0,
            speed: 0,
        }
    }
}

impl Message for SampleHeader {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 1];

    const ID: [u8; 1] = [0x4E];
    const LEN: Option<usize> = Some(39);
}

impl Incoming for SampleHeader {
    #[allow(clippy::ptr_offset_with_cast)]
    fn parse_data(slice: &[u8]) -> Result<Self, ParseError> {
        let (sample_no, data) = read_u8(slice);

        // TODO: POD cast, reserve
        let mut data: Vec<u8> = FromKorgData::new(data.iter().copied().map(U7::new)).collect();
        if data.len() < 32 {
            return Err(ParseError::NotEnoughData);
        }

        let sample_props = array_ref![
            &data,
            Self::NAME_LEN,
            mem::size_of::<u32>() + 2 * mem::size_of::<u16>()
        ];
        let (length, level, speed) = array_type_refs![sample_props, u32, u16, u16];
        let length = u32::from_le_bytes(*length);
        let level = u16::from_le_bytes(*level);
        let speed = u16::from_le_bytes(*speed);

        data.truncate(Self::NAME_LEN);
        let zeros = data.iter().rev().take_while(|c| **c == 0).count();
        data.truncate(Self::NAME_LEN - zeros);

        Ok(Self {
            sample_no,
            length,
            level,
            speed,
            name: String::from_utf8(data)?,
        })
    }
}

impl Outgoing for SampleHeader {
    fn encode_data(&self, mut dest: impl io::Write) -> io::Result<()> {
        write_u8(&mut dest, self.sample_no)?;
        let mut buf = [U7::new(0); Self::DATA_SIZE_7BIT];

        let name_padding = Self::NAME_LEN - self.name.len();
        let raw_data = self
            .name
            .bytes()
            .chain(std::iter::repeat(0).take(name_padding))
            .chain(self.length.to_le_bytes())
            .chain(self.level.to_le_bytes())
            .chain(self.speed.to_le_bytes());
        IntoKorgData::new(raw_data)
            .enumerate()
            .for_each(|(idx, byte)| buf[idx] = byte);

        dest.write_all(cast_slice(&buf))
    }
}

// ===== Sample Data =====

/// Request [`SampleData`].
#[derive(Debug, Clone)]
pub struct SampleDataDumpRequest {
    pub sample_no: u8,
}

impl Message for SampleDataDumpRequest {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 1];

    const ID: [u8; 1] = [0x1F];
    const LEN: Option<usize> = Some(2);
}

impl Outgoing for SampleDataDumpRequest {
    fn encode_data(&self, dest: impl io::Write) -> io::Result<()> {
        write_u8(dest, self.sample_no)
    }
}

/// Sample audio data.
#[derive(Clone, Debug)]
pub struct SampleData {
    pub sample_no: u8,
    pub data: Vec<i16>,
}

impl SampleData {
    pub fn new(sample_no: u8, name: &str, data: Vec<i16>) -> (SampleHeader, SampleData) {
        let name_len = name.len().min(SampleHeader::NAME_LEN);
        let name = name[..name_len].to_string();
        let header = SampleHeader {
            sample_no,
            name,
            length: data.len() as u32,
            level: SampleHeader::DEFAULT_LEVEL,
            speed: SampleHeader::DEFAULT_SPEED,
        };
        let data = SampleData { sample_no, data };

        (header, data)
    }
}

impl Message for SampleData {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 1];

    const ID: [u8; 1] = [0x4F];
}

impl Incoming for SampleData {
    fn parse_data(slice: &[u8]) -> Result<Self, ParseError> {
        let (sample_no, data) = read_u8(slice);
        let mut buf = Vec::with_capacity(U7ToU8::convert_len(data.len()) / 2 + 1);
        let mut current_num = [0, 0];
        FromKorgData::new(data.iter().copied().map(U7::new)) // TODO: Pod cast
            .enumerate()
            .for_each(|(idx, byte)| {
                if idx % 2 == 0 {
                    current_num = [byte, 0];
                } else {
                    current_num[1] = byte;
                    buf.push(i16::from_le_bytes(current_num));
                }
            });
        Ok(SampleData {
            sample_no,
            data: buf,
        })
    }
}

impl Outgoing for SampleData {
    fn encode_data(&self, mut dest: impl io::Write) -> io::Result<()> {
        write_u8(&mut dest, self.sample_no)?;

        let buf_len = U8ToU7::convert_len(self.data.len() * 2);
        let mut buf = Vec::with_capacity(buf_len);
        let bytes_u8 = self.data.iter().copied().flat_map(i16::to_le_bytes);
        let bytes_u7 = IntoKorgData::new(bytes_u8);
        buf.extend(bytes_u7);
        dest.write_all(cast_slice(&buf))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Read;

    use hound::WavReader;

    use super::*;

    fn test_template(idx: usize) {
        let expected = WavReader::open(format!("test_data/sample{idx}.wav.raw"))
            .unwrap()
            .into_samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let data_dump = File::open(format!("test_data/sample_data_dump{idx}.raw"))
            .unwrap()
            .bytes()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let sample_data = SampleData::parse(&data_dump).unwrap().1;

        assert_eq!(sample_data.data, expected);
    }

    #[test]
    fn test_sample_1() {
        test_template(1)
    }

    #[test]
    fn test_sample_2() {
        test_template(2)
    }

    #[test]
    fn test_sample_3() {
        test_template(3)
    }

    #[test]
    fn test_sample_4() {
        test_template(4)
    }

    #[test]
    fn test_sample_5() {
        test_template(5)
    }

    #[test]
    fn test_sample_6() {
        test_template(6)
    }

    #[test]
    fn test_sample_7() {
        test_template(7)
    }

    #[test]
    fn test_sample_8() {
        test_template(8)
    }

    #[test]
    fn test_sample_9() {
        test_template(9)
    }

    #[test]
    fn test_sample_10() {
        test_template(10)
    }

    #[test]
    fn test_sample_11() {
        test_template(11)
    }

    #[test]
    fn test_sample_12() {
        test_template(12)
    }

    #[test]
    fn test_sample_13() {
        test_template(13)
    }

    #[test]
    fn test_sample_14() {
        test_template(14)
    }
}
