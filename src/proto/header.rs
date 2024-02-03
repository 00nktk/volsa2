//! Korg SysEx header types.

use hex_literal::hex;
use thiserror::Error;

use crate::seven_bit::U7;
use crate::util;

use super::{EST, KORG_ID};

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

/// Trait holding common header logic.
pub trait Header: Sized {
    /// Header raw (array) representation.
    ///
    /// Main purpose is to statically define header length, since we cannot use associated
    /// constants in const-generics.
    type Array: util::Array<ArrayItem = u8>;
    const LEN: usize = <Self::Array as util::Array>::LEN;

    fn parse(slice: &[u8]) -> Result<Self, ParseHeaderError>;
    /// Tries to parse header from a slice and returns the header and the remaining unparsed data.
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

/// Korg Exclusive Message header. Used in most sample and sequence related messages.
/// Exclusive header is "F0 42 3g 00 01 2D", where `g` is global channel.
#[derive(Debug, Clone)]
pub struct ExtendedKorgSysEx {
    global_channel: u8,
}

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
