use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};

use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::settings::actions::PadConfig;

const PAD_COUNT: usize = 16;

/// TOML key for `[pads.N]` is the physical pad number labelled on the device
/// (1 = bottom-right, 16 = top-left). Internal indexing keeps the device's
/// native byte ordering (0..=15). Map between the two.
pub(crate) const fn config_key_to_internal(toml_key: usize) -> usize {
    PAD_COUNT - toml_key
}

pub(crate) const fn internal_to_config_key(internal: usize) -> usize {
    PAD_COUNT - internal
}

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
        for (internal, pad) in self.0.iter().enumerate() {
            map.insert(internal_to_config_key(internal).to_string(), pad);
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
            let config_key: usize = key.parse().map_err(|_| {
                DeError::custom(format!("pad key must be an integer 1..=16, got {key:?}"))
            })?;
            if !(1..=PAD_COUNT).contains(&config_key) {
                return Err(DeError::custom(format!(
                    "pad index {config_key} out of range 1..=16"
                )));
            }
            out[config_key_to_internal(config_key)] = Some(cfg);
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
            s.contains("[1.hit]") || s.contains("[\"1\".hit]") || s.contains("[1]"),
            "got:\n{s}"
        );
    }

    #[test]
    fn config_key_round_trip_through_internal() {
        for toml_key in 1..=PAD_COUNT {
            let internal = config_key_to_internal(toml_key);
            assert!(internal < PAD_COUNT);
            assert_eq!(internal_to_config_key(internal), toml_key);
        }
    }

    #[test]
    fn toml_key_1_deserializes_to_internal_index_15() {
        let mut full: BTreeMap<String, PadConfig> = BTreeMap::new();
        full.insert("1".to_string(), make_pad(99));
        for n in 2..=PAD_COUNT {
            full.insert(n.to_string(), make_pad(48));
        }
        let pads: PadsByIndex = toml::from_str(&toml::to_string(&full).unwrap()).unwrap();
        match &pads[15].hit {
            PadHitAction::Note { note, .. } => assert_eq!(*note, 99),
        }
    }

    #[test]
    fn internal_index_zero_serializes_as_toml_key_16() {
        let mut pads = make_pads();
        pads.0[0] = make_pad(99);
        let s = toml::to_string(&pads).unwrap();
        // Internal index 0 carries note=99 → emitted under TOML key 16.
        assert!(
            s.contains("[16.hit]") || s.contains("[\"16\".hit]"),
            "expected pad at TOML key 16 to carry note=99\ngot:\n{s}"
        );
        let parsed: PadsByIndex = toml::from_str(&s).unwrap();
        assert_eq!(parsed, pads);
    }

    #[test]
    fn deserialize_rejects_toml_key_zero() {
        let mut full: BTreeMap<String, PadConfig> = BTreeMap::new();
        full.insert("0".to_string(), make_pad(99));
        for n in 1..PAD_COUNT {
            full.insert(n.to_string(), make_pad(48));
        }
        let serialized = toml::to_string(&full).unwrap();
        let err = toml::from_str::<PadsByIndex>(&serialized).unwrap_err();
        assert!(
            err.to_string().contains("out of range 1..=16"),
            "got: {err}"
        );
    }
}
