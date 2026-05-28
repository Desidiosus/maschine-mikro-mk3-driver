use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};

use maschine_library::controls::{BUTTON_NAMES, button_index_from_name};
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::settings::actions::ButtonConfig;

const BUTTON_COUNT: usize = 41;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ButtonsByName(pub [ButtonConfig; BUTTON_COUNT]);

impl ButtonsByName {
    pub fn filled_with(value: ButtonConfig) -> Self {
        Self(std::array::from_fn(|_| value.clone()))
    }

    pub fn as_array(&self) -> &[ButtonConfig; BUTTON_COUNT] {
        &self.0
    }

    pub fn as_array_mut(&mut self) -> &mut [ButtonConfig; BUTTON_COUNT] {
        &mut self.0
    }
}

impl Index<usize> for ButtonsByName {
    type Output = ButtonConfig;
    fn index(&self, idx: usize) -> &ButtonConfig {
        &self.0[idx]
    }
}

impl IndexMut<usize> for ButtonsByName {
    fn index_mut(&mut self, idx: usize) -> &mut ButtonConfig {
        &mut self.0[idx]
    }
}

impl Serialize for ButtonsByName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = BTreeMap::new();
        for (idx, name) in BUTTON_NAMES.iter().enumerate() {
            map.insert(*name, &self.0[idx]);
        }
        map.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ButtonsByName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let map: BTreeMap<String, ButtonConfig> = BTreeMap::deserialize(deserializer)?;

        if map.len() != BUTTON_COUNT {
            return Err(DeError::custom(format!(
                "expected {BUTTON_COUNT} button entries, got {}",
                map.len()
            )));
        }

        let mut out: [Option<ButtonConfig>; BUTTON_COUNT] = std::array::from_fn(|_| None);
        for (name, cfg) in map {
            let idx = button_index_from_name(&name)
                .ok_or_else(|| DeError::custom(format!("unknown button name: {name}")))?;
            out[idx] = Some(cfg);
        }
        let collected: [ButtonConfig; BUTTON_COUNT] = std::array::from_fn(|i| {
            out[i]
                .clone()
                .expect("len check guarantees all slots filled")
        });
        Ok(ButtonsByName(collected))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::actions::{ButtonConfig, ButtonPressAction};

    #[test]
    fn serializes_to_table_with_snake_case_keys() {
        let buttons = ButtonsByName::filled_with(ButtonConfig {
            press: ButtonPressAction::Cc {
                channel: None,
                cc: 0,
            },
        });
        let s = toml::to_string(&buttons).unwrap();
        assert!(
            s.contains("[play.press]"),
            "expected '[play.press]' in:\n{s}"
        );
        assert!(
            s.contains("[encoder_touch.press]"),
            "expected '[encoder_touch.press]' in:\n{s}"
        );
    }

    #[test]
    fn full_buttons_table_round_trips() {
        let buttons = ButtonsByName::filled_with(ButtonConfig {
            press: ButtonPressAction::Cc {
                channel: None,
                cc: 0,
            },
        });
        let s = toml::to_string(&buttons).unwrap();
        let back: ButtonsByName = toml::from_str(&s).unwrap();
        assert_eq!(back, buttons);
    }
}
