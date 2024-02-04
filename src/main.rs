mod audio;
mod device;
mod domain;
mod opt;
mod proto;
mod seven_bit;
mod util;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use clap::Parser;

use crate::audio::{write_sample_to_file, AudioReader, MonoMode};
use crate::device::Device;
use crate::domain::BackupData;
use crate::util::{ask, extract_file_name, normalize_path};

struct App {
    chunk_cooldown: Duration,
    volca: Option<Device>,
}

impl App {
    fn new(chunk_cooldown: Duration) -> Self {
        Self {
            chunk_cooldown,
            volca: None,
        }
    }

    fn volca(&mut self) -> Result<&Device> {
        if self.volca.is_none() {
            let mut volca = Device::new(self.chunk_cooldown)?;
            volca.connect()?;
            self.volca.replace(volca);
        }

        Ok(self.volca.as_ref().unwrap())
    }

    fn list_samples(&mut self, show_empty: bool) -> Result<()> {
        let volca = self.volca()?;

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

        Ok(())
    }

    fn download_sample(&mut self, sample_no: u8, output: PathBuf, sample_type: &str) -> Result<()> {
        let volca = self.volca()?;

        let header = volca.get_sample_header(sample_no)?;
        println!(r#"Downloading sample "{}" from Volca"#, header.name);
        let sample_data = volca.get_sample(sample_no)?;

        Self::save_sample(&sample_data.data, &output, &header.name, sample_type)
    }

    fn upload_sample(
        &mut self,
        sample_no: Option<u8>,
        name: &str,
        data: Vec<i16>,
        check_overwrite: bool,
    ) -> Result<()> {
        let volca = self.volca()?;
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
        if !current_header.is_empty() && check_overwrite {
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
                self.download_sample(sample_no, "./".into(), "backup")?;
            }
        }

        let (header, data) = proto::SampleData::new(sample_no, name, data);
        self.volca()?.send_sample(header, data)?;
        println!("Loaded sample {name} in slot {sample_no}");

        Ok(())
    }

    fn upload_sample_from_file(
        &mut self,
        input: PathBuf,
        sample_no: Option<u8>,
        mono_mode: MonoMode,
        output: Option<PathBuf>,
        dry_run: bool,
        name: Option<&str>,
        check_overwrite: bool,
    ) -> Result<()> {
        let file_name = extract_file_name(&input)?;
        let name = name.unwrap_or(&file_name);
        let sample = Self::load_audio_file(&input, mono_mode)?;
        output
            .map(|path| Self::save_sample(&sample, &path, &name, "processed"))
            .transpose()?;

        if !dry_run {
            self.upload_sample(sample_no, &name, sample, check_overwrite)?;
        }

        Ok(())
    }

    fn delete_sample(&mut self, sample_no: u8, print_name: bool) -> Result<()> {
        let volca = self.volca()?;
        let name = if print_name {
            let mut header = volca.get_sample_header(sample_no)?;
            if header.is_empty() {
                println!("Sample {sample_no} is already empty");
                return Ok(());
            }

            header.name.push(' ');
            header.name
        } else {
            String::new()
        };

        volca.delete_sample(sample_no)?;
        println!("Removed sample {name}at slot {sample_no}");
        Ok(())
    }

    fn load_audio_file(path: &Path, mono_mode: MonoMode) -> Result<Vec<i16>> {
        let reader = AudioReader::open_file(path)?;
        let sample = match (reader.channels(), mono_mode) {
            (1, _) | (_, MonoMode::Left) => reader.take_channel(0).resample_to_volca()?,
            (_, MonoMode::Right) => reader.take_channel(1).resample_to_volca()?,
            (_, MonoMode::Mid) => reader.take_mid().resample_to_volca()?,
            (_, MonoMode::Side) => reader.take_side().resample_to_volca()?,
        };
        Ok(sample)
    }

    fn save_sample(data: &[i16], path: &Path, name: &str, sample_type: &str) -> Result<()> {
        let output = normalize_path(path, name, "wav")?;
        write_sample_to_file(data, &output)?;
        let space = if sample_type.is_empty() { "" } else { " " };
        println!("Wrote {sample_type}{space}sample to {output:?}");

        Ok(())
    }

    fn get_sample_memory_backup(&mut self) -> Result<BackupData> {
        let volca = self.volca()?;

        let mut backup = BackupData::new();

        for header in volca
            .iter_sample_headers()
            .filter(|res| res.as_ref().map_or(true, |header| !header.is_empty()))
        {
            let header = header?;
            backup.sample_slots[header.sample_no as usize] = Some(header.name);
        }

        Ok(backup)
    }

    fn download_backup_data(&mut self, output: PathBuf) -> Result<()> {
        let backup = self.get_sample_memory_backup()?;
        Self::save_backup_data(backup, output)
    }

    fn save_backup_data(backup: BackupData, output: PathBuf) -> Result<()> {
        let f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .open(&output)?;
        serde_yaml::to_writer(f, &backup)?;

        Ok(())
    }

    fn load_backup_data(input: &PathBuf) -> Result<BackupData> {
        // check extension to enforce yaml format
        let ext = match input.extension() {
            Some(ffi_str) => ffi_str.to_str().unwrap_or(""),
            None => "",
        };

        if ext != "yaml" {
            return Err(anyhow!(
                "Volsa2 currently only supports volca backups in Yaml format, \
                 input path was {ext}"
            ));
        }

        let f = fs::OpenOptions::new().read(true).open(&input)?;
        let backup: BackupData = serde_yaml::from_reader(f)?;

        Ok(backup)
    }

    fn backup(&mut self, output: PathBuf, sample_type: &str) -> Result<()> {
        let backup = self.get_sample_memory_backup()?;
        fs::create_dir_all(&output)?;

        let volca = self.volca()?;

        for i in 0..backup.sample_slots.len() {
            match &backup.sample_slots[i] {
                Some(slot) => {
                    println!(r#"Downloading sample "{}" from Volca"#, slot);
                    let sample_data = volca.get_sample(i as u8)?;
                    Self::save_sample(
                        &sample_data.data,
                        &output,
                        &format!("{slot}.wav"),
                        &sample_type,
                    )?;
                }
                None => {}
            }
        }

        let layout_filename = normalize_path(&output, "layout", "yaml")?;
        Self::save_backup_data(backup, layout_filename)
    }

    fn restore(&mut self, backup_data_path: PathBuf, dry_run: bool) -> Result<()> {
        if !dry_run {
            let question = "This will replace all samples on the device. Are you sure?";

            if !ask(&question)? {
                bail!("Restore cancelled");
            }
        }

        let backup = Self::load_backup_data(&backup_data_path)?;

        let parent_folder = backup_data_path.parent().unwrap();

        for i in 0..backup.sample_slots.len() {
            match &backup.sample_slots[i] {
                Some(sample_name) => {
                    if dry_run {
                        println!("{i:03} - {sample_name}");
                    }

                    let file_name = normalize_path(parent_folder, sample_name.as_str(), "wav")?;
                    self.upload_sample_from_file(
                        file_name,
                        Some(i as u8),
                        MonoMode::Mid,
                        None,
                        dry_run,
                        Some(sample_name.as_str()),
                        false, // already checked this for the restore operation
                    )?;
                }
                None => {
                    if dry_run {
                        println!("{i:03} - EMPTY");
                    } else {
                        self.delete_sample(i as u8, true)?;
                    }
                }
            }
        }

        return Ok(());
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let opts = opt::Opts::parse();
    let mut app = App::new(opts.chunk_cooldown.into());

    match opts.cmd {
        opt::Operation::List { show_empty } => app.list_samples(show_empty)?,
        opt::Operation::Download { sample_no, output } => {
            app.download_sample(sample_no, output, "")?
        }
        opt::Operation::Upload {
            sample_no,
            file,
            mono_mode,
            output,
            dry_run,
        } => {
            app.upload_sample_from_file(file, sample_no, mono_mode, output, dry_run, None, true)?
        }
        opt::Operation::Remove {
            sample_no,
            print_name,
        } => app.delete_sample(sample_no, print_name)?,
        opt::Operation::Layout { output } => app.download_backup_data(output)?,
        opt::Operation::Backup { output } => app.backup(output, "")?,
        opt::Operation::Restore { input, dry_run } => app.restore(input, dry_run)?,
    }

    Ok(())
}
