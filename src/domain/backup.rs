//! Handles serialization and deserialization of backup structs
use super::sample_slots::SampleSlots;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct BackupData {
	pub sample_slots: SampleSlots,
}

impl BackupData {
	pub fn new() -> Self {
		Self {
			sample_slots: SampleSlots::new(),
		}
	}
}
