//! Pure derivation of the short MIDI-assignment string shown on the device
//! diagram (selection label + Show-all-labels overlay).

use maschine_library::controls::Buttons;
use protocol::ControlRef;
use settings::Settings;

/// MIDI note name in the convention where note 0 = `C-2`, 60 = `C3`,
/// 127 = `G8` (matches the reference editor's pad labels).
pub fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = note as i16 / 12 - 2;
    let name = NAMES[(note % 12) as usize];
    format!("{name}{octave}")
}

/// The control's **main** action as a short label: encoder→turn CC, button→press
/// CC, slider→position CC, pad→hit note name.
pub fn control_label(settings: &Settings, control: ControlRef) -> String {
    match control {
        ControlRef::Pad(i) => match settings.active_pads()[i as usize].hit {
            settings::PadHitAction::Note { note, .. } => note_name(note),
            settings::PadHitAction::Off => "Off".to_string(),
        },
        ControlRef::Button(i) => match settings.buttons[i as usize].press {
            settings::ButtonPressAction::Cc { cc, .. } => format!("CC {cc}"),
            settings::ButtonPressAction::Off => "Off".to_string(),
        },
        ControlRef::Encoder => match settings.encoder.turn {
            settings::EncoderTurnAction::Cc { cc, .. } => format!("CC {cc}"),
            settings::EncoderTurnAction::Off => "Off".to_string(),
        },
        ControlRef::Slider => match settings.slider.position {
            settings::SliderPositionAction::Cc { cc, .. } => format!("CC {cc}"),
            settings::SliderPositionAction::Off => "Off".to_string(),
        },
    }
}

/// Label for the header box of a specific sub-action tab. `tab` selects which
/// slot (A=Turn/Hit/Position, B=Push/Press/Touch, C=encoder Touch).
pub fn subaction_label(
    settings: &Settings,
    control: ControlRef,
    tab: crate::inspector::assign::forms::AssignTab,
) -> String {
    use crate::inspector::assign::forms::AssignTab::*;
    match control {
        ControlRef::Encoder => match tab {
            A => control_label(settings, ControlRef::Encoder),
            B => control_label(settings, ControlRef::Button(Buttons::EncoderPress as u8)),
            C => control_label(settings, ControlRef::Button(Buttons::EncoderTouch as u8)),
        },
        ControlRef::Pad(i) => match tab {
            B => match settings.active_pads()[i as usize].pressure {
                settings::PadPressureAction::Disabled => "Off".to_string(),
                settings::PadPressureAction::Poly { .. } => "Poly".to_string(),
            },
            _ => control_label(settings, control),
        },
        ControlRef::Slider => match tab {
            B => match settings.slider.touch {
                settings::SliderTouchAction::Disabled => "Off".to_string(),
                settings::SliderTouchAction::Note { note, .. } => note_name(note),
                settings::SliderTouchAction::Cc { cc, .. } => format!("CC {cc}"),
            },
            _ => control_label(settings, control),
        },
        ControlRef::Button(_) => control_label(settings, control),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_name_pins_the_convention() {
        assert_eq!(note_name(0), "C-2");
        assert_eq!(note_name(60), "C3");
        assert_eq!(note_name(127), "G8");
        assert_eq!(note_name(61), "C#3");
    }

    #[test]
    fn control_label_off_is_off() {
        let mut s = Settings::default();
        s.encoder.turn = settings::EncoderTurnAction::Off;
        assert_eq!(control_label(&s, ControlRef::Encoder), "Off");
    }

    #[test]
    fn control_label_reads_the_main_action() {
        let s = Settings::default();
        assert!(control_label(&s, ControlRef::Encoder).starts_with("CC "));
        assert!(control_label(&s, ControlRef::Slider).starts_with("CC "));
        assert!(control_label(&s, ControlRef::Button(0)).starts_with("CC "));
        let pad = control_label(&s, ControlRef::Pad(0));
        assert!(
            !pad.starts_with("CC "),
            "pad label should be a note name: {pad}"
        );
    }
}
