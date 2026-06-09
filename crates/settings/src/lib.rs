use serde::{Deserialize, Serialize};

pub mod actions;
pub mod buttons_by_name;
pub mod defaults;
pub mod groups;
pub mod pads_by_index;
pub mod partial;
pub mod velocity_curve;

pub use partial::PartialSettings;
pub use velocity_curve::PadVelocityCurve;

pub use actions::{
    ButtonConfig, ButtonPressAction, CcValueMode, EncoderConfig, EncoderTurnAction, PadConfig,
    PadHitAction, PadPressureAction, SliderConfig, SliderLedMode, SliderLedSettings,
    SliderPositionAction, SliderTouchAction,
};
pub use buttons_by_name::ButtonsByName;
pub use groups::{BridgeSettings, GlobalSettings, HardwareSettings};
pub use maschine_library::lights::PadColors;
pub use pads_by_index::PadsByIndex;

#[derive(Default, Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BacklightBrightness {
    #[default]
    Dim,
    Normal,
    Bright,
}

impl BacklightBrightness {
    pub fn as_light_brightness(self) -> maschine_library::lights::Brightness {
        match self {
            Self::Dim => maschine_library::lights::Brightness::Dim,
            Self::Normal => maschine_library::lights::Brightness::Normal,
            Self::Bright => maschine_library::lights::Brightness::Bright,
        }
    }
}

impl std::fmt::Display for BacklightBrightness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BacklightBrightness::Dim => "dim",
            BacklightBrightness::Normal => "normal",
            BacklightBrightness::Bright => "bright",
        };
        f.write_str(s)
    }
}

impl BacklightBrightness {
    pub const ALL: [BacklightBrightness; 3] = [
        BacklightBrightness::Dim,
        BacklightBrightness::Normal,
        BacklightBrightness::Bright,
    ];
}

#[derive(Default, Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(try_from = "u8", into = "u8")]
pub struct MidiChannel(u8);

impl MidiChannel {
    pub const fn as_u8(self) -> u8 {
        self.0
    }

    /// Convert an optional stored channel byte (0..=15) into `Option<MidiChannel>`.
    pub fn try_from_opt(v: u8) -> Option<MidiChannel> {
        MidiChannel::try_from(v).ok()
    }
}

impl TryFrom<u8> for MidiChannel {
    type Error = String;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= 15 {
            Ok(Self(value))
        } else {
            Err(format!(
                "invalid midi_channel={value} (expected integer in range 0..=15)"
            ))
        }
    }
}

impl From<MidiChannel> for u8 {
    fn from(value: MidiChannel) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub global: GlobalSettings,
    pub hardware: HardwareSettings,
    pub bridge: BridgeSettings,
    pub pads: PadsByIndex,
    pub buttons: ButtonsByName,
    pub encoder: EncoderConfig,
    pub slider: SliderConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            global: GlobalSettings::default(),
            hardware: HardwareSettings::default(),
            bridge: BridgeSettings::default(),
            pads: defaults::default_pads(),
            buttons: defaults::default_buttons(),
            encoder: defaults::default_encoder(),
            slider: defaults::default_slider(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.hardware.pad_sensitivity > 100 {
            return Err("pad_sensitivity must be in range 0..=100".to_string());
        }
        if self.hardware.display_contrast > 100 {
            return Err("display_contrast must be in range 0..=100".to_string());
        }
        if self.global.client_name.is_empty() {
            return Err("client_name must not be empty".to_string());
        }
        if self.global.port_name.is_empty() {
            return Err("port_name must not be empty".to_string());
        }
        if self.global.port_name_in.is_empty() {
            return Err("port_name_in must not be empty".to_string());
        }
        // Exhaustive over `EncoderTurnAction` on purpose: a future value-bearing
        // variant must fail to compile here (forcing a validation decision)
        // rather than silently skip range checks via a catch-all early return.
        match &self.encoder.turn {
            EncoderTurnAction::Off => {}
            EncoderTurnAction::Cc { mode, .. } => match mode {
                actions::CcValueMode::Absolute { lo, hi, step, .. } => {
                    if lo > hi {
                        return Err(format!(
                            "encoder Absolute mode: lo ({lo}) must be <= hi ({hi})"
                        ));
                    }
                    if *step == 0 {
                        return Err("encoder Absolute mode: step must be >= 1".to_string());
                    }
                    if *lo > 127 || *hi > 127 || *step > 127 {
                        return Err(
                            "encoder Absolute mode: lo, hi, step must each be <= 127".to_string()
                        );
                    }
                }
                actions::CcValueMode::Relative { step } => {
                    if *step == 0 {
                        return Err("encoder Relative mode: step must be >= 1".to_string());
                    }
                    if *step > 63 {
                        return Err(
                            "encoder Relative mode: step must be <= 63 (NI relative protocol range)"
                                .to_string(),
                        );
                    }
                }
                actions::CcValueMode::RelativeOffset { step } => {
                    if *step == 0 {
                        return Err("encoder RelativeOffset mode: step must be >= 1".to_string());
                    }
                    if *step > 127 {
                        return Err("encoder RelativeOffset mode: step must be <= 127".to_string());
                    }
                }
            },
        }
        Ok(())
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;
    use crate::actions::CcValueMode;

    #[test]
    fn midi_channel_try_from_opt_clamps_range() {
        assert!(crate::MidiChannel::try_from_opt(0).is_some());
        assert!(crate::MidiChannel::try_from_opt(15).is_some());
        assert!(crate::MidiChannel::try_from_opt(16).is_none());
    }

    fn settings_with_encoder_mode(mode: CcValueMode) -> Settings {
        let mut s = Settings::default();
        s.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode,
        };
        s
    }

    #[test]
    fn backlight_display_matches_serde_token_for_all_variants() {
        for v in super::BacklightBrightness::ALL {
            let token = serde_json::to_string(&v).unwrap();
            assert_eq!(v.to_string(), token.trim_matches('"'), "{v:?}");
        }
    }

    #[test]
    fn validate_rejects_absolute_with_lo_greater_than_hi() {
        let s = settings_with_encoder_mode(CcValueMode::Absolute {
            lo: 100,
            hi: 50,
            step: 1,
            wrap: false,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_absolute_step_zero() {
        let s = settings_with_encoder_mode(CcValueMode::Absolute {
            lo: 0,
            hi: 127,
            step: 0,
            wrap: false,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_relative_step_zero() {
        let s = settings_with_encoder_mode(CcValueMode::Relative { step: 0 });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_relative_offset_step_zero() {
        let s = settings_with_encoder_mode(CcValueMode::RelativeOffset { step: 0 });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_default_encoder() {
        let s = Settings::default();
        assert!(s.validate().is_ok());
    }

    #[test]
    fn validate_rejects_relative_step_above_63() {
        let s = settings_with_encoder_mode(CcValueMode::Relative { step: 64 });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_relative_offset_step_127() {
        let s = settings_with_encoder_mode(CcValueMode::RelativeOffset { step: 127 });
        assert!(s.validate().is_ok());
    }
}
