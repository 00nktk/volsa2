use std::fmt;
use std::ops;

use bytemuck::{cast_slice, Pod, Zeroable};

/// Helper trait for using arrays in trait bounds and associated types
pub trait Array: // TODO: Seal?
    AsRef<[Self::ArrayItem]>
    + ops::IndexMut<usize, Output = Self::ArrayItem>
    + IntoIterator<Item = Self::ArrayItem>
    + Sized
{
    type ArrayItem: Clone + Sized;
    const LEN: usize;

}

impl<const N: usize, T: Clone + Sized> Array for [T; N] {
    type ArrayItem = T;
    const LEN: usize = N;
}

macro_rules! array_type_refs {
    ($slice:expr, $($ty:ty),+ $(,)?) => {
        ::arrayref::array_refs![$slice, $( std::mem::size_of::<$ty>() ),+]
    }
}

pub(crate) use array_type_refs;

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(transparent)]
pub struct Hex(u8);

impl fmt::Debug for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{:02X}", self.0))
    }
}

pub fn hexbuf(slice: &[u8]) -> &[Hex] {
    cast_slice(slice)
}

#[derive(Clone, Copy, Pod, Zeroable)]
#[repr(transparent)]
pub struct Bin(u8);

impl fmt::Debug for Bin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_fmt(format_args!("{:08b}", self.0))
    }
}

pub fn binbuf(slice: &[u8]) -> &[Bin] {
    cast_slice(slice)
}
