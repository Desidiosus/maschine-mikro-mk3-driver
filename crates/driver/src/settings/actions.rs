use maschine_library::lights::PadColors;
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

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SliderLedMode {
    #[default]
    Bar,
    Pan,
    Dot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliderLedSettings {
    pub mode: SliderLedMode,
    pub color: PadColors,
    pub stylized: bool,
}

impl Default for SliderLedSettings {
    fn default() -> Self {
        Self {
            mode: SliderLedMode::Bar,
            color: PadColors::White,
            stylized: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SliderConfig {
    pub position: SliderPositionAction,
    pub touch: SliderTouchAction,
    pub led: SliderLedSettings,
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

    #[test]
    fn slider_led_mode_pan_round_trips_as_lowercase() {
        let led = SliderLedSettings {
            mode: SliderLedMode::Pan,
            color: maschine_library::lights::PadColors::Cyan,
            stylized: true,
        };
        let s = toml::to_string(&led).unwrap();
        assert!(s.contains("mode = \"pan\""), "got: {s}");
        assert!(s.contains("color = \"cyan\""), "got: {s}");
        let back: SliderLedSettings = toml::from_str(&s).unwrap();
        assert_eq!(back, led);
    }
}
