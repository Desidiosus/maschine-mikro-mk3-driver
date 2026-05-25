use maschine_library::lights::PadColors;
use serde::{Deserialize, Serialize};

use crate::settings::MidiChannel;

fn default_lo() -> u8 {
    0
}
fn default_hi() -> u8 {
    127
}
fn default_step() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CcValueMode {
    Absolute {
        #[serde(default = "default_lo")]
        lo: u8,
        #[serde(default = "default_hi")]
        hi: u8,
        #[serde(default = "default_step")]
        step: u8,
        #[serde(default)]
        wrap: bool,
    },
    Relative {
        #[serde(default = "default_step")]
        step: u8,
    },
    RelativeOffset {
        #[serde(default = "default_step")]
        step: u8,
    },
}

impl Default for CcValueMode {
    fn default() -> Self {
        Self::Relative { step: 1 }
    }
}

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
    /// Milliseconds after slider release before LEDs blank. `0` disables.
    pub auto_off_ms: u64,
}

impl Default for SliderLedSettings {
    fn default() -> Self {
        Self {
            mode: SliderLedMode::Bar,
            color: PadColors::White,
            stylized: false,
            auto_off_ms: 5000,
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
            auto_off_ms: 1234,
        };
        let s = toml::to_string(&led).unwrap();
        assert!(s.contains("mode = \"pan\""), "got: {s}");
        assert!(s.contains("color = \"cyan\""), "got: {s}");
        assert!(s.contains("auto_off_ms = 1234"), "got: {s}");
        let back: SliderLedSettings = toml::from_str(&s).unwrap();
        assert_eq!(back, led);
    }

    #[test]
    fn cc_value_mode_relative_round_trips_lowercase_kind() {
        let m = CcValueMode::Relative { step: 1 };
        let s = toml::to_string(&m).unwrap();
        assert!(s.contains("kind = \"relative\""), "got: {s}");
        let back: CcValueMode = toml::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn cc_value_mode_relative_offset_round_trips() {
        let m = CcValueMode::RelativeOffset { step: 2 };
        let s = toml::to_string(&m).unwrap();
        assert!(s.contains("kind = \"relative_offset\""), "got: {s}");
        let back: CcValueMode = toml::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn cc_value_mode_absolute_round_trips_with_explicit_fields() {
        let m = CcValueMode::Absolute {
            lo: 10,
            hi: 100,
            step: 3,
            wrap: true,
        };
        let s = toml::to_string(&m).unwrap();
        assert!(s.contains("kind = \"absolute\""), "got: {s}");
        let back: CcValueMode = toml::from_str(&s).unwrap();
        assert_eq!(back, m);
    }

    #[test]
    fn cc_value_mode_absolute_fills_defaults_when_fields_omitted() {
        let toml_str = r#"
kind = "absolute"
"#;
        let m: CcValueMode = toml::from_str(toml_str).unwrap();
        assert_eq!(
            m,
            CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step: 1,
                wrap: false,
            }
        );
    }
}
