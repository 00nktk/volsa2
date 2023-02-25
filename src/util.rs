use std::borrow::Cow;
use std::fmt;
use std::io;
use std::ops;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use bytemuck::{cast_slice, Pod, Zeroable};

pub const DEBUG_TRESHOLD: usize = 16;

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

pub fn extract_file_name(path: &Path) -> Result<Cow<'_, str>> {
    if !path.is_file() {
        bail!("path must point to a file: {path:?}")
    }

    path.file_stem()
        .map(|name| name.to_string_lossy())
        .ok_or_else(|| anyhow!("could not extract filename"))
}

pub fn ask(question: &str) -> io::Result<bool> {
    use io::Write;

    let mut buf = String::new();
    let stdin = io::stdin();
    let stdout = io::stdout();
    loop {
        print!("{question} [Y/N]: ");
        stdout.lock().flush()?;
        stdin.read_line(&mut buf)?;
        match buf.as_str() {
            "Y\n" | "y\n" => return Ok(true),
            "N\n" | "n\n" => return Ok(false),
            _ => buf.clear(),
        }
    }
}

pub fn normalize_path(path: &Path, filename: &str) -> PathBuf {
    let mut path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if path.is_dir() {
        path.set_file_name(filename);
        path.set_extension("wav");
    }
    path
}
