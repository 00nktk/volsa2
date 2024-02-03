//! Handles backups of the volca's sample memory slot layout
use serde::de::{Deserialize, Deserializer, MapAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, Serializer};
use std::fmt;
use std::ops::{Index, IndexMut};

extern crate serde_yaml;

// This is used instead of a HashMap to enforce a fixed set of keys and
// customize serialization behaviour for readability
pub struct SampleSlots {
	slots: [Option<String>; 200],
}

impl SampleSlots {
	const EMPTY_SAMPLE: Option<String> = Option::None;

	pub fn new() -> Self {
		Self {
			slots: [Self::EMPTY_SAMPLE; 200],
		}
	}
	pub fn len(&self) -> usize {
		self.slots.len()
	}
}

impl Index<usize> for SampleSlots {
	type Output = Option<String>;

	fn index(&self, index: usize) -> &Self::Output {
		return &self.slots[index as usize];
	}
}

impl IndexMut<usize> for SampleSlots {
	fn index_mut(&mut self, index: usize) -> &mut Self::Output {
		return &mut self.slots[index as usize];
	}
}

impl Serialize for SampleSlots {
	fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
	where
		S: Serializer,
	{
		let mut map = serializer.serialize_map(Some(self.slots.len()))?;
		for i in 0..self.slots.len() {
			if self.slots[i].is_some() {
				map.serialize_entry(&i, &self.slots[i])?;
			}
		}
		map.end()
	}
}

struct SampleSlotsVisitor;

impl SampleSlotsVisitor {
	fn new() -> Self {
		SampleSlotsVisitor {}
	}
}

impl<'de> Visitor<'de> for SampleSlotsVisitor {
	// The type that our Visitor is going to produce.
	type Value = SampleSlots;

	// Format a message stating what data this Visitor expects to receive.
	fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
		formatter.write_str("a map of sample memory locations to filenames without file extensions")
	}

	// Deserialize MyMap from an abstract "map" provided by the
	// Deserializer. The MapAccess input is a callback provided by
	// the Deserializer to let us see each entry in the map.
	fn visit_map<M>(self, mut access: M) -> Result<Self::Value, M::Error>
	where
		M: MapAccess<'de>,
	{
		let mut backup = SampleSlots::new();

		// While there are entries remaining in the input, add them
		// into our map.
		while let Some((key, value)) = access.next_entry::<usize, String>()? {
			backup.slots[key] = Some(value);
		}

		Ok(backup)
	}
}

// This is the trait that informs Serde how to deserialize MyMap.
impl<'de> Deserialize<'de> for SampleSlots {
	fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
	where
		D: Deserializer<'de>,
	{
		// Instantiate our Visitor and ask the Deserializer to drive
		// it over the input data, resulting in an instance of MyMap.
		deserializer.deserialize_map(SampleSlotsVisitor::new())
	}
}

// tests use yaml at the moment but could be made generic to support other
// (de)serializers
#[cfg(test)]
mod tests {
	use super::*;

	const EXPECTED_YAML: &str = "0: Hello1\n2: Hello3\n";

	#[test]
	fn serialize_samples_backup() {
		let mut samples_backup = SampleSlots::new();
		samples_backup.slots[0] = Some(String::from("Hello1"));
		samples_backup.slots[2] = Some(String::from("Hello3"));

		let s = serde_yaml::to_string(&samples_backup).unwrap();
		assert_eq!(EXPECTED_YAML, s);
	}

	#[test]
	fn deserialize_samples_backup() {
		let samples_backup: SampleSlots =
			serde_yaml::from_str(EXPECTED_YAML).unwrap();

		assert_eq!("Hello1", samples_backup.slots[0].as_ref().unwrap());
		assert_eq!(None, samples_backup.slots[1]);
		assert_eq!("Hello3", samples_backup.slots[2].as_ref().unwrap());
		for i in 3..200 {
			assert_eq!(None, samples_backup.slots[i]);
		}
	}
}
