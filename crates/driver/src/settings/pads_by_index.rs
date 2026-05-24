use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::settings::actions::PadConfig;

const PAD_COUNT: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PadsByIndex(pub [PadConfig; PAD_COUNT]);

impl PadsByIndex {
    pub fn new(pads: [PadConfig; PAD_COUNT]) -> Self {
        Self(pads)
    }

    pub fn as_array(&self) -> &[PadConfig; PAD_COUNT] {
        &self.0
    }

    pub fn as_array_mut(&mut self) -> &mut [PadConfig; PAD_COUNT] {
        &mut self.0
    }

    pub fn iter(&self) -> std::slice::Iter<'_, PadConfig> {
        self.0.iter()
    }
}

impl Index<usize> for PadsByIndex {
    type Output = PadConfig;
    fn index(&self, idx: usize) -> &PadConfig {
        &self.0[idx]
    }
}

impl IndexMut<usize> for PadsByIndex {
    fn index_mut(&mut self, idx: usize) -> &mut PadConfig {
        &mut self.0[idx]
    }
}

impl Serialize for PadsByIndex {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = BTreeMap::new();
        for (idx, pad) in self.0.iter().enumerate() {
            // Stringify the key so TOML emits "0", "1", ... style sections
            // matching what PartialSettings::deserialize_partial_pads expects.
            map.insert(idx.to_string(), pad);
        }
        map.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PadsByIndex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map: BTreeMap<String, PadConfig> = BTreeMap::deserialize(deserializer)?;

        if map.len() != PAD_COUNT {
            return Err(DeError::custom(format!(
                "expected {PAD_COUNT} pad entries, got {}",
                map.len()
            )));
        }

        let mut out: [Option<PadConfig>; PAD_COUNT] = std::array::from_fn(|_| None);
        for (key, cfg) in map {
            let idx: usize = key.parse().map_err(|_| {
                DeError::custom(format!("pad key must be an integer 0..=15, got {key:?}"))
            })?;
            if idx >= PAD_COUNT {
                return Err(DeError::custom(format!(
                    "pad index {idx} out of range 0..=15"
                )));
            }
            out[idx] = Some(cfg);
        }
        let collected: [PadConfig; PAD_COUNT] = std::array::from_fn(|i| {
            out[i]
                .clone()
                .expect("len check guarantees all slots filled")
        });
        Ok(PadsByIndex(collected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::actions::{PadHitAction, PadPressureAction};

    fn make_pad(note: u8) -> PadConfig {
        PadConfig {
            hit: PadHitAction::Note {
                channel: None,
                note,
            },
            pressure: PadPressureAction::Disabled,
        }
    }

    fn make_pads() -> PadsByIndex {
        PadsByIndex(std::array::from_fn(|i| make_pad(48 + i as u8)))
    }

    #[test]
    fn pads_round_trip_with_integer_string_keys() {
        let pads = make_pads();
        let s = toml::to_string(&pads).unwrap();
        let back: PadsByIndex = toml::from_str(&s).unwrap();
        assert_eq!(back, pads);
    }

    #[test]
    fn serializes_with_integer_keyed_sections() {
        let pads = make_pads();
        let s = toml::to_string(&pads).unwrap();
        // toml-rs may quote string keys that start with a digit
        assert!(
            s.contains("[0.hit]") || s.contains("[\"0\".hit]") || s.contains("[0]"),
            "got:\n{s}"
        );
    }
}
