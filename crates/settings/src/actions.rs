use maschine_library::lights::{Brightness, PadColors, pad_color_from_velocity};
use serde::{Deserialize, Serialize};

use crate::MidiChannel;

fn default_lo() -> u8 {
    0
}
fn default_hi() -> u8 {
    127
}
fn default_step() -> i8 {
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
        step: i8,
        #[serde(default)]
        wrap: bool,
    },
    Relative {
        #[serde(default = "default_step")]
        step: i8,
    },
    RelativeOffset {
        #[serde(default = "default_step")]
        step: i8,
    },
}

impl CcValueMode {
    /// 7-bit MIDI CC data-byte range. `Absolute` `lo`/`hi` are bounded by it, and
    /// the encoder emit logic clamps relative-offset output to it.
    pub const CC_VALUE_MIN: u8 = 0;
    pub const CC_VALUE_MAX: u8 = 127;

    // Encoder `step` is signed: the sign sets turn direction, the magnitude sets
    // sensitivity, and `0` is invalid in every mode (it would freeze the encoder).

    /// `Absolute` and `RelativeOffset` clamp their *output* to the 7-bit CC range,
    /// so any nonzero step is valid and they span the full signed range.
    pub const ABSOLUTE_STEP_MIN: i8 = i8::MIN;
    pub const ABSOLUTE_STEP_MAX: i8 = i8::MAX;
    /// `Relative` emits the turn magnitude directly on the wire (NI sign-magnitude,
    /// magnitude up to 63), so its step is bounded to ±63.
    pub const RELATIVE_STEP_MIN: i8 = -63;
    pub const RELATIVE_STEP_MAX: i8 = 63;

    /// This mode's `step`.
    pub fn step(&self) -> i8 {
        match self {
            CcValueMode::Absolute { step, .. }
            | CcValueMode::Relative { step }
            | CcValueMode::RelativeOffset { step } => *step,
        }
    }

    /// Inclusive `[min, max]` bounds for this variant's `step`. Single source of
    /// truth shared by validation and the GUI clamping paths.
    pub fn step_bounds(&self) -> (i8, i8) {
        match self {
            CcValueMode::Relative { .. } => (Self::RELATIVE_STEP_MIN, Self::RELATIVE_STEP_MAX),
            CcValueMode::Absolute { .. } | CcValueMode::RelativeOffset { .. } => {
                (Self::ABSOLUTE_STEP_MIN, Self::ABSOLUTE_STEP_MAX)
            }
        }
    }

    /// Return this mode with its `step` clamped into the variant's valid range,
    /// coercing the invalid `0` (which would freeze the encoder) to `1` — the
    /// smallest forward step, which every variant's range includes — and
    /// preserving every other field.
    pub fn with_clamped_step(self) -> Self {
        let (min, max) = self.step_bounds();
        let step = match self.step().clamp(min, max) {
            0 => 1,
            v => v,
        };
        match self {
            Self::Absolute { lo, hi, wrap, .. } => Self::Absolute { lo, hi, step, wrap },
            Self::Relative { .. } => Self::Relative { step },
            Self::RelativeOffset { .. } => Self::RelativeOffset { step },
        }
    }
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
    Off,
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

/// Which MIDI direction drives a pad's LED. Mutually exclusive — only the
/// selected source lights the LED, so the In and Out feedback paths never fight.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadLedSource {
    /// LED stays dark regardless of MIDI.
    Off,
    /// Incoming host MIDI lights the LED (`midi_in` mode).
    MidiIn,
    /// The pad's own hit lights the LED (`midi_out` mode).
    #[default]
    MidiOut,
}

impl PadLedSource {
    pub const ALL: [PadLedSource; 3] = [
        PadLedSource::Off,
        PadLedSource::MidiIn,
        PadLedSource::MidiOut,
    ];
}

impl std::fmt::Display for PadLedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PadLedSource::Off => "Off",
            PadLedSource::MidiIn => "For MIDI In",
            PadLedSource::MidiOut => "For MIDI Out",
        })
    }
}

/// Which rule a pad LED uses to pick its color for one source.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadLedMode {
    /// One color: dim when idle, brighter when hit.
    Single,
    /// Two colors: `dual_off` when idle, `dual_on` when hit (both normal brightness).
    Dual,
    /// Velocity→hue gradient on hit, dark when idle.
    #[default]
    Velocity,
}

impl PadLedMode {
    pub const ALL: [PadLedMode; 3] = [PadLedMode::Single, PadLedMode::Dual, PadLedMode::Velocity];
}

impl std::fmt::Display for PadLedMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PadLedMode::Single => "Single",
            PadLedMode::Dual => "Dual",
            PadLedMode::Velocity => "Velocity",
        })
    }
}

/// The LED color config for one source: the active `mode` plus the colors for
/// every mode. Storing all of them means switching modes never drops the colors
/// you set for the other modes — they persist and come back when you switch back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PadLedColorMode {
    pub mode: PadLedMode,
    /// Color for `Single` mode.
    pub single: PadColors,
    /// Lit ("on") color for `Dual` mode.
    pub dual_on: PadColors,
    /// Idle ("off") color for `Dual` mode.
    pub dual_off: PadColors,
}

impl Default for PadLedColorMode {
    fn default() -> Self {
        Self {
            mode: PadLedMode::Velocity,
            single: PadColors::White,
            dual_on: PadColors::Blue,
            dual_off: PadColors::Off,
        }
    }
}

impl PadLedColorMode {
    /// `Single` mode preset with `color`; other modes' colors keep their defaults.
    pub const fn single(color: PadColors) -> Self {
        Self {
            mode: PadLedMode::Single,
            single: color,
            dual_on: PadColors::Blue,
            dual_off: PadColors::Off,
        }
    }

    /// `Dual` mode preset with `on`/`off`; other modes' colors keep their defaults.
    pub const fn dual(on: PadColors, off: PadColors) -> Self {
        Self {
            mode: PadLedMode::Dual,
            single: PadColors::White,
            dual_on: on,
            dual_off: off,
        }
    }

    /// `Velocity` mode preset; the (unused) mode colors keep their defaults.
    pub const fn velocity() -> Self {
        Self {
            mode: PadLedMode::Velocity,
            single: PadColors::White,
            dual_on: PadColors::Blue,
            dual_off: PadColors::Off,
        }
    }

    /// Resolve to a concrete `(color, brightness)` for the LED state. `on` is the
    /// note-on / hit state; `velocity` is the hit (or incoming-note) velocity,
    /// used only by `Velocity`. An `Off` color always yields a dark LED.
    pub fn resolve(&self, on: bool, velocity: u8) -> (PadColors, Brightness) {
        let (color, brightness) = match self.mode {
            PadLedMode::Single => (
                self.single,
                if on {
                    Brightness::Normal
                } else {
                    Brightness::Dim
                },
            ),
            PadLedMode::Dual => (
                if on { self.dual_on } else { self.dual_off },
                Brightness::Normal,
            ),
            PadLedMode::Velocity => {
                if on {
                    (pad_color_from_velocity(velocity), Brightness::Normal)
                } else {
                    (PadColors::Off, Brightness::Off)
                }
            }
        };
        if color == PadColors::Off {
            (PadColors::Off, Brightness::Off)
        } else {
            (color, brightness)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadLedConfig {
    pub source: PadLedSource,
    pub midi_in: PadLedColorMode,
    pub midi_out: PadLedColorMode,
}

impl Default for PadLedConfig {
    fn default() -> Self {
        Self {
            source: PadLedSource::MidiOut,
            midi_in: PadLedColorMode::velocity(),
            midi_out: PadLedColorMode::dual(PadColors::Blue, PadColors::Off),
        }
    }
}

impl PadLedConfig {
    /// Resolve to a concrete `(color, brightness)` for the active source's color
    /// mode. `on`/`velocity` feed the mode exactly as [`PadLedColorMode::resolve`];
    /// source `Off` is always a dark LED.
    pub fn resolve(&self, on: bool, velocity: u8) -> (PadColors, Brightness) {
        match self.source {
            PadLedSource::Off => (PadColors::Off, Brightness::Off),
            PadLedSource::MidiIn => self.midi_in.resolve(on, velocity),
            PadLedSource::MidiOut => self.midi_out.resolve(on, velocity),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PadConfig {
    pub hit: PadHitAction,
    pub pressure: PadPressureAction,
    #[serde(default)]
    pub led: PadLedConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ButtonPressAction {
    Cc {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<MidiChannel>,
        cc: u8,
    },
    Off,
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
        #[serde(default)]
        mode: CcValueMode,
    },
    Off,
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
    Off,
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

impl SliderLedMode {
    pub const ALL: [SliderLedMode; 3] =
        [SliderLedMode::Bar, SliderLedMode::Pan, SliderLedMode::Dot];
}

impl std::fmt::Display for SliderLedMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            SliderLedMode::Bar => "Bar",
            SliderLedMode::Pan => "Pan",
            SliderLedMode::Dot => "Dot",
        };
        f.write_str(s)
    }
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
    fn off_variants_round_trip_toml() {
        let h: PadHitAction =
            toml::from_str(&toml::to_string(&PadHitAction::Off).unwrap()).unwrap();
        assert_eq!(h, PadHitAction::Off);
        let b: ButtonPressAction =
            toml::from_str(&toml::to_string(&ButtonPressAction::Off).unwrap()).unwrap();
        assert_eq!(b, ButtonPressAction::Off);
        let e: EncoderTurnAction =
            toml::from_str(&toml::to_string(&EncoderTurnAction::Off).unwrap()).unwrap();
        assert_eq!(e, EncoderTurnAction::Off);
        let s: SliderPositionAction =
            toml::from_str(&toml::to_string(&SliderPositionAction::Off).unwrap()).unwrap();
        assert_eq!(s, SliderPositionAction::Off);
    }

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

    #[test]
    fn encoder_turn_without_mode_field_defaults_to_relative() {
        let toml_str = r#"
type = "cc"
cc = 1
"#;
        let action: EncoderTurnAction = toml::from_str(toml_str).unwrap();
        assert_eq!(
            action,
            EncoderTurnAction::Cc {
                channel: None,
                cc: 1,
                mode: CcValueMode::Relative { step: 1 },
            }
        );
    }

    #[test]
    fn encoder_turn_with_absolute_mode_round_trips() {
        let action = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step: 1,
                wrap: false,
            },
        };
        let s = toml::to_string(&action).unwrap();
        let back: EncoderTurnAction = toml::from_str(&s).unwrap();
        assert_eq!(back, action);
    }

    #[test]
    fn slider_led_mode_labels_are_unique_and_nonempty() {
        use super::SliderLedMode;
        let labels: Vec<String> = SliderLedMode::ALL.iter().map(|m| m.to_string()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()), "{labels:?}");
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "duplicate label: {labels:?}");
    }

    #[test]
    fn pad_led_resolve_single_is_dim_idle_normal_hit() {
        use maschine_library::lights::Brightness;
        let m = PadLedColorMode::single(PadColors::Green);
        assert_eq!(m.resolve(false, 0), (PadColors::Green, Brightness::Dim));
        assert_eq!(m.resolve(true, 0), (PadColors::Green, Brightness::Normal));
    }

    #[test]
    fn pad_led_resolve_dual_switches_color_at_normal() {
        use maschine_library::lights::Brightness;
        let m = PadLedColorMode::dual(PadColors::Blue, PadColors::Off);
        // off-color Off collapses to a dark LED.
        assert_eq!(m.resolve(false, 0), (PadColors::Off, Brightness::Off));
        assert_eq!(m.resolve(true, 0), (PadColors::Blue, Brightness::Normal));
    }

    #[test]
    fn pad_led_resolve_velocity_is_dark_idle_gradient_hit() {
        use maschine_library::lights::Brightness;
        let m = PadLedColorMode::velocity();
        assert_eq!(m.resolve(false, 100), (PadColors::Off, Brightness::Off));
        assert_eq!(m.resolve(true, 64), (PadColors::Lime, Brightness::Normal));
    }
}
