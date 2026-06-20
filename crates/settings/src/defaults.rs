use crate::{
    actions::{
        ButtonConfig, ButtonPressAction, CcValueMode, EncoderConfig, EncoderTurnAction, PadConfig,
        PadHitAction, PadLedConfig, PadPressureAction, SliderConfig, SliderLedSettings,
        SliderPositionAction, SliderTouchAction,
    },
    buttons_by_name::ButtonsByName,
    pads_by_index::PadsByIndex,
};

const DEFAULT_PAD_NOTES: [u8; 16] = [
    48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39,
];

const DEFAULT_BUTTON_CCS: [u8; 41] = [
    20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43,
    44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60,
];

pub(crate) const DEFAULT_ENCODER_CC: u8 = 1;
pub(crate) const DEFAULT_SLIDER_CC: u8 = 9;

pub(crate) fn default_pads() -> PadsByIndex {
    let arr: [PadConfig; 16] = std::array::from_fn(|i| PadConfig {
        hit: PadHitAction::Note {
            channel: None,
            note: DEFAULT_PAD_NOTES[i],
        },
        pressure: PadPressureAction::Disabled,
        led: PadLedConfig::default(),
    });
    PadsByIndex(arr)
}

pub(crate) fn default_buttons() -> ButtonsByName {
    let arr: [ButtonConfig; 41] = std::array::from_fn(|i| ButtonConfig {
        press: ButtonPressAction::Cc {
            channel: None,
            cc: DEFAULT_BUTTON_CCS[i],
        },
    });
    ButtonsByName(arr)
}

pub(crate) fn default_encoder() -> EncoderConfig {
    EncoderConfig {
        turn: EncoderTurnAction::Cc {
            channel: None,
            cc: DEFAULT_ENCODER_CC,
            mode: CcValueMode::Relative { step: 1 },
        },
    }
}

pub(crate) fn default_slider() -> SliderConfig {
    SliderConfig {
        position: SliderPositionAction::Cc {
            channel: None,
            cc: DEFAULT_SLIDER_CC,
        },
        touch: SliderTouchAction::Disabled,
        led: SliderLedSettings::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Settings;

    #[test]
    fn settings_default_pads_have_expected_notes() {
        let pads = default_pads();
        match &pads[0].hit {
            PadHitAction::Note { note, .. } => assert_eq!(*note, 48),
            PadHitAction::Off => panic!("expected note"),
        }
        assert_eq!(pads[0].pressure, PadPressureAction::Disabled);
        match &pads[15].hit {
            PadHitAction::Note { note, .. } => assert_eq!(*note, 39),
            PadHitAction::Off => panic!("expected note"),
        }
    }

    #[test]
    fn settings_default_buttons_have_expected_ccs() {
        use maschine_library::controls::Buttons;
        let buttons = default_buttons();
        match &buttons[Buttons::Play as usize].press {
            ButtonPressAction::Cc { cc, .. } => assert_eq!(*cc, 42),
            ButtonPressAction::Off => panic!("expected cc"),
        }
    }

    #[test]
    fn settings_default_is_round_trippable() {
        let s = Settings::default();
        let serialized = toml::to_string(&s).unwrap();
        let back: Settings = toml::from_str(&serialized).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn default_pad_led_preserves_blue_on_hit() {
        use maschine_library::lights::{Brightness, PadColors};
        let pads = default_pads();
        let led = pads[0].led;
        assert_eq!(led.source, crate::PadLedSource::MidiOut);
        assert_eq!(
            led.midi_out.resolve(true, 0),
            (PadColors::Blue, Brightness::Normal)
        );
        assert_eq!(
            led.midi_out.resolve(false, 0),
            (PadColors::Off, Brightness::Off)
        );
        assert_eq!(led.midi_in.mode, crate::PadLedMode::Velocity);
    }
}
