//! Domain model structs
use serde::ser::{Serialize, Serializer, SerializeMap};

extern crate serde_yaml;

pub struct SampleMemoryBackup 
{
    pub sample_memory: [Option<String>; 200],
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


#[cfg(test)]
mod tests {
    use super::*;

    const EMPTY_SAMPLE: Option<String> = Option::None;

    #[test]
    fn serialize_samples_backup() {
        let mut samples_backup = SampleMemoryBackup {
            sample_memory: [EMPTY_SAMPLE; 200], 
        };
        samples_backup.sample_memory[0] = Some(String::from("Hello1")); 
        samples_backup.sample_memory[2] = Some(String::from("Hello3"));

        let s = serde_yaml::to_string(&samples_backup).unwrap();
        assert_eq!(s, "0: Hello1\n2: Hello3\n");
    }
}

