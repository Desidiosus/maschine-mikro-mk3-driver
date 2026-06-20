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
    PadHitAction, PadLedColorMode, PadLedConfig, PadLedMode, PadLedSource, PadPressureAction,
    SliderConfig, SliderLedMode, SliderLedSettings, SliderPositionAction, SliderTouchAction,
};
pub use buttons_by_name::ButtonsByName;
pub use groups::{BridgeSettings, GlobalSettings, HardwareSettings};
pub use maschine_library::lights::PadColors;
pub use maschine_library::preferences::MAX_BUTTON_BRIGHTNESS;
pub use pads_by_index::PadsByIndex;

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
        if self.hardware.led_brightness > MAX_BUTTON_BRIGHTNESS {
            return Err(format!(
                "led_brightness must be in range 0..={MAX_BUTTON_BRIGHTNESS}"
            ));
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
        // Every emitted MIDI data byte must fit in 7 bits. The GUI clamps these,
        // but a hand-edited config file or a raw IPC client could carry an
        // out-of-range value; reject it here (the single gate every load and apply
        // passes through) so it can never reach the wire as a corrupt byte whose
        // high bit would be read as a status byte.
        let check = |label: &str, value: u8| -> Result<(), String> {
            if value > CcValueMode::CC_VALUE_MAX {
                Err(format!(
                    "{label} must be in range 0..={} (got {value})",
                    CcValueMode::CC_VALUE_MAX
                ))
            } else {
                Ok(())
            }
        };

        // Each match below is exhaustive on purpose: a future value-bearing
        // variant must fail to compile here (forcing a validation decision)
        // rather than silently skip range checks via a catch-all early return.
        for (i, pad) in self.pads.iter().enumerate() {
            let key = crate::pads_by_index::internal_to_config_key(i);
            match &pad.hit {
                PadHitAction::Note { note, .. } => check(&format!("pad {key} note"), *note)?,
                PadHitAction::Off => {}
            }
            match &pad.pressure {
                PadPressureAction::Disabled => {}
                PadPressureAction::Poly {
                    note: Some(note), ..
                } => check(&format!("pad {key} pressure note"), *note)?,
                PadPressureAction::Poly { note: None, .. } => {}
            }
        }
        for (i, button) in self.buttons.0.iter().enumerate() {
            match &button.press {
                ButtonPressAction::Cc { cc, .. } => check(
                    &format!("button {} cc", maschine_library::controls::BUTTON_NAMES[i]),
                    *cc,
                )?,
                ButtonPressAction::Off => {}
            }
        }
        match &self.slider.position {
            SliderPositionAction::Cc { cc, .. } => check("slider position cc", *cc)?,
            SliderPositionAction::Off => {}
        }
        match &self.slider.touch {
            SliderTouchAction::Disabled => {}
            SliderTouchAction::Note {
                note,
                on_value,
                off_value,
                ..
            } => {
                check("slider touch note", *note)?;
                check("slider touch on_value", *on_value)?;
                check("slider touch off_value", *off_value)?;
            }
            SliderTouchAction::Cc {
                cc,
                on_value,
                off_value,
                ..
            } => {
                check("slider touch cc", *cc)?;
                check("slider touch on_value", *on_value)?;
                check("slider touch off_value", *off_value)?;
            }
        }

        match &self.encoder.turn {
            EncoderTurnAction::Off => {}
            EncoderTurnAction::Cc { cc, mode, .. } => {
                check("encoder cc", *cc)?;
                if let actions::CcValueMode::Absolute { lo, hi, .. } = mode {
                    if lo > hi {
                        return Err(format!(
                            "encoder Absolute mode: lo ({lo}) must be <= hi ({hi})"
                        ));
                    }
                    if *lo > CcValueMode::CC_VALUE_MAX || *hi > CcValueMode::CC_VALUE_MAX {
                        return Err(format!(
                            "encoder Absolute mode: lo and hi must each be <= {}",
                            CcValueMode::CC_VALUE_MAX
                        ));
                    }
                }
                // A `0` step would freeze the encoder; the rest of the range is
                // bounded per variant (`Relative` is the only NI-wire-limited one).
                let step = mode.step();
                if step == 0 {
                    return Err("encoder mode: step must not be 0".to_string());
                }
                let (min, max) = mode.step_bounds();
                if step < min || step > max {
                    return Err(format!("encoder mode: step must be in [{min}, {max}]"));
                }
            }
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
    fn validate_rejects_led_brightness_above_10() {
        let mut s = Settings::default();
        s.hardware.led_brightness = 11;
        assert!(s.validate().is_err());
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
    fn validate_rejects_relative_step_above_max() {
        let s = settings_with_encoder_mode(CcValueMode::Relative {
            step: CcValueMode::RELATIVE_STEP_MAX + 1,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_relative_step_below_min() {
        let s = settings_with_encoder_mode(CcValueMode::Relative {
            step: CcValueMode::RELATIVE_STEP_MIN - 1,
        });
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_relative_step_at_magnitude_bounds() {
        for step in [
            CcValueMode::RELATIVE_STEP_MIN,
            CcValueMode::RELATIVE_STEP_MAX,
        ] {
            let s = settings_with_encoder_mode(CcValueMode::Relative { step });
            assert!(s.validate().is_ok(), "step {step} should be accepted");
        }
    }

    #[test]
    fn validate_accepts_relative_offset_step_beyond_ni_range() {
        // RelativeOffset clamps its emitted value to the 7-bit CC range, so a
        // step magnitude past the NI relative limit is still valid.
        for step in [-100, 100, i8::MIN, i8::MAX] {
            let s = settings_with_encoder_mode(CcValueMode::RelativeOffset { step });
            assert!(s.validate().is_ok(), "step {step} should be accepted");
        }
    }

    #[test]
    fn validate_accepts_absolute_negative_step() {
        let s = settings_with_encoder_mode(CcValueMode::Absolute {
            lo: 0,
            hi: 127,
            step: CcValueMode::ABSOLUTE_STEP_MIN,
            wrap: false,
        });
        assert!(s.validate().is_ok());
    }

    #[test]
    fn validate_rejects_out_of_range_pad_note() {
        let mut s = Settings::default();
        s.pads[0].hit = crate::actions::PadHitAction::Note {
            channel: None,
            note: 200,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_out_of_range_button_cc() {
        let mut s = Settings::default();
        s.buttons.0[0].press = crate::actions::ButtonPressAction::Cc {
            channel: None,
            cc: 240,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_out_of_range_slider_touch_on_value() {
        let mut s = Settings::default();
        s.slider.touch = crate::actions::SliderTouchAction::Cc {
            channel: None,
            cc: 70,
            on_value: 200,
            off_value: 0,
        };
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_accepts_max_legal_data_bytes() {
        let mut s = Settings::default();
        s.pads[0].hit = crate::actions::PadHitAction::Note {
            channel: None,
            note: 127,
        };
        s.slider.touch = crate::actions::SliderTouchAction::Note {
            channel: None,
            note: 127,
            on_value: 127,
            off_value: 127,
        };
        assert!(s.validate().is_ok());
    }
}
