# VolSa 2

Volsa 2 is a simple command line sample manager for KORG **Vol**ca **Sa**mple **2** that works over ALSA MIDI sequencer. It can list, upload, download and remove samples via USB.

*This project is in early alpha stage. Use it at your own risk.*

There is also an Electron-based GUI available at [Turbnok/volsa2gui](https://github.com/Turbnok/volsa2gui).

## Installation

To build and install volsa2-cli you need alsa-lib and Rust 1.64.0 or higher. The most convenient
way is to use `cargo install` command:
```sh
cargo install volsa2-cli
```
This way the binary will end up in your `$HOME/.cargo/bin` (or `$CARGO_HOME/bin`). Make sure to add it to your `$PATH`.

Otherwise you can clone the repository and build it.
```sh
git clone https://github.com/00nktk/volsa2
cd volsa2
cargo build --release
```

## Usage
Use `--help` to print command description and available options.
```sh
volsa2-cli <command> --help
```

### List (`ls`)

```sh
volsa2-cli list
```
This command lists samples loaded into Volca Sample 2 memory. Use `-a`/`--show-empty` flag to include empty slots in the output.

### Download (`dl`)

```sh
volsa2-cli download <sample-no>
```
This will download sample from slot `<sample-no>`. You can specify output path via `-o`/`--output`. By default the sample is saved in the working directory named the same way as on the device.

### Upload (`up`)

```sh
volsa2-cli upload <path-to-sample> [<sample-no>]
```
Loads a sample from `<path-to-sample>` into `<sample-no>` slot. If no `<sample-no>` is specified, will use the first empty slot. Sample is converted to 31.25kHz mono. *Currently only WAV files are supported*.

Volsa2 will offer you to backup the sample if the desired slot is occupied.
##### Options:
- `-m`/`--mono-mode` - Lets you choose which channel to use as mono. Available options are: `left`, `right`, `mid`, `side`. Default is `mid` (mono mix).
- `-o`/`--output` - If specified, will save converted audio at the provided path. 
- `--dry-run` - Convert the sample, but do not load it into the device.

### Remove (`rm`)
```sh
volsa2-cli remove <sample-no>
```
Erases sample at slot `<sample-no>` from the device memory. Use `-p`/`--print-name` if you want to print the name of the sample.

### Backup (`bk`)
```sh 
volsa2-cli backup <backup-directory-path>
```
Creates a folder at `<backup-directory-path>` if one doesn't already exist, and dumps all samples from the Volca Sample 2 into it. Creates a file called layout.yaml in the folder that specifies which samples are to be inserted into which sample slots.

### Restore (`rs`)
```sh 
volsa2-cli restore <input-yaml-path>
```
Reads the backup data in the yaml file at `<input-yaml-path>`, and attempts to restore the Volca Sample 2 to the state specified in this yaml file. This means it will clear slots that are not specified in the yaml, and upload the samples that are specified. This expects the samples to be placed in the same directory as the yaml file, and to be named the same as specified in the yaml file but with a `.wav` extension.

For example, your yaml file might look like this:
```yaml
sample_slots:
    0: bd909
    1: bd808
    2: bd707
```
and your directory may look like this:
```
sample_backup/
|-bd909.wav
|-bd808.wav 
|-bd707.wav
```
When restored, all sample slots on the Volca Sample 2 will be cleared except for the first three which will contain the three `bdx0x.wav` samples.

##### Options:
- `--dry-run` - Check the behaviour without actually modifying anything on the device
