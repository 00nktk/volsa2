use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use auto_enums::auto_enum;
use clap::ValueEnum;
use derive_more::Display;
use hound::{Result as WavResult, SampleFormat, WavReader, WavSpec, WavWriter};
use rubato::{FftFixedIn, Resampler};
use thiserror::Error;

pub const VOLCA_SAMPLERATE: u32 = 31250;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("unsupported format {1}bit {0:?}")]
    Format(SampleFormat, u16),
    #[error("read WAV error: {0}")]
    Hound(#[from] hound::Error),
    #[error("could not build resampler: {0}")]
    ResamplerBuild(#[from] rubato::ResamplerConstructionError),
    #[error("resample error: {0}")]
    Resample(#[from] rubato::ResampleError),
}

pub type Result<T> = std::result::Result<T, AudioError>;
pub type AudioItem = WavResult<f64>;

#[derive(Debug, Display, Clone, ValueEnum, Default)]
pub enum MonoMode {
    Left,
    Right,
    #[default]
    Mid,
    Side,
    // Channel(u16),
}

pub fn write_sample_to_file(sample_data: &[i16], path: &Path) -> WavResult<()> {
    let length = sample_data.len() as u32;
    let header = WavSpec {
        channels: 1,
        sample_rate: VOLCA_SAMPLERATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open(path)?;
    let mut writer = WavWriter::new(file, header)?;
    let mut writer = writer.get_i16_writer(length);

    for sample in sample_data {
        writer.write_sample(*sample);
    }
    writer.flush()
}

pub struct AudioReader<'a, I> {
    reader: I,
    spec: WavSpec,
    path: &'a Path,
    duration: u32,
}

impl AudioReader<'_, ()> {
    #[auto_enum]
    pub fn open_file(path: &Path) -> Result<AudioReader<'_, impl Iterator<Item = AudioItem>>> {
        let reader = WavReader::open(path)?;
        let spec = reader.spec();
        let duration = reader.duration();
        let reader = into_samples_f64(reader)?;
        let duration_secs = Duration::from_secs_f64(duration as f64 / spec.sample_rate as f64);

        tracing::debug!(
            ?path,
            sample_rate = spec.sample_rate,
            num_channels = spec.channels,
            sample_format = ?spec.sample_format,
            sample_depth = spec.bits_per_sample,
            duration_samples = duration,
            duration = %humantime::format_duration(duration_secs),
            "opened file"
        );
        Ok(AudioReader {
            reader,
            spec,
            path,
            duration,
        })
    }
}

impl<I> AudioReader<'_, I> {
    pub fn channels(&self) -> u16 {
        self.spec.channels
    }
}

// TODO: statically forbid calling these methods more than once
impl<'a, I> AudioReader<'a, I>
where
    I: Iterator<Item = AudioItem>,
{
    pub fn take_channel(self, channel: u8) -> AudioReader<'a, impl Iterator<Item = AudioItem>> {
        tracing::debug!(path = ?self.path, channel, "filtering channel");
        let channels = self.spec.channels;
        let reader = self
            .reader
            .enumerate()
            .filter(move |(idx, _)| idx % channels as usize == channel as usize)
            .map(|(_, sample)| sample);

        AudioReader {
            reader,
            spec: self.spec,
            path: self.path,
            duration: self.duration,
        }
    }

    fn lr_transform<F>(self, mut f: F) -> AudioReader<'a, impl Iterator<Item = AudioItem>>
    where
        F: FnMut(f64, f64) -> f64,
    {
        assert!(self.spec.channels > 1);
        let channels = self.spec.channels;
        let reader = self
            .reader
            .enumerate()
            .filter(move |(idx, _)| idx % (channels as usize) < 2)
            .map(|(_, sample)| sample)
            .scan(None, move |left, sample| {
                let sample = match sample {
                    Ok(sample) => sample,
                    Err(err) => return Some(Some(Err(err))),
                };
                // Outer option must always be `Some` for the iterator to be polled
                Some(lr_scanner(left, sample, &mut f).map(Ok))
            })
            .flatten();

        AudioReader {
            reader,
            spec: self.spec,
            path: self.path,
            duration: self.duration,
        }
    }

    pub fn take_mid(self) -> AudioReader<'a, impl Iterator<Item = AudioItem>> {
        tracing::debug!(path = ?self.path, "filtering mid");
        self.lr_transform(|l, r| (l + r) / 2.)
    }

    pub fn take_side(self) -> AudioReader<'a, impl Iterator<Item = AudioItem>> {
        tracing::debug!(path = ?self.path, "filtering side");
        self.lr_transform(|l, r| (l - r) / 2.)
    }

    pub fn resample_to_volca(self) -> Result<Vec<i16>> {
        if self.spec.sample_rate == VOLCA_SAMPLERATE {
            // TODO: optimize this
            tracing::debug!("skipping resampling");
            self.reader
                .map(|result| result.map(float_to_i16))
                .collect::<WavResult<Vec<_>>>()
                .map_err(Into::into)
        } else {
            let original = self.reader.collect::<WavResult<Vec<_>>>()?;
            let mut resampler = FftFixedIn::new(
                self.spec.sample_rate as usize,
                VOLCA_SAMPLERATE as usize,
                self.duration as usize,
                self.duration as usize,
                1,
            )?;
            let result = resampler.process(&[original], None)?.pop().unwrap();
            Ok(result
                .into_iter()
                .map(|sample| (sample * i16::MAX as f64).round() as i16)
                .collect())
        }
    }
}

fn float_to_i16(sample: f64) -> i16 {
    (sample * i16::MAX as f64).round() as i16
}

/// Scan function that applies binary operation to left and right channel for each frame.
/// Returns None for items that must be skipped (to use with `flatten` combinator).
fn lr_scanner<F>(left: &mut Option<f64>, sample: f64, mut f: F) -> Option<f64>
where
    F: FnMut(f64, f64) -> f64,
{
    match left.take() {
        // If left is not empty then the `sample` is from the right channel
        Some(left) => Some(f(left, sample)),
        // If left is empty then the `sample` is from the left channel
        None => {
            *left = Some(sample);
            None
        }
    }
}

#[auto_enum]
fn into_samples_f64<R>(reader: WavReader<R>) -> Result<impl Iterator<Item = WavResult<f64>>>
where
    R: io::Read,
{
    let spec = reader.spec();

    #[auto_enum(Iterator)]
    let iter = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Int, 8) => reader
            .into_samples::<i8>()
            .map(|res| res.map(IntSample::normalize_to_f64)),
        (SampleFormat::Int, 16) => reader
            .into_samples::<i16>()
            .map(|res| res.map(IntSample::normalize_to_f64)),
        (SampleFormat::Int, n) if n <= 32 => reader
            .into_samples::<i32>()
            .map(|res| res.map(IntSample::normalize_to_f64)),
        (SampleFormat::Float, 32) => reader.into_samples::<f32>().map(|res| res.map(Into::into)),
        // (SampleFormat::Float, 64) => reader.into_samples::<f32>(),
        (format, bits) => return Err(AudioError::Format(format, bits)),
    };

    Ok(iter)
}

trait IntSample: Into<f64> {
    const MAX: Self;

    fn normalize_to_f64(self) -> f64 {
        self.into() / Self::MAX.into()
    }
}

macro_rules! impl_int_sample {
    ($($ty:ty),*) => {$(
        impl IntSample for $ty {
            const MAX: $ty = <$ty>::MAX;
        }
    )*}
}
impl_int_sample![i8, i16, i32];
