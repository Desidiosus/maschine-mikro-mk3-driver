use maschine_library::controls::Buttons;
use maschine_library::lights::{Brightness, PadColors};
use maschine_library::screen::{ScreenCommand, parse_sysex_command, render_centered_text};
use num::FromPrimitive;

use crate::backend::midi as backend_midi;
use crate::outputs::DeviceOutputs;
use crate::settings::MidiMapping;

pub fn apply_incoming_midi_message(
    message: &[u8],
    outputs: &DeviceOutputs,
    midi_mapping: &MidiMapping,
    backlight_enabled: bool,
    backlight_brightness: Brightness,
) {
    if message.first().copied() == Some(0xF0) {
        apply_incoming_sysex(message, outputs);
        return;
    }

    if message.len() < 3 {
        return;
    }

    let status = message[0] & 0xF0;
    let channel = message[0] & 0x0F;
    let data1 = message[1];
    let data2 = message[2];

    match status {
        0x90 => {
            if let Some(index) = backend_midi::pad_index_for_message(midi_mapping, channel, data1) {
                outputs.with_lights_mut(|lights| {
                    if data2 > 0 {
                        lights.set_pad(index, pad_color_from_velocity(data2), Brightness::Normal);
                    } else {
                        lights.set_pad(index, PadColors::Off, Brightness::Off);
                    }
                });
            }
        }
        0x80 => {
            if let Some(index) = backend_midi::pad_index_for_message(midi_mapping, channel, data1) {
                outputs.with_lights_mut(|lights| {
                    lights.set_pad(index, PadColors::Off, Brightness::Off);
                });
            }
        }
        0xB0 => {
            let Some(button_index) =
                backend_midi::button_index_for_message(midi_mapping, channel, data1)
            else {
                return;
            };
            let Some(button) = Buttons::from_usize(button_index) else {
                return;
            };

            outputs.with_lights_mut(|lights| {
                if !lights.button_has_light(button) {
                    return;
                }

                let brightness = backend_midi::button_brightness_from_value(
                    data2,
                    backlight_enabled,
                    backlight_brightness,
                );
                lights.set_button(button, brightness);
            });
        }
        _ => {}
    }
}

fn apply_incoming_sysex(message: &[u8], outputs: &DeviceOutputs) {
    match parse_sysex_command(message) {
        Some(ScreenCommand::Text(text)) => {
            outputs.with_screen_mut(|screen| render_centered_text(screen, &text));
        }
        Some(ScreenCommand::Clear) => {
            outputs.with_screen_mut(|screen| screen.reset());
        }
        None => {}
    }
}

pub fn pad_color_from_velocity(velocity: u8) -> PadColors {
    match velocity {
        0 => PadColors::Off,
        1..=7 => PadColors::Red,
        8..=14 => PadColors::Orange,
        15..=21 => PadColors::LightOrange,
        22..=28 => PadColors::WarmYellow,
        29..=35 => PadColors::Yellow,
        36..=42 => PadColors::Lime,
        43..=49 => PadColors::Green,
        50..=56 => PadColors::Mint,
        57..=63 => PadColors::Cyan,
        64..=70 => PadColors::Turquoise,
        71..=77 => PadColors::Blue,
        78..=84 => PadColors::Plum,
        85..=91 => PadColors::Violet,
        92..=98 => PadColors::Purple,
        99..=105 => PadColors::Magenta,
        106..=112 => PadColors::Fuchsia,
        _ => PadColors::White,
    }
}

#[cfg(test)]
mod tests {
    use maschine_library::controls::Buttons;
    use maschine_library::lights::{Brightness, PadColors};

    #[test]
    fn incoming_sysex_text_marks_screen_dirty_and_updates_screen() {
        let outputs = crate::outputs::DeviceOutputs::new();
        let message = [0xF0, 0x00, 0x21, 0x09, 0x01, b'H', b'i', 0xF7];
        let mapping = crate::settings::Settings::default().midi;

        crate::feedback::midi::apply_incoming_midi_message(
            &message,
            &outputs,
            &mapping,
            false,
            Brightness::Dim,
        );

        assert!(outputs.screen_dirty());
        assert!(outputs.take_screen_dirty());
        assert!(!outputs.take_screen_dirty());
        assert!(
            outputs.with_screen(|screen| {
                (0..32).any(|row| (0..128).any(|col| screen.get(row, col)))
            })
        );
    }

    #[test]
    fn incoming_note_updates_pad_lights_and_marks_dirty() {
        let outputs = crate::outputs::DeviceOutputs::new();
        let mapping = crate::settings::MidiMapping {
            pad_notes: [
                36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51,
            ],
            ..crate::settings::Settings::default().midi
        };
        let message = [0x90, 36, 64];

        crate::feedback::midi::apply_incoming_midi_message(
            &message,
            &outputs,
            &mapping,
            false,
            Brightness::Dim,
        );

        assert!(outputs.lights_dirty());
        assert!(outputs.take_lights_dirty());
        assert!(!outputs.take_lights_dirty());
        assert_eq!(
            outputs.with_lights(|lights| lights.get_pad(0)),
            (PadColors::Turquoise, Brightness::Normal)
        );
    }

    #[test]
    fn incoming_cc_updates_button_light_and_marks_dirty() {
        let outputs = crate::outputs::DeviceOutputs::new();
        let message = [0xB0, 42, 100];
        let mapping = crate::settings::Settings::default().midi;

        crate::feedback::midi::apply_incoming_midi_message(
            &message,
            &outputs,
            &mapping,
            false,
            Brightness::Dim,
        );

        assert!(outputs.lights_dirty());
        assert_eq!(
            outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
            Brightness::Bright
        );
    }

    #[test]
    fn incoming_feedback_honors_configured_nonzero_channel() {
        let outputs = crate::outputs::DeviceOutputs::new();
        let mapping = crate::settings::MidiMapping {
            channel: 2u8.try_into().unwrap(),
            pad_notes: [
                60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75,
            ],
            button_ccs: {
                let mut ccs = crate::settings::Settings::default().midi.button_ccs;
                ccs[Buttons::Play as usize] = 62;
                ccs
            },
            ..crate::settings::Settings::default().midi
        };

        crate::feedback::midi::apply_incoming_midi_message(
            &[0x92, 60, 64],
            &outputs,
            &mapping,
            false,
            Brightness::Dim,
        );
        crate::feedback::midi::apply_incoming_midi_message(
            &[0xB2, 62, 100],
            &outputs,
            &mapping,
            false,
            Brightness::Dim,
        );

        assert_eq!(
            outputs.with_lights(|lights| lights.get_pad(0)),
            (PadColors::Turquoise, Brightness::Normal)
        );
        assert_eq!(
            outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
            Brightness::Bright
        );
    }
}
