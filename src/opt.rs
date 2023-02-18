use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
/// Korg Volca Sample CLI.
pub struct Opts {
    #[command(subcommand)]
    pub cmd: Operation,
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
    /// Download a sample from the device.
    #[command(alias = "dl")]
    Download {
        /// Sample ID as shown in the device "sample" menu or in the output of List command.
        sample_no: u8, // TODO: bound this
        /// Output path. Sample name will be used if provided path points to directory.
        #[arg(short, long, default_value = "./")]
        output: PathBuf,
    },
}
