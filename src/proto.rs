use std::io;
use std::mem;
use std::string::FromUtf8Error;

use arrayref::{array_ref, array_refs};
use bytemuck::cast_slice;
use derive_more::Display;
use hex_literal::hex;
use thiserror::Error;

use crate::seven_bit::FromKorgData;
use crate::seven_bit::IntoKorgData;
use crate::seven_bit::U7ToU8;
use crate::seven_bit::U8ToU7;
use crate::seven_bit::U7;
use crate::util;
use crate::util::array_type_refs;

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("could not parse header: {0}")]
    InvalidHeader(#[from] ParseHeaderError),
    #[error("could not parse payload")]
    InvalidData,
    #[error("not enough data")]
    NotEnoughData,
    #[error("invalid id: expected {expected:02X?}, received:02X?")]
    IvanlidId {
        expected: Box<[u8]>,
        received: Box<[u8]>,
    }, // TODO: SmallBox
    #[error("missing end byte")]
    InvalidEndByte,
    #[error("invalid string: {0}")]
    MalformedString(#[from] FromUtf8Error),
}

pub const EST: u8 = 0xF0;
pub const EOX: u8 = 0xF7;

pub const KORG_ID: u8 = 0x42;
pub const VOLCA_SAMPLE_2_ID: [u8; 4] = hex!("2D 01 08 00");

#[derive(Debug, Display, Clone, Copy)]
#[display(fmt = "{}.{}", "self.0", "self.1")]
pub struct Version(u16, u16);

#[derive(Debug, Error)]
pub enum ParseHeaderError {
    #[error("invalid length")]
    InvalidLength,
    #[error("invalid data")]
    InvalidData,
    #[error("invalid function id: expected {expected:02X?}, received:02X?")]
    IvanlidId {
        expected: Box<[u8]>,
        received: Box<[u8]>,
    }, // TODO: SmallBox
}

pub trait Header: Sized {
    type Array: util::Array<ArrayItem = u8>;
    const LEN: usize = <Self::Array as util::Array>::LEN;

    fn parse(slice: &[u8]) -> Result<Self, ParseHeaderError>;
    fn split_and_parse(slice: &[u8]) -> Result<(Self, &[u8]), ParseHeaderError> {
        if slice.len() < Self::LEN {
            return Err(ParseHeaderError::InvalidLength);
        }

        let (header, data) = slice.split_at(Self::LEN);
        Self::parse(header).map(|this| (this, data))
    }
    fn encode(self) -> Self::Array;

    fn from_channel(channel: U7) -> Self;
}

pub trait Message: Sized {
    type Header: Header;
    type Id: util::Array<ArrayItem = u8>;

    const ID: Self::Id;
    const LEN: Option<usize> = None;

    fn len_hint() -> Option<usize> {
        Self::LEN
            // 1 for END_OF_EX
            .map(|len| len + <Self::Header as Header>::LEN + <Self::Id as util::Array>::LEN + 1)
    }
}

pub trait Incoming: Message {
    fn parse(slice: &[u8]) -> Result<(Self::Header, Self), ParseError> {
        let (header, data) = Self::Header::split_and_parse(slice)?;
        if data.len() < <Self::Id as util::Array>::LEN {
            return Err(ParseHeaderError::InvalidLength.into());
        }
        let (id, data) = data.split_at(<Self::Id as util::Array>::LEN);

        if id != Self::ID.as_ref() {
            return Err(ParseHeaderError::IvanlidId {
                expected: Self::ID.as_ref().to_vec().into_boxed_slice(),
                received: id.as_ref().to_vec().into_boxed_slice(),
            }
            .into());
        }
        let (end, data) = data.split_last().ok_or(ParseHeaderError::InvalidLength)?;
        if *end != EOX {
            return Err(ParseError::InvalidEndByte);
        }

        Self::check_length(data)?;
        Self::parse_data(data).map(|data| (header, data))
    }

    fn check_length(slice: &[u8]) -> Result<(), ParseError> {
        if let Some(len) = <Self as Message>::LEN {
            if slice.len() != len {
                return Err(ParseError::NotEnoughData);
            }
        }
        Ok(())
    }

    // Assumes length was checked for messages with defined length.
    fn parse_data(slice: &[u8]) -> Result<Self, ParseError>;
}

pub trait Outgoing: Message {
    fn encode(&self, header: Self::Header, mut dest: impl io::Write) -> io::Result<()> {
        dest.write_all(header.encode().as_ref())?;
        dest.write_all(Self::ID.as_ref())?;
        self.encode_data(&mut dest)?;
        dest.write_all(&[EOX])
    }

    fn encode_data(&self, dest: impl io::Write) -> io::Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct KorgSysEx;
impl KorgSysEx {
    const HEADER: [u8; 2] = [EST, KORG_ID];
}

impl Header for KorgSysEx {
    type Array = [u8; 2];

    fn parse(slice: &[u8]) -> Result<Self, ParseHeaderError> {
        (slice == Self::HEADER)
            .then_some(Self)
            .ok_or(ParseHeaderError::InvalidData)
    }

    fn encode(self) -> Self::Array {
        Self::HEADER
    }

    fn from_channel(_: U7) -> Self {
        Self
    }
}

#[derive(Debug, Clone)]
pub struct ExtendedKorgSysEx {
    global_channel: u8,
}

/// Korg Exclusive Message header. Used in most sample and sequence related messages.
/// Exclusive header is "F0 42 3g 00 01 2D", where `g` is global channel.
impl ExtendedKorgSysEx {
    const CHANNEL_PREFIX: u8 = 3 << 4;
    const SUFFIX: [u8; 3] = hex!("00 01 2D");

    const HEADER_TEMPLATE: [u8; 6] = hex!("F0 42 30 00 01 2D"); // TODO: concat
}

impl Header for ExtendedKorgSysEx {
    type Array = [u8; 6];

    fn parse(slice: &[u8]) -> Result<Self, ParseHeaderError> {
        let (short, extended) = slice.split_at(KorgSysEx::LEN);
        KorgSysEx::parse(short)?;
        let (channel, suffix) = extended
            .split_first()
            .ok_or(ParseHeaderError::InvalidLength)?;
        if suffix == Self::SUFFIX && *channel & 0b1111_0000 == Self::CHANNEL_PREFIX {
            let channel = channel & 0b0000_1111;
            Ok(Self {
                global_channel: channel,
            })
        } else {
            Err(ParseHeaderError::InvalidData)
        }
    }

    fn encode(self) -> Self::Array {
        let mut output = Self::HEADER_TEMPLATE;
        assert_eq!(self.global_channel & 0b1111_0000, 0);
        output[2] = Self::CHANNEL_PREFIX | self.global_channel;
        output
    }

    fn from_channel(channel: U7) -> Self {
        Self {
            global_channel: channel.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchDeviceRequest {
    pub echo: U7,
}

impl Message for SearchDeviceRequest {
    type Header = KorgSysEx;
    type Id = [u8; 2];

    const ID: [u8; 2] = [0x50, 0x00];
    const LEN: Option<usize> = Some(2);
}

impl Outgoing for SearchDeviceRequest {
    fn encode_data(&self, mut dest: impl io::Write) -> io::Result<()> {
        dest.write_all(&[self.echo.as_u8()])
    }
}

#[derive(Debug, Clone)]
pub struct SearchDeviceReply {
    pub echo: U7,
    pub device_id: U7,
    pub version: Version,
}

impl Message for SearchDeviceReply {
    type Header = KorgSysEx;
    type Id = [u8; 2];

    const ID: [u8; 2] = [0x50, 0x01];
    const LEN: Option<usize> = Some(10);
}

impl Incoming for SearchDeviceReply {
    fn parse_data(slice: &[u8]) -> Result<Self, ParseError> {
        let slice = array_ref!(slice, 0, 10);
        let (channel, echo, model_id, minor, major) = array_refs![slice, 1, 1, 4, 2, 2];
        if model_id != &VOLCA_SAMPLE_2_ID {
            return Err(ParseError::IvanlidId {
                expected: VOLCA_SAMPLE_2_ID.to_vec().into_boxed_slice(),
                received: model_id.to_vec().into_boxed_slice(),
            });
        }
        let version = Version(u16::from_le_bytes(*major), u16::from_le_bytes(*minor));

        Ok(Self {
            device_id: U7::new_checked(channel[0]).ok_or(ParseError::InvalidData)?,
            echo: U7::new_checked(echo[0]).ok_or(ParseError::InvalidData)?,
            version,
        })
    }
}

pub const ACK_STATUS: u8 = 0x23;

#[derive(Debug, Error, Clone, Copy)]
pub enum NakStatus {
    #[error("device is busy")]
    Busy = 0x24,
    #[error("sample memory is full")]
    SampleFull = 0x25,
    #[error("invalid data format")]
    DataFormat = 0x26,
}

pub type Status = Result<(), NakStatus>;
impl Message for Status {
    type Header = ExtendedKorgSysEx;
    type Id = [u8; 0];

    const ID: [u8; 0] = [];
    const LEN: Option<usize> = Some(1);
}

impl Incoming for Status {
    fn parse_data(slice: &[u8]) -> Result<Self, ParseError> {
        let (status, _) = slice.split_first().ok_or(ParseError::NotEnoughData)?;
        let status = match *status {
            ACK_STATUS => Ok(()),
            x if x == NakStatus::Busy as u8 => Err(NakStatus::Busy),
            x if x == NakStatus::SampleFull as u8 => Err(NakStatus::SampleFull),
            x if x == NakStatus::DataFormat as u8 => Err(NakStatus::DataFormat),
            _ => return Err(ParseError::NotEnoughData),
        };
        Ok(status)
    }
}

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

fn write_u8(mut dest: impl io::Write, value: u8) -> io::Result<()> {
    let (msb, lsb) = U7::split_u8(value);
    dest.write_all(&[lsb.as_u8(), msb])
}

// Panics if slice length is less than 2
fn read_u8(slice: &[u8]) -> (u8, &[u8]) {
    let (sample_no, data) = slice.split_at(2);
    let [lsb, msb]: [u8; 2] = sample_no.try_into().expect("checked at split");
    (U7::new(lsb).merge(msb == 1), data)
}

#[test]
fn version_fmt() {
    assert_eq!(format!("{}", Version(42, 69)), "42.69");
}
