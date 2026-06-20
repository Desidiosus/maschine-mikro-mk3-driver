use maschine_library::controls::Buttons;
use maschine_library::screen::{ScreenCommand, parse_sysex_command, render_centered_text};
use num::FromPrimitive;

use crate::backend::midi as backend_midi;
use crate::outputs::DeviceOutputs;
use crate::settings::PadLedSource;
use crate::settings::Settings;
use crate::settings::actions::{CcValueMode, EncoderTurnAction};

pub fn apply_incoming_midi_message(
    message: &[u8],
    outputs: &DeviceOutputs,
    settings: &Settings,
    rt: &crate::runtime_state::RuntimeState,
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

    if let EncoderTurnAction::Cc {
        channel: enc_channel,
        cc: enc_cc,
        mode: enc_mode,
    } = &settings.encoder.turn
    {
        let enc_resolved_channel = enc_channel.map(|c| c.as_u8()).unwrap_or(0);
        if status == 0xB0
            && channel == enc_resolved_channel
            && data1 == *enc_cc
            && matches!(enc_mode, CcValueMode::Absolute { .. })
        {
            rt.encoder_absolute
                .store(data2, std::sync::atomic::Ordering::Relaxed);
        }
    }

    match status {
        0x90 => {
            if let Some(index) = backend_midi::pad_index_for_message(settings, channel, data1) {
                super::render_pad_led(
                    outputs,
                    settings,
                    PadLedSource::MidiIn,
                    index,
                    data2 > 0,
                    data2,
                );
            }
        }
        0x80 => {
            if let Some(index) = backend_midi::pad_index_for_message(settings, channel, data1) {
                super::render_pad_led(outputs, settings, PadLedSource::MidiIn, index, false, 0);
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
                    settings.hardware.led_brightness > 0,
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
        let mut settings = Settings::default();
        settings.pads[0].led.source = settings::PadLedSource::MidiIn;
        // pads[0].hit.note default = 48
        apply_incoming_midi_message(
            &[0x90, 48, 64],
            &outputs,
            &settings,
            &crate::runtime_state::RuntimeState::default(),
        );

        assert!(outputs.take_lights_dirty());
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Lime, Brightness::Normal)
        );
    }

    #[test]
    fn incoming_note_off_turns_pad_off() {
        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.pads[0].led.source = settings::PadLedSource::MidiIn;
        apply_incoming_midi_message(
            &[0x80, 48, 0],
            &outputs,
            &settings,
            &crate::runtime_state::RuntimeState::default(),
        );

        assert!(outputs.take_lights_dirty());
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
        apply_incoming_midi_message(
            &[0xB0, 42, 100],
            &outputs,
            &settings,
            &crate::runtime_state::RuntimeState::default(),
        );

        assert_eq!(
            outputs.with_lights(|l| l.get_button(Buttons::Play)),
            Brightness::Bright
        );
    }

    #[test]
    fn incoming_message_honors_per_action_channel_override() {
        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.pads[0].led.source = settings::PadLedSource::MidiIn;
        settings.pads[0].hit = crate::settings::actions::PadHitAction::Note {
            channel: crate::settings::MidiChannel::try_from(2).ok(),
            note: 60,
        };
        apply_incoming_midi_message(
            &[0x92, 60, 64],
            &outputs,
            &settings,
            &crate::runtime_state::RuntimeState::default(),
        );

        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Lime, Brightness::Normal)
        );
    }

    #[test]
    fn incoming_message_for_wrong_channel_is_ignored_for_pad() {
        let outputs = DeviceOutputs::new();
        let settings = Settings::default(); // controls default to channel 0
        apply_incoming_midi_message(
            &[0x95, 48, 64],
            &outputs,
            &settings,
            &crate::runtime_state::RuntimeState::default(),
        );

        // No dirty bit set, no pad change
        assert!(!outputs.lights_dirty());
    }

    #[test]
    fn incoming_cc_for_absolute_mode_encoder_syncs_runtime_state() {
        use crate::runtime_state::RuntimeState;
        use crate::settings::actions::{CcValueMode, EncoderTurnAction};

        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.encoder.turn = EncoderTurnAction::Cc {
            channel: None,
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step: 1,
                wrap: false,
            },
        };
        let rt = RuntimeState::default();
        apply_incoming_midi_message(&[0xB0, 1, 64], &outputs, &settings, &rt);
        assert_eq!(rt.encoder_value(), 64);
    }

    #[test]
    fn incoming_cc_for_non_absolute_mode_does_not_sync_runtime_state() {
        use crate::runtime_state::RuntimeState;

        let outputs = DeviceOutputs::new();
        let settings = Settings::default();
        let rt = RuntimeState::default();
        rt.set_encoder_value(42);
        apply_incoming_midi_message(&[0xB0, 1, 64], &outputs, &settings, &rt);
        assert_eq!(rt.encoder_value(), 42);
    }

    #[test]
    fn incoming_cc_for_absolute_mode_wrong_channel_does_not_sync() {
        use crate::runtime_state::RuntimeState;
        use crate::settings::actions::{CcValueMode, EncoderTurnAction};

        let outputs = DeviceOutputs::new();
        let mut settings = Settings::default();
        settings.encoder.turn = EncoderTurnAction::Cc {
            channel: crate::settings::MidiChannel::try_from(0).ok(),
            cc: 1,
            mode: CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step: 1,
                wrap: false,
            },
        };
        let rt = RuntimeState::default();
        apply_incoming_midi_message(&[0xB5, 1, 64], &outputs, &settings, &rt);
        assert_eq!(rt.encoder_value(), 0);
    }
}
