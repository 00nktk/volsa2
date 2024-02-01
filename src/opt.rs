use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::audio::MonoMode;

#[derive(Parser)]
/// Korg Volca Sample CLI.
pub struct Opts {
    #[command(subcommand)]
    pub cmd: Operation,
    /// Interval duration to wait before sending a new chunk.
    ///
    /// Volca Sample 2 can hang when receiving long messages (SampleDataDump specifically).
    /// We introduce a "cooldown" for sending a chunk to avoid this.
    #[arg(short, long, default_value = "10ms")]
    pub chunk_cooldown: humantime::Duration,
}

#[derive(Subcommand)]
pub enum Operation {
    /// List samples loaded into the device.
    #[command(alias = "ls")]
    List {
        /// Print empty sample slots in the output.
        #[arg(short = 'a', long, default_value = "false")]
        show_empty: bool,
    },
    /// Save the sample memory layout to a YAML file
    #[command(alias = "lay")]
    Layout {
        /// Output path
        output: PathBuf,
    },
    /// Backup entire sample memory to a given folder
    #[command(alias = "bk")]
    Backup {
        /// Output folder path
        output: PathBuf,
    },
    /// Download a sample from the device.
    #[command(alias = "dl")]
    Download {
        /// Sample ID as shown in the device "sample" menu or in the output of List command.
        sample_no: u8, // TODO: bound this
        /// Output path. Sample name will be used if the provided path points to a directory.
        #[arg(short, long, default_value = "./")]
        output: PathBuf,
    },
    /// Load sample into the device.
    #[command(alias = "up")]
    Upload {
        /// Path to audio file to upload.
        file: PathBuf,
        /// Sample slot number. Will choose first empty slot if not provided.
        sample_no: Option<u8>,
        /// Mono convertion mode.
        #[arg(short, long, value_enum, default_value_t = MonoMode::Mid)]
        mono_mode: MonoMode,
        /// Converted audio output path.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Do not upload the sample after convertion.
        #[arg(long, default_value = "false")]
        dry_run: bool,
    },
    /// Erase sample from device memory
    #[command(alias = "rm")]
    Remove {
        /// Sample slot number.
        sample_no: u8,
        /// Print sample name.
        #[arg(short, long, default_value = "false")]
        print_name: bool,
    },
}
