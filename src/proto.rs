mod header;
mod sample;
mod system;

use std::io;
use std::string::FromUtf8Error;

use derive_more::Display;
use hex_literal::hex;
use thiserror::Error;

use crate::seven_bit::U7;
use crate::util;

pub use header::{ExtendedKorgSysEx, Header, KorgSysEx, ParseHeaderError};
pub use sample::{
	SampleData, SampleDataDumpRequest, SampleHeader, SampleHeaderDumpRequest,
};
pub use sample::{SampleSpaceDump, SampleSpaceDumpRequest};
pub use system::{SearchDeviceReply, SearchDeviceRequest, Status};

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

/// Exclusive status magic.
pub const EST: u8 = 0xF0;
/// End of exclusive magic.
pub const EOX: u8 = 0xF7;

/// KORG manufacturer ID.
pub const KORG_ID: u8 = 0x42;
/// Volca Sample 2 ID.
pub const VOLCA_SAMPLE_2_ID: [u8; 4] = hex!("2D 01 08 00");

/// Volca Sample firmware version
#[derive(Debug, Display, Clone, Copy)]
#[display(fmt = "{}.{}", "self.0", "self.1")]
pub struct Version(u16, u16);

/// A common message trait that is implemented for all supported SysEx message types.
pub trait Message: Sized {
	/// Message header type.
	type Header: Header;
	/// Message Function ID raw representation.
	type Id: util::Array<ArrayItem = u8>;

	/// Message Function ID.
	const ID: Self::Id;
	/// Message length.
	const LEN: Option<usize> = None;

	fn len_hint() -> Option<usize> {
		Self::LEN
			// 1 for END_OF_EX
			.map(|len| {
				len + <Self::Header as Header>::LEN
					+ <Self::Id as util::Array>::LEN
					+ 1
			})
	}
}

/// A Message that can be *transmitted by* KORG Volca Sample 2.
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
		let (end, data) =
			data.split_last().ok_or(ParseHeaderError::InvalidLength)?;
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

	/// Parse message payload.
	// Assumes length was checked for messages with defined length.
	fn parse_data(slice: &[u8]) -> Result<Self, ParseError>;
}

/// A Message that can be *received by* KORG Volca Sample 2.
pub trait Outgoing: Message {
	fn encode(
		&self,
		header: Self::Header,
		mut dest: impl io::Write,
	) -> io::Result<()> {
		dest.write_all(header.encode().as_ref())?;
		dest.write_all(Self::ID.as_ref())?;
		self.encode_data(&mut dest)?;
		dest.write_all(&[EOX])
	}

	fn encode_data(&self, dest: impl io::Write) -> io::Result<()>;
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
