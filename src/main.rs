mod audio;
mod opt;
mod proto;
mod seven_bit;
mod util;

use std::any::type_name;
use std::borrow::Cow;
use std::ffi::CString;
use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use alsa::seq::{self, ClientInfo};
use anyhow::{anyhow, bail, Result};
use clap::Parser;
use seven_bit::U7;
use smallvec::SmallVec;
use tracing::{debug, trace, warn};

use crate::audio::{write_sample_to_file, AudioReader, MonoMode};
use crate::util::{hexbuf, DEBUG_TRESHOLD};

const SELF_NAME: &str = "VolSa2";

fn find_volca(seq: &seq::Seq) -> Result<seq::Addr> {
    let mut clients = seq::ClientIter::new(seq);

    let client: ClientInfo = clients
        .find(|client| {
            client
                .get_name()
                .ok()
                .filter(|&name| name == "volca sample")
                .is_some()
        })
        .ok_or_else(|| anyhow!("could not find volca sample"))?;

    let port = seq::PortIter::new(seq, client.get_client())
        .next()
        .ok_or_else(|| anyhow!("no port"))?;

    Ok(port.addr())
}

struct State {
    seq: seq::Seq,
    me: seq::Addr,
    volca: seq::Addr,
    chunk_cooldown: Duration,
}

impl State {
    fn new(chunk_cooldown: Duration) -> Result<Self> {
        let seq = seq::Seq::open(None, None, false)?;
        seq.set_client_name(&CString::new(SELF_NAME)?)?;
        let mut me = seq::PortInfo::empty()?;
        me.set_capability(
            seq::PortCap::WRITE
            | seq::PortCap::SUBS_WRITE
            | seq::PortCap::READ
            | seq::PortCap::SUBS_READ
            // | seq::PortCap::SYNC_READ
            // | seq::PortCap::SYNC_WRITE
            | seq::PortCap::DUPLEX,
        );
        me.set_type(seq::PortType::MIDI_GENERIC | seq::PortType::APPLICATION | seq::PortType::PORT);
        me.set_name(&CString::new(SELF_NAME)?);

        seq.create_port(&me)?;

        let volca = find_volca(&seq)?;
        let me = me.addr();

        Ok(Self {
            me,
            seq,
            volca,
            chunk_cooldown,
        })
    }

    fn connect(&self) -> Result<()> {
        let sub = seq::PortSubscribe::empty()?;
        sub.set_sender(self.volca);
        sub.set_dest(self.me);
        self.seq.subscribe_port(&sub)?;

        let sub = seq::PortSubscribe::empty()?;
        sub.set_sender(self.me);
        sub.set_dest(self.volca);
        self.seq.subscribe_port(&sub)?;

        let echo = U7::new(42);
        self.send(proto::KorgSysEx, proto::SearchDeviceRequest { echo })?;

        let _response = self.receive::<proto::SearchDeviceReply>()?;
        Ok(())
    }

    fn send<T>(&self, header: T::Header, msg: T) -> Result<()>
    where
        T: proto::Outgoing + Debug,
        T::Header: Debug,
    {
        let mut buf = SmallVec::<[u8; 6]>::new();
        msg.encode(header, &mut buf)?;

        if buf.len() > DEBUG_TRESHOLD {
            debug!(msg = type_name::<T>(), len = buf.len(), "send msg");
            trace!(?msg, raw = ?hexbuf(&buf), len = buf.len(), "send msg");
        } else {
            debug!(?msg, len = buf.len(), "send msg");
        }

        for slice in buf.chunks(256) {
            let mut event = seq::Event::new_ext(seq::EventType::Sysex, slice);

            trace!(len = slice.len(), raw = ?hexbuf(slice), "send chunk");

            event.set_source(self.me.port);
            event.set_direct();
            event.set_priority(true);
            event.set_dest(self.volca);

            self.seq.event_output_direct(&mut event)?;
            if !slice.ends_with(&[proto::EOX]) && !self.chunk_cooldown.is_zero() {
                std::thread::sleep(self.chunk_cooldown);
            }
        }
        self.seq.sync_output_queue()?;
        self.seq.drain_output()?;

        Ok(())
    }

    fn receive<T>(&self) -> Result<(T::Header, T)>
    where
        T: proto::Incoming + Debug,
        T::Header: Debug,
    {
        self.seq.set_client_pool_input(1024)?;
        let mut input = self.seq.input();

        macro_rules! next_event {
            () => {
                loop {
                    let event = input.event_input()?;
                    if event.get_type() == seq::EventType::Sysex
                        && event.get_source() == self.volca
                        && event.get_dest() == self.me
                    {
                        break event;
                    }
                }
            };
        }

        let event = next_event!();
        let mut owned_data = None;
        let mut data = event
            .get_ext()
            .ok_or_else(|| anyhow!("SysEx without data"))?;
        trace!(raw = ?hexbuf(data), len = data.len(), "recv fst chunk");

        #[allow(unused_assignments)]
        // TODO: Fix this
        if !data.ends_with(&[proto::EOX]) {
            owned_data.replace(data.to_vec());
            data = &[]; // Free input borrow

            while !owned_data
                .as_ref()
                .expect("replaced")
                .ends_with(&[proto::EOX])
            {
                let event = next_event!();
                let new_data = event
                    .get_ext()
                    .ok_or_else(|| anyhow!("SysEx without data"))?;
                trace!(raw = ?hexbuf(new_data), len = new_data.len(), "recv chunk");
                owned_data
                    .as_mut()
                    .expect("replaced earlier")
                    .extend(new_data);
            }
            data = owned_data.as_ref().expect("replaced");
        }

        let data = &data;
        let msg = T::parse(data).map_err(Into::into);
        if data.len() > DEBUG_TRESHOLD {
            debug!(msg = type_name::<T>(), len = data.len(), "recv msg");
            trace!(?msg, raw = ?hexbuf(data), "recv_msg");
        } else {
            debug!(?msg, raw = ?hexbuf(data), len = data.len(), "recv_msg");
        }
        msg
    }

    fn get_sample_header(&self, sample_no: u8) -> Result<proto::SampleHeader> {
        // TODO: restrict this in type
        if sample_no > 199 {
            bail!("sample_no must be less than 200");
        }

        self.send(
            proto::ExtendedKorgSysEx::new(0),
            proto::SampleHeaderDumpRequest { sample_no },
        )?;
        let (_, header) = self.receive::<proto::SampleHeader>()?;
        Ok(header)
    }

    fn get_sample(&self, sample_no: u8) -> Result<proto::SampleData> {
        // TODO: restrict this in type
        if sample_no > 199 {
            bail!("sample_no must be less than 200");
        }

        self.send(
            proto::ExtendedKorgSysEx::new(0),
            proto::SampleDataDumpRequest { sample_no },
        )?;
        let (_, sample_data) = self.receive::<proto::SampleData>()?;
        Ok(sample_data)
    }

    fn delete_sample(&self, sample_no: u8) -> Result<()> {
        // TODO: restrict this in type
        if sample_no > 199 {
            bail!("sample_no must be less than 200");
        }

        self.send(
            proto::ExtendedKorgSysEx::new(0),
            proto::SampleHeader::empty(sample_no),
        )?;
        self.receive::<proto::Status>()?.1?;
        Ok(())
    }

    fn send_sample(&self, header: proto::SampleHeader, data: proto::SampleData) -> Result<()> {
        self.send(proto::ExtendedKorgSysEx::new(0), header)?;
        self.receive::<proto::Status>()?.1?;
        self.send(proto::ExtendedKorgSysEx::new(0), data)?;
        self.receive::<proto::Status>()?.1?;
        Ok(())
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let opts = opt::Opts::parse();

    let state = State::new(opts.chunk_cooldown.into())?;
    state.connect()?;

    match opts.cmd {
        opt::Operation::List { show_empty } => {
            state.send(
                proto::ExtendedKorgSysEx::new(0),
                proto::SampleSpaceDumpRequest,
            )?;
            let (_, response) = state.receive::<proto::SampleSpaceDump>()?;
            println!("Occupied space: {:.1}%", response.occupied() * 100.);

            let mut last_printed = 0;
            for i in 0..200 {
                state.send(
                    proto::ExtendedKorgSysEx::new(0),
                    proto::SampleHeaderDumpRequest { sample_no: i },
                )?;
                let (_, response) = state.receive::<proto::SampleHeader>()?;
                if !response.is_empty() {
                    if show_empty {
                        for idx in (last_printed + 1)..response.sample_no {
                            println!("{idx:3}: <EMPTY>");
                        }
                    }
                    last_printed = response.sample_no;
                    println!(
                        "{i:3}: {:24} - length: {:8}, speed: {:5}, level: {:5}",
                        response.name, response.length, response.speed, response.level
                    );
                }
            }
        }
        opt::Operation::Download { sample_no, output } => {
            let header = state.get_sample_header(sample_no)?;
            println!(r#"Downloading sample "{}" from Volca"#, header.name);
            let sample_data = state.get_sample(sample_no)?;

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

            let current_header = state.get_sample_header(sample_no)?;
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
                    let sample = state.get_sample(sample_no)?;
                    let path = format!("./{}.wav", current_header.name);
                    write_sample_to_file(&sample.data, path.as_ref())?;
                    println!("Wrote backup to {path}");
                }
            }

            let (header, data) = proto::SampleData::new(sample_no, &name, sample);
            state.send_sample(header, data)?;
            println!("Loaded sample {name} in slot {sample_no}");
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
