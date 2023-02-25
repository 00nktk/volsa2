//! Utility messages.

use std::io;

use arrayref::{array_ref, array_refs};
use thiserror::Error;

use crate::seven_bit::U7;

use super::header::{ExtendedKorgSysEx, KorgSysEx};
use super::{Incoming, Message, Outgoing, ParseError, Version, VOLCA_SAMPLE_2_ID};

/// Acknowledge status magic.
pub const ACK_STATUS: u8 = 0x23;
/// Not-Acknowledge status.
#[derive(Debug, Error, Clone, Copy)]
pub enum NakStatus {
    #[error("device is busy")]
    Busy = 0x24,
    #[error("sample memory is full")]
    SampleFull = 0x25,
    #[error("invalid data format")]
    DataFormat = 0x26,
}

/// Message representing result of an operation.
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

/// Discovery request.
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

/// Discovery response.
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
