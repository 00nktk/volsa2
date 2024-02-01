//! Handles serialization and deserialization of backup structs
use serde::{Serialize, Deserialize};
use super::sample_slots::SampleSlots;

#[derive(Serialize, Deserialize)]
pub struct BackupData {
    pub sample_slots: SampleSlots,
}

impl BackupData {
    pub fn new() -> Self { Self { sample_slots: SampleSlots::new() } }
}
