use bytemuck::{Pod, TransparentWrapper, Zeroable};
use derive_more::{Display, Into};

use crate::util::Array;

#[derive(Clone, Copy, Debug, Display, Default, Into, Pod, Zeroable, TransparentWrapper)] // ?: Maybe protected Into
#[repr(transparent)]
pub struct U7(u8);

impl U7 {
    pub fn new(raw: u8) -> Self {
        debug_assert_eq!(0b1000_0000 & raw, 0);

        Self(raw)
    }

    pub fn new_checked(byte: u8) -> Option<Self> {
        (byte < 0b1000_0000).then_some(Self(byte))
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
    fn convert_chunk(input: Self::InputBuffer, len: u8) -> (Self::OutputBuffer, u8);
}

pub struct U8ToU7;
impl U8ToU7 {
    pub fn convert_len(len: usize) -> usize {
        let bits = len * 8;
        bits / 7 + u8::from(bits % 7 != 0) as usize
    }
}
impl Convert for U8ToU7 {
    type Input = u8;
    type InputBuffer = [u8; 7];

    type Output = U7;
    type OutputBuffer = [U7; 8];

    fn convert_chunk(input: Self::InputBuffer, len: u8) -> (Self::OutputBuffer, u8) {
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
}

pub struct U7ToU8;
impl U7ToU8 {
    pub fn convert_len(len: usize) -> usize {
        let bits = len * 7;
        bits / 8 + u8::from(bits % 8 != 0) as usize
    }
}

impl Convert for U7ToU8 {
    type Input = U7;
    type InputBuffer = [U7; 8];

    type Output = u8;
    type OutputBuffer = [u8; 7];

    fn convert_chunk(input: Self::InputBuffer, len: u8) -> (Self::OutputBuffer, u8) {
        let mut output = [0; 7];
        let mut amount_to_take = 0;

        assert!(len as usize <= Self::InputBuffer::LEN);
        if len > 1 {
            let (msbs, input) = input.split_first().expect("it's an array");
            for (idx, byte) in input.iter().enumerate().take(len as usize - 1) {
                output[idx] = byte.0 | msbs.take_nth_msb(idx);
                amount_to_take += 1;
            }
        }

        (output, amount_to_take)
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
    C::InputBuffer: Default,
{
    pub fn new(iter: Iter) -> Self
    where
        C::OutputBuffer: Default,
    {
        let mut this = Self {
            inner: iter,
            buffer: C::OutputBuffer::default().into_iter(),
            amount_to_take: 0,
        };
        this.setup_new_buffer();
        this
    }

    fn setup_new_buffer(&mut self) {
        let mut input = C::InputBuffer::default();
        let mut input_len = 0;

        for (idx, byte) in self.inner.by_ref().enumerate().take(C::InputBuffer::LEN) {
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
    C::InputBuffer: Default,
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
}
