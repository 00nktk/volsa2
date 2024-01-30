//! Domain model structs
use std::fmt;
use serde::ser::{Serialize, Serializer, SerializeMap};
use serde::de::{Deserialize, Deserializer, Visitor, MapAccess};

extern crate serde_yaml;

pub struct SampleMemoryBackup {
    pub sample_memory: [Option<String>; 200],
}

impl SampleMemoryBackup {
    const EMPTY_SAMPLE: Option<String> = Option::None;
    
    pub fn new() -> Self { Self { sample_memory: [Self::EMPTY_SAMPLE; 200] } }
}

impl Serialize for SampleMemoryBackup 
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer
    {
        let mut map = serializer.serialize_map(Some(self.sample_memory.len()))?;
        for i in 0..self.sample_memory.len() {
            if self.sample_memory[i].is_some() {
                map.serialize_entry(&i, &self.sample_memory[i])?;
            }
        }
        map.end()
    }
}

struct SampleMemoryBackupVisitor;

impl SampleMemoryBackupVisitor {
    fn new() -> Self {
        SampleMemoryBackupVisitor {}
    }
}

impl<'de> Visitor<'de> for SampleMemoryBackupVisitor
{
    // The type that our Visitor is going to produce.
    type Value = SampleMemoryBackup;

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
        let mut backup = SampleMemoryBackup::new();

        // While there are entries remaining in the input, add them
        // into our map.
        while let Some((key, value)) = access.next_entry::<usize,String>()? {
            backup.sample_memory[key] = Some(value);
        }

        Ok(backup)
    }
}

// This is the trait that informs Serde how to deserialize MyMap.
impl<'de> Deserialize<'de> for SampleMemoryBackup
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Instantiate our Visitor and ask the Deserializer to drive
        // it over the input data, resulting in an instance of MyMap.
        deserializer.deserialize_map(SampleMemoryBackupVisitor::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXPECTED_YAML: &str = "0: Hello1\n2: Hello3\n";

    #[test]
    fn serialize_samples_backup() {
        let mut samples_backup = SampleMemoryBackup::new();
        samples_backup.sample_memory[0] = Some(String::from("Hello1")); 
        samples_backup.sample_memory[2] = Some(String::from("Hello3"));

        let s = serde_yaml::to_string(&samples_backup).unwrap();
        assert_eq!(EXPECTED_YAML, s);
    }

    #[test]
    fn deserialize_samples_backup() {
        let samples_backup: SampleMemoryBackup = serde_yaml::from_str(EXPECTED_YAML).unwrap();

        assert_eq!("Hello1", samples_backup.sample_memory[0].as_ref().unwrap());
        assert_eq!(None, samples_backup.sample_memory[1]);
        assert_eq!("Hello3", samples_backup.sample_memory[2].as_ref().unwrap());
        for i in 3..200 {
            assert_eq!(None, samples_backup.sample_memory[i]);
        }
    }
}
 
