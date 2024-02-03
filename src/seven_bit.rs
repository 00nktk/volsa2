use bytemuck::{Pod, TransparentWrapper, Zeroable};
use derive_more::{Display, Into};

use crate::util::Array;

#[rustfmt::skip]
#[derive(Pod, Zeroable, TransparentWrapper)]
#[derive(Clone, Copy, Debug, Display, Default, Into)] // ?: Maybe protected Into
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct U7(u8);

impl U7 {
	pub const MAX: U7 = U7((1 << 7) - 1); // 127
	pub const MIN: U7 = U7(0);

	pub fn new(raw: u8) -> Self {
		debug_assert_eq!(0b1000_0000 & raw, 0);

		Self(raw)
	}

	pub fn new_checked(byte: u8) -> Option<Self> {
		(byte < Self::MAX.0).then_some(Self(byte))
	}

	pub const fn split_u8(num: u8) -> (u8, U7) {
		let msb = (0b1000_0000 & num).rotate_left(1);
		let num = 0b0111_1111 & num;
		(msb, Self(num))
	}

	pub fn merge(self, msb: bool) -> u8 {
		self.0 | (u8::from(msb) << 7)
	}

	pub const fn take_nth_msb(self, n: usize) -> u8 {
		(self.0 & (1 << n)).rotate_left(7 - n as u32)
	}

	pub const fn as_u8(self) -> u8 {
		self.0
	}
}

pub type FromKorgData<I> = Converter<I, U7ToU8>;
pub type IntoKorgData<I> = Converter<I, U8ToU7>;

pub trait Convert {
	type Input: Sized;
	type InputBuffer: Array<ArrayItem = Self::Input>;

	type Output: Sized;
	type OutputBuffer: Array<ArrayItem = Self::Output>;

	/// Len must be less or equal to input length
	fn convert_chunk(
		input: Self::InputBuffer,
		len: u8,
	) -> (Self::OutputBuffer, u8);

	fn output_len(input_len: usize) -> usize;
}

pub struct U8ToU7;
impl U8ToU7 {
	pub fn convert_len(len: usize) -> usize {
		let mut msbs = len / 7;
		if len % 7 != 0 {
			msbs += 1;
		}
		len + msbs
		// let bits = len * 8;
		// let num_octets = bits / 7 + u8::from(bits % 7 != 0) as usize;
	}
}
impl Convert for U8ToU7 {
	type Input = u8;
	type InputBuffer = [u8; 7];

	type Output = U7;
	type OutputBuffer = [U7; 8];

	fn convert_chunk(
		input: Self::InputBuffer,
		len: u8,
	) -> (Self::OutputBuffer, u8) {
		let mut output = [U7(0); 8];
		let mut amount_to_take = 0;

		assert!(len as usize <= Self::InputBuffer::LEN);
		for (idx, byte) in input.into_iter().enumerate().take(len as usize) {
			let (msb, byte7) = U7::split_u8(byte);
			output[0].0 |= msb << idx;
			output[idx + 1] = byte7;
			amount_to_take += 1;
		}

		if amount_to_take > 0 {
			amount_to_take += 1;
		}

		(output, amount_to_take)
	}

	fn output_len(input_len: usize) -> usize {
		Self::convert_len(input_len)
	}
}

pub struct U7ToU8;
impl U7ToU8 {
	pub fn convert_len(len: usize) -> usize {
		if len == 0 {
			0
		} else {
			// Number of bytes that hold MSBs for rest in the octet.
			let mut msbs = len / 8;
			if len % 8 != 0 {
				msbs += 1;
			}
			len - msbs
		}
	}
}

impl Convert for U7ToU8 {
	type Input = U7;
	type InputBuffer = [U7; 8];

	type Output = u8;
	type OutputBuffer = [u8; 7];

	fn convert_chunk(
		input: Self::InputBuffer,
		len: u8,
	) -> (Self::OutputBuffer, u8) {
		let mut output = [0; 7];
		let mut amount_to_take = 0;

		assert!(len as usize <= Self::InputBuffer::LEN);
		if len > 1 {
			let (msbs, input) = input.split_first().expect("it's an array");
			for (idx, byte) in input.iter().enumerate().take(len as usize - 1)
			{
				output[idx] = byte.0 | msbs.take_nth_msb(idx);
				amount_to_take += 1;
			}
		}

		(output, amount_to_take)
	}

	fn output_len(input_len: usize) -> usize {
		Self::convert_len(input_len)
	}
}

// Helper type to extract IntoIter
type OutputIter<C> = <<C as Convert>::OutputBuffer as IntoIterator>::IntoIter;

pub struct Converter<I, C: Convert> {
	inner: I,
	buffer: OutputIter<C>,
	amount_to_take: u8,
}

// TODO: exact size
impl<Iter, C> Converter<Iter, C>
where
	Iter: Iterator<Item = C::Input>,
	C: Convert,
	C::InputBuffer: Zeroable,
{
	pub fn new(iter: Iter) -> Self
	where
		C::OutputBuffer: Zeroable,
	{
		let mut this = Self {
			inner: iter,
			buffer: C::OutputBuffer::zeroed().into_iter(),
			amount_to_take: 0,
		};
		this.setup_new_buffer();
		this
	}

	fn setup_new_buffer(&mut self) {
		let mut input = C::InputBuffer::zeroed();
		let mut input_len = 0;

		for (idx, byte) in
			self.inner.by_ref().enumerate().take(C::InputBuffer::LEN)
		{
			input[idx] = byte;
			input_len += 1;
		}

		if input_len > 0 {
			let (output, amount_to_take) = C::convert_chunk(input, input_len);
			self.amount_to_take = amount_to_take;
			self.buffer = output.into_iter();
		}
	}
}

impl<I, C> Iterator for Converter<I, C>
where
	I: Iterator<Item = C::Input>,
	C: Convert,
	C::InputBuffer: Zeroable,
{
	type Item = C::Output;

	fn next(&mut self) -> Option<Self::Item> {
		if self.amount_to_take == 0 {
			self.setup_new_buffer();
		}

		if self.amount_to_take > 0 {
			self.amount_to_take -= 1;
			self.buffer.next()
		} else {
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use std::any::type_name;
	use std::fmt::Debug;

	use proptest::arbitrary::any;
	use proptest::collection::vec;
	use proptest::strategy::Strategy;
	use proptest::{prop_compose, proptest};

	use super::*;

	#[test]
	fn extract_msb() {
		assert_eq!(U7(0b0000_0001).take_nth_msb(0), 0b1000_0000);
		assert_eq!(U7(0b0000_0001).take_nth_msb(1), 0b0000_0000);

		assert_eq!(U7(0b0000_0010).take_nth_msb(1), 0b1000_0000);
		assert_eq!(U7(0b0000_0001).take_nth_msb(0), 0b1000_0000);

		assert_eq!(U7(0b0000_0100).take_nth_msb(2), 0b1000_0000);
		assert_eq!(U7(0b0000_1000).take_nth_msb(3), 0b1000_0000);
		assert_eq!(U7(0b0001_0000).take_nth_msb(4), 0b1000_0000);
		assert_eq!(U7(0b0010_0000).take_nth_msb(5), 0b1000_0000);
		assert_eq!(U7(0b0100_0001).take_nth_msb(6), 0b1000_0000);
		assert_eq!(U7(0b1000_0001).take_nth_msb(7), 0b1000_0000);

		assert_eq!(U7(0b0000_0100).take_nth_msb(1), 0b0000_0000);
		assert_eq!(U7(0b0000_1000).take_nth_msb(2), 0b0000_0000);
		assert_eq!(U7(0b0001_0000).take_nth_msb(3), 0b0000_0000);
		assert_eq!(U7(0b0010_0000).take_nth_msb(4), 0b0000_0000);
		assert_eq!(U7(0b0100_0001).take_nth_msb(5), 0b0000_0000);
		assert_eq!(U7(0b1000_0001).take_nth_msb(6), 0b0000_0000);
	}

	prop_compose! {
		fn u7_full_range()(raw in U7::MIN.as_u8()..U7::MAX.as_u8()) -> U7 {
			U7::new_checked(raw).expect("overflow")
		}
	}

	prop_compose! {
		fn u7_len(from: usize, to: usize)
				 (len in (from..to)
				 .prop_filter("U7 array cannot be of len 8n + 1", |x| x % 8 == 1))
				 -> usize
		{
			len
		}
	}

	fn filter_map_u7_vec(mut data: Vec<U7>) -> Option<Vec<U7>> {
		if data.len() % 8 == 1 {
			// U7 data array cannot be of length 8n + 1, since every first byte in an octet
			// hold MSBs of other bytes.
			return None;
		}

		let tail_len = data.len() % 8;
		// If the last octet in data vec is not full length, we must clear unused bits in MSB byte
		// so we dont test uninterpretebale data.
		if tail_len > 0 {
			let len = data.len();
			let tail_msb = &mut data[len - tail_len];
			let pre_tail_msb = *tail_msb;
			// Just unset bits for inexistent bytes.
			for idx in tail_len..8 {
				let idx = idx - 1; // From 1-based index to 0-based index
				tail_msb.0 &= !(1 << idx);
			}
			println!("tail_msb: {pre_tail_msb} -> {tail_msb} for {len}");
		}
		Some(data)
	}

	/// Tests that no data is corrupted after forward-backward convertion
	fn test_two_way<F, B>(data: Vec<F::Input>)
	where
		F: Convert,
		B: Convert<Input = F::Output, Output = F::Input>,

		F::InputBuffer: Zeroable,
		F::OutputBuffer: Zeroable,
		B::InputBuffer: Zeroable,
		B::OutputBuffer: Zeroable,

		F::Input: Debug + PartialEq + Clone,
	{
		let predicted_size = F::output_len(data.len());
		let converted_data =
			Converter::<_, F>::new(data.iter().cloned()).collect::<Vec<_>>();
		assert_eq!(
			converted_data.len(),
			predicted_size,
			"{}(output) data predicted len",
			type_name::<F::Output>()
		);
		assert_eq!(
			data.len(),
			B::output_len(converted_data.len()),
			"{}(input) data predicted len",
			type_name::<F::Input>()
		);

		let recovered_data =
			Converter::<_, B>::new(converted_data.into_iter())
				.collect::<Vec<_>>();
		assert_eq!(recovered_data, data);
	}

	/// Tests converter iterator logic
	fn test_converter<C: Convert>(data: Vec<C::Input>)
	where
		C::Input: Copy,
		C::InputBuffer: Zeroable + AsMut<[C::Input]>,
		C::Output: Debug + PartialEq,
		C::OutputBuffer: Zeroable,
	{
		let mut converted_data_expected = Vec::new();
		for chunk in data.chunks(C::InputBuffer::LEN) {
			let len = chunk.len();
			let mut array = C::InputBuffer::zeroed();
			array.as_mut()[..chunk.len()].copy_from_slice(chunk);
			let (chunk, to_take) = C::convert_chunk(array, len as u8);
			converted_data_expected
				.extend(chunk.into_iter().take(to_take as usize));
		}

		let converted_data =
			Converter::<_, C>::new(data.into_iter()).collect::<Vec<_>>();

		assert_eq!(converted_data, converted_data_expected);
	}

	proptest! {
		#[test]
		fn u8_to_u7_and_back(data in vec(u8::MIN..u8::MAX, 0..(1024 * 100))) {
			test_two_way::<U8ToU7, U7ToU8>(data)
		}

		#[test]
		fn u7_to_u8_and_back(
			data in vec(u7_full_range(), 0..(1024 * 100)).prop_filter_map(
				"U7 array cannot be of len 8n + 1",
				filter_map_u7_vec
			)
		) {
			test_two_way::<U7ToU8, U8ToU7>(data)
		}

		#[test]
		fn converter_u8_to_u7(data in vec(u8::MIN..u8::MAX, 0..(1024 * 100))) {
			test_converter::<U8ToU7>(data)
		}

		#[test]
		fn converter_u7_to_u8(
			data in vec(u7_full_range(), 0..(1024 * 100)).prop_filter_map(
				"U7 array cannot be of len 8n + 1",
				filter_map_u7_vec
			)
		) {
			test_converter::<U7ToU8>(data)
		}

		#[test]
		fn take_msb(nth in 0..7usize, is_one in any::<bool>()) {
			let mut num = 0u8;
			if is_one {
				num |= 1 << nth;
			}

			let num = U7::new(num);
			assert_eq!(num.take_nth_msb(nth) == 1 << 7, is_one);
		}
	}
}
