use serde::{Deserialize, Serialize};

use crate::settings::MidiChannel;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PadHitAction {
    Note {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        note: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum PadPressureAction {
    Disabled,
    Poly {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadConfig {
    pub hit: PadHitAction,
    pub pressure: PadPressureAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ButtonPressAction {
    Cc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        cc: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ButtonConfig {
    pub press: ButtonPressAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EncoderTurnAction {
    Cc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        cc: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncoderConfig {
    pub turn: EncoderTurnAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SliderPositionAction {
    Cc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        cc: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SliderTouchAction {
    Disabled,
    Note {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        note: u8,
        on_value: u8,
        off_value: u8,
    },
    Cc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        cc: u8,
        on_value: u8,
        off_value: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliderConfig {
    pub position: SliderPositionAction,
    pub touch: SliderTouchAction,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_pressure_disabled_round_trips_as_disabled_string() {
        let action = PadPressureAction::Disabled;
        let s = toml::to_string(&action).unwrap();
        assert!(s.contains("type = \"disabled\""), "got: {s}");
        let back: PadPressureAction = toml::from_str(&s).unwrap();
        assert_eq!(back, PadPressureAction::Disabled);
    }
}
