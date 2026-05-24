use maschine_library::controls::Buttons;
use maschine_library::lights::{Brightness, PadColors};
use maschine_library::screen::{ScreenCommand, parse_sysex_command, render_centered_text};
use num::FromPrimitive;

use crate::backend::midi as backend_midi;
use crate::outputs::DeviceOutputs;
use crate::settings::Settings;

pub fn apply_incoming_midi_message(message: &[u8], outputs: &DeviceOutputs, settings: &Settings) {
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
            if let Some(index) = backend_midi::pad_index_for_message(settings, channel, data1) {
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
            if let Some(index) = backend_midi::pad_index_for_message(settings, channel, data1) {
                outputs.with_lights_mut(|lights| {
                    lights.set_pad(index, PadColors::Off, Brightness::Off);
                });
            }
        }
        0xB0 => {
            let Some(button_index) =
                backend_midi::button_index_for_message(settings, channel, data1)
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
                    settings.hardware.backlight_buttons,
                    settings.hardware.backlight_brightness.as_light_brightness(),
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
    use super::apply_incoming_midi_message;
    use crate::outputs::DeviceOutputs;
    use crate::settings::Settings;
    use maschine_library::controls::Buttons;
    use maschine_library::lights::{Brightness, PadColors};

    #[test]
    fn incoming_note_on_lights_pad_with_velocity_color() {
        let outputs = DeviceOutputs::new();
        let settings = Settings::default();
        // pads[0].hit.note default = 48
        apply_incoming_midi_message(&[0x90, 48, 64], &outputs, &settings);

        assert!(outputs.take_lights_dirty());
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Turquoise, Brightness::Normal)
        );
    }

    #[test]
    fn incoming_note_off_turns_pad_off() {
        let outputs = DeviceOutputs::new();
        let settings = Settings::default();
        apply_incoming_midi_message(&[0x80, 48, 0], &outputs, &settings);

        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Off, Brightness::Off)
        );
    }

    #[test]
    fn incoming_cc_for_play_button_sets_brightness() {
        let outputs = DeviceOutputs::new();
        let settings = Settings::default();
        // play button default CC = 42
        apply_incoming_midi_message(&[0xB0, 42, 100], &outputs, &settings);

        assert_eq!(
            outputs.with_lights(|l| l.get_button(Buttons::Play)),
            Brightness::Bright
        );
    }

    #[test]
    fn incoming_message_honors_per_action_channel_override() {
        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.pads[0].hit = crate::settings::actions::PadHitAction::Note {
            channel: crate::settings::MidiChannel::try_from(2).ok(),
            note: 60,
        };
        apply_incoming_midi_message(&[0x92, 60, 64], &outputs, &settings);

        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Turquoise, Brightness::Normal)
        );
    }

    #[test]
    fn incoming_message_for_wrong_channel_is_ignored_for_pad() {
        let outputs = DeviceOutputs::new();
        let settings = Settings::default(); // global channel 0
        apply_incoming_midi_message(&[0x95, 48, 64], &outputs, &settings);

        // No dirty bit set, no pad change
        assert!(!outputs.lights_dirty());
    }
}
