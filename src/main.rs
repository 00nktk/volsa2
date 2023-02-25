mod audio;
mod device;
mod opt;
mod proto;
mod seven_bit;
mod util;

use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use clap::Parser;
use tracing::warn;

use crate::audio::{write_sample_to_file, AudioReader, MonoMode};
use crate::device::Device;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let opts = opt::Opts::parse();

    let init_volca = || -> Result<Device> {
        let mut volca = Device::new(opts.chunk_cooldown.into())?;
        volca.connect()?;
        Ok(volca)
    };

    match opts.cmd {
        opt::Operation::List { show_empty } => {
            let volca = init_volca()?;

            volca.send(proto::SampleSpaceDumpRequest)?;
            let (_, response) = volca.receive::<proto::SampleSpaceDump>()?;
            println!("Occupied space: {:.1}%", response.occupied() * 100.);

            let mut last_printed = 0;
            for header in volca
                .iter_sample_headers()
                .filter(|res| res.as_ref().map_or(true, |header| !header.is_empty()))
            {
                let header = header?;
                if show_empty {
                    for idx in (last_printed + 1)..header.sample_no {
                        println!("{idx:3}: <EMPTY>");
                    }
                }
                last_printed = header.sample_no;
                println!(
                    "{:3}: {:24} - length: {:8}, speed: {:5}, level: {:5}",
                    header.sample_no, header.name, header.length, header.speed, header.level
                );
            }
        }
        opt::Operation::Download { sample_no, output } => {
            let volca = init_volca()?;

            let header = volca.get_sample_header(sample_no)?;
            println!(r#"Downloading sample "{}" from Volca"#, header.name);
            let sample_data = volca.get_sample(sample_no)?;

            let output = normalize_path(&output, &header.name);
            write_sample_to_file(&sample_data.data, &output)?;
            println!("Wrote sample to {output:?}");
        }
        opt::Operation::Upload {
            sample_no,
            file,
            mono_mode,
            output,
            dry_run,
        } => {
            let name = extract_file_name(&file)?;
            let reader = AudioReader::open_file(&file)?;
            let sample = match (reader.channels(), mono_mode) {
                (1, _) | (_, MonoMode::Left) => reader.take_channel(0).resample_to_volca()?,
                (_, MonoMode::Right) => reader.take_channel(1).resample_to_volca()?,
                (_, MonoMode::Mid) => reader.take_mid().resample_to_volca()?,
                (_, MonoMode::Side) => reader.take_side().resample_to_volca()?,
            };

            if let Some(output) = output {
                let output = normalize_path(&output, &name);
                if let Err(error) = write_sample_to_file(&sample, &output) {
                    warn!(%error, path = ?output, "could not save sample");
                } else {
                    println!("Saved processed sample to {output:?}");
                }
            }
            if dry_run {
                return Ok(());
            }

            let volca = init_volca()?;
            let sample_no = sample_no
                .map(Ok)
                .or_else(|| {
                    volca.iter_sample_headers().find_map(|result| {
                        result
                            .map(|header| header.is_empty().then_some(header.sample_no))
                            .transpose()
                    })
                })
                .ok_or_else(|| anyhow!("could not find empty slot"))??;

            let current_header = volca.get_sample_header(sample_no)?;
            if !current_header.is_empty() {
                // TODO: format_args?
                let question = format!(
                    "Sample slot is not empty (current - {}). Do you want to overwrite?",
                    current_header.name
                );
                if !ask(&question)? {
                    bail!("sample slot is not empty");
                }

                if ask(&format!(
                    "Do you want to backup the loaded sample ({})?",
                    current_header.name
                ))? {
                    let sample = volca.get_sample(sample_no)?;
                    let path = format!("./{}.wav", current_header.name);
                    write_sample_to_file(&sample.data, path.as_ref())?;
                    println!("Wrote backup to {path}");
                }
            }

            let (header, data) = proto::SampleData::new(sample_no, &name, sample);
            volca.send_sample(header, data)?;
            println!("Loaded sample {name} in slot {sample_no}");
        }
        opt::Operation::Remove {
            sample_no,
            print_name,
        } => {
            let volca = init_volca()?;
            let name = if print_name {
                let mut header = volca.get_sample_header(sample_no)?;
                if header.is_empty() {
                    println!("Sample is already empty");
                    return Ok(());
                }

                header.name.push(' ');
                header.name
            } else {
                String::new()
            };

            volca.delete_sample(sample_no)?;
            println!("Removed sample {name}at slot {sample_no}");
        }
    }

    Ok(())
}

fn extract_file_name(path: &Path) -> Result<Cow<'_, str>> {
    if !path.is_file() {
        bail!("path must point to a file: {path:?}")
    }

    path.file_stem()
        .map(|name| name.to_string_lossy())
        .ok_or_else(|| anyhow!("could not extract filename"))
}

fn ask(question: &str) -> io::Result<bool> {
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

fn normalize_path(path: &Path, filename: &str) -> PathBuf {
    let mut path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if path.is_dir() {
        path.set_file_name(filename);
        path.set_extension("wav");
    }
    path
}
