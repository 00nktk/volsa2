mod proto;
mod seven_bit;
mod util;

use std::ffi::CString;
use std::fmt::Debug;

use alsa::seq::{self, ClientInfo};
use anyhow::{anyhow, Context, Result};
use bytemuck::cast_slice;
use hound::{SampleFormat, WavSpec, WavWriter};
use seven_bit::U7;
use smallvec::SmallVec;
use tracing::{debug, trace};

use crate::util::{binbuf, hexbuf, Bin, Hex};

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
}

impl State {
    fn new() -> Result<Self> {
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
        seq.set_client_pool_output(1024)?;
        seq.set_client_pool_output_room(1024)?;
        seq.set_client_pool_input(10240)?;

        seq.create_port(&me)?;

        let volca = find_volca(&seq)?;
        let me = me.addr();

        Ok(Self { me, seq, volca })
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

        let mut event = seq::Event::new_ext(seq::EventType::Sysex, buf.as_slice());

        debug!(?msg, raw = ?hexbuf(&buf), len = buf.len(), "send");
        trace!(bin = ?binbuf(&buf), "send");

        // event.set_source(self.me.port);
        event.set_direct();
        event.set_dest(self.volca);
        // event.set_priority(true); // ?: Do I need this?
        // event.set_subs();
        // event.schedule_tick(253, true, 1);
        // event.schedule_real(253, true, Duration::from_secs(1));

        self.seq.event_output_direct(&mut event)?;

        // self.seq.sync_output_queue()?;
        // self.seq.drain_output()?;
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
                owned_data
                    .as_mut()
                    .expect("replaced earlier")
                    .extend(new_data);
            }
            data = owned_data.as_ref().expect("replaced");
        }

        let data = &data;
        let msg = T::parse(data).map_err(Into::into);
        debug!(?msg, raw = ?cast_slice::<_, Hex>(data), len = data.len(), "recv");
        trace!(binary = ?cast_slice::<_, Bin>(data), "recv");
        msg
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let state = State::new()?;
    state.connect()?;

    state.send(
        proto::ExtendedKorgSysEx::new(0),
        proto::SampleSpaceDumpRequest,
    )?;
    let (_, response) = state.receive::<proto::SampleSpaceDump>()?;
    println!("Occupied space: {:.1}%", response.occupied() * 100.);

    for i in 0..200 {
        state.send(
            proto::ExtendedKorgSysEx::new(0),
            proto::SampleHeaderDumpRequest { sample_no: i },
        )?;
        let (_, response) = state.receive::<proto::SampleHeader>()?;
        if !response.is_empty() {
            println!(
                "{i:3}: {:24} - length: {:8}, speed: {:5}, level: {:5}",
                response.name, response.length, response.speed, response.level
            );
        }
    }

    let sample_no: u8 = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow!("sample no not provided"))?
        .parse()
        .context("parse sample no")?;
    state.send(
        proto::ExtendedKorgSysEx::new(0),
        proto::SampleHeaderDumpRequest { sample_no },
    )?;
    let (_, response) = state.receive::<proto::SampleHeader>()?;
    let length = response.length;
    state.send(
        proto::ExtendedKorgSysEx::new(0),
        proto::SampleDataDumpRequest { sample_no },
    )?;
    let (_, sample_data) = state.receive::<proto::SampleData>()?;

    let header = WavSpec {
        channels: 1,
        sample_rate: 31250,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .create(true)
        .open("./sample.wav")?;
    let mut writer = WavWriter::new(file, header)?;
    let mut writer = writer.get_i16_writer(length);

    for sample in sample_data.data {
        writer.write_sample(sample);
    }
    writer.flush()?;

    Ok(())
}
