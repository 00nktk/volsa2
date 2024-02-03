use std::any::type_name;
use std::ffi::CString;
use std::fmt::Debug;
use std::time::Duration;

use alsa::seq::{self, ClientInfo};
use anyhow::{anyhow, bail, Result};
use smallvec::SmallVec;
use tracing::{debug, info, trace};

use crate::proto::{self, Header};
use crate::seven_bit::U7;
use crate::util::{hexbuf, DEBUG_TRESHOLD};

const SELF_NAME: &str = "VolSa2";

/// Represents connection to Volca.
pub struct Device {
	seq: seq::Seq,
	me: seq::Addr,
	volca: seq::Addr,
	channel: U7,
	chunk_cooldown: Duration,
}

impl Device {
	pub fn new(chunk_cooldown: Duration) -> Result<Self> {
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
		me.set_type(
			seq::PortType::MIDI_GENERIC
				| seq::PortType::APPLICATION
				| seq::PortType::PORT,
		);
		me.set_name(&CString::new(SELF_NAME)?);

		seq.create_port(&me)?;

		let volca = find_volca(&seq)?;
		let me = me.addr();

		Ok(Self {
			me,
			seq,
			volca,
			channel: U7::new(0),
			chunk_cooldown,
		})
	}

	pub fn connect(&mut self) -> Result<()> {
		let sub = seq::PortSubscribe::empty()?;
		sub.set_sender(self.volca);
		sub.set_dest(self.me);
		self.seq.subscribe_port(&sub)?;

		let sub = seq::PortSubscribe::empty()?;
		sub.set_sender(self.me);
		sub.set_dest(self.volca);
		self.seq.subscribe_port(&sub)?;

		let echo = U7::new(42);
		self.send(proto::SearchDeviceRequest { echo })?;

		let (_, response) = self.receive::<proto::SearchDeviceReply>()?;
		info!(
			global_channel = %response.device_id, version = %response.version,
			"connected to volca sample 2"
		);
		self.channel = response.device_id;
		Ok(())
	}

	pub fn send<T>(&self, msg: T) -> Result<()>
	where
		T: proto::Outgoing + Debug,
		T::Header: Debug,
	{
		let mut buf = SmallVec::<[u8; 6]>::new();
		let header = T::Header::from_channel(self.channel);
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
			if !slice.ends_with(&[proto::EOX])
				&& !self.chunk_cooldown.is_zero()
			{
				std::thread::sleep(self.chunk_cooldown);
			}
		}
		self.seq.sync_output_queue()?;
		self.seq.drain_output()?;

		Ok(())
	}

	pub fn receive<T>(&self) -> Result<(T::Header, T)>
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

	pub fn iter_sample_headers(
		&self,
	) -> impl Iterator<Item = Result<proto::SampleHeader>> + '_ {
		(0..200).map(|idx| {
			self.send(proto::SampleHeaderDumpRequest { sample_no: idx })?;
			let (_, response) = self.receive::<proto::SampleHeader>()?;
			Ok(response)
		})
	}

	pub fn get_sample_header(
		&self,
		sample_no: u8,
	) -> Result<proto::SampleHeader> {
		// TODO: restrict this in type
		if sample_no > 199 {
			bail!("sample_no must be less than 200");
		}

		self.send(proto::SampleHeaderDumpRequest { sample_no })?;
		let (_, header) = self.receive::<proto::SampleHeader>()?;
		Ok(header)
	}

	pub fn get_sample(&self, sample_no: u8) -> Result<proto::SampleData> {
		// TODO: restrict this in type
		if sample_no > 199 {
			bail!("sample_no must be less than 200");
		}

		self.send(proto::SampleDataDumpRequest { sample_no })?;
		let (_, sample_data) = self.receive::<proto::SampleData>()?;
		Ok(sample_data)
	}

	pub fn delete_sample(&self, sample_no: u8) -> Result<()> {
		// TODO: restrict this in type
		if sample_no > 199 {
			bail!("sample_no must be less than 200");
		}

		self.send(proto::SampleHeader::empty(sample_no))?;
		self.receive::<proto::Status>()?.1?;
		Ok(())
	}

	pub fn send_sample(
		&self,
		header: proto::SampleHeader,
		data: proto::SampleData,
	) -> Result<()> {
		self.send(header)?;
		self.receive::<proto::Status>()?.1?;
		self.send(data)?;
		self.receive::<proto::Status>()?.1?;
		Ok(())
	}
}

fn find_volca(seq: &seq::Seq) -> Result<seq::Addr> {
	let mut clients = seq::ClientIter::new(seq);

	let client: ClientInfo = clients
		.find(|client| {
			trace!(?client, "trying client");
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
