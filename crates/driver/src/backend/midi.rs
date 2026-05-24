use maschine_library::lights::Brightness;
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::midi::apply_incoming_midi_message;
use crate::outputs::DeviceOutputs;
use crate::settings::actions::{
    ButtonPressAction, EncoderTurnAction, PadHitAction, PadPressureAction, SliderPositionAction,
    SliderTouchAction,
};
use crate::settings::{MidiChannel, Settings};
use crate::soft_off::SoftOffSync;
use crate::virmidi_bridge::try_autoconnect_virmidi;

/// Downstream MIDI send step; lets tests substitute a capturing fake.
pub trait MidiSink {
    fn send(&mut self, bytes: &[u8]) -> DriverResult<()>;
}

impl MidiSink for MidiOutputConnection {
    fn send(&mut self, bytes: &[u8]) -> DriverResult<()> {
        MidiOutputConnection::send(self, bytes)
            .map_err(|err| DriverError::Midi(format!("failed to send MIDI message: {err}")))
    }
}

pub struct MidiBackend<S: MidiSink = MidiOutputConnection> {
    settings: Settings,
    sink: S,
    _input: Option<MidiInputConnection<DeviceOutputs>>,
}

impl MidiBackend {
    pub fn new(
        settings: &Settings,
        outputs: &DeviceOutputs,
        soft_off: SoftOffSync,
    ) -> DriverResult<Self> {
        let sink = MidiOutput::new(&settings.global.client_name)
            .map_err(|err| DriverError::Midi(format!("couldn't open MIDI output: {err}")))?
            .create_virtual(&settings.global.port_name)
            .map_err(|err| {
                DriverError::Midi(format!("couldn't create virtual output port: {err}"))
            })?;

        let input = create_midi_input(settings, outputs.clone(), soft_off)?;

        if settings.bridge.midi_bridge_virmidi && settings.bridge.autoconnect_virmidi {
            try_autoconnect_virmidi(settings)?;
        }

        Ok(Self {
            settings: settings.clone(),
            sink,
            _input: Some(input),
        })
    }
}

impl<S: MidiSink> MidiBackend<S> {
    /// Construct a backend around an arbitrary sink, without opening a
    /// MIDI input port. Intended for tests.
    pub fn with_sink(settings: Settings, sink: S) -> Self {
        Self {
            settings,
            sink,
            _input: None,
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn handle_event(&mut self, event: &ControlEvent) -> DriverResult<()> {
        let bytes = event_to_midi_bytes(event, &self.settings)
            .ok_or_else(|| DriverError::Midi(format!("unsupported control event: {event:?}")))?;

        self.sink.send(&bytes)
    }
}

fn create_midi_input(
    settings: &Settings,
    outputs: DeviceOutputs,
    soft_off: SoftOffSync,
) -> DriverResult<MidiInputConnection<DeviceOutputs>> {
    let settings_clone = settings.clone();
    let client_name = format!("{} In", settings.global.client_name);

    MidiInput::new(&client_name)
        .map_err(|err| DriverError::Midi(format!("couldn't open MIDI input: {err}")))?
        .create_virtual(
            &settings.global.port_name_in,
            move |_timestamp, message, outputs| {
                let _guard = soft_off.lock();
                if soft_off.is_active() {
                    return;
                }
                apply_incoming_midi_message(message, outputs, &settings_clone);
            },
            outputs,
        )
        .map_err(|err| DriverError::Midi(format!("couldn't create virtual input port: {err}")))
}

fn resolve_channel(per_action: Option<MidiChannel>, global: MidiChannel) -> u8 {
    per_action.unwrap_or(global).as_u8()
}

pub fn event_to_midi_bytes(event: &ControlEvent, settings: &Settings) -> Option<[u8; 3]> {
    let global = settings.global.midi_channel;

    match event {
        ControlEvent::ButtonChanged { index, pressed } => {
            let btn = settings.buttons.0.get(*index)?;
            match &btn.press {
                ButtonPressAction::Cc { channel, cc } => Some([
                    0xB0 | resolve_channel(*channel, global),
                    *cc,
                    if *pressed { 127 } else { 0 },
                ]),
            }
        }
        ControlEvent::EncoderTurn { cc_value, .. } => {
            let EncoderTurnAction::Cc { channel, cc } = &settings.encoder.turn;
            Some([0xB0 | resolve_channel(*channel, global), *cc, *cc_value])
        }
        ControlEvent::SliderMoved { cc_value, .. } => {
            let SliderPositionAction::Cc { channel, cc } = &settings.slider.position;
            Some([0xB0 | resolve_channel(*channel, global), *cc, *cc_value])
        }
        ControlEvent::SliderTouch { pressed } => match &settings.slider.touch {
            SliderTouchAction::Disabled => None,
            SliderTouchAction::Note {
                channel,
                note,
                on_value,
                off_value,
            } => Some(if *pressed {
                [0x90 | resolve_channel(*channel, global), *note, *on_value]
            } else {
                [0x80 | resolve_channel(*channel, global), *note, *off_value]
            }),
            SliderTouchAction::Cc {
                channel,
                cc,
                on_value,
                off_value,
            } => Some([
                0xB0 | resolve_channel(*channel, global),
                *cc,
                if *pressed { *on_value } else { *off_value },
            ]),
        },
        ControlEvent::PadNoteOn { index, velocity } => {
            let pad = settings.pads.get(*index)?;
            let PadHitAction::Note { channel, note } = &pad.hit;
            Some([0x90 | resolve_channel(*channel, global), *note, *velocity])
        }
        ControlEvent::PadNoteOff { index, velocity } => {
            let pad = settings.pads.get(*index)?;
            let PadHitAction::Note { channel, note } = &pad.hit;
            Some([0x80 | resolve_channel(*channel, global), *note, *velocity])
        }
        ControlEvent::PadAftertouch { index, pressure } => {
            let pad = settings.pads.get(*index)?;
            match &pad.pressure {
                PadPressureAction::Disabled => None,
                PadPressureAction::Poly { channel, note } => {
                    let resolved_note = note.unwrap_or_else(|| {
                        let PadHitAction::Note { note, .. } = &pad.hit;
                        *note
                    });
                    Some([
                        0xA0 | resolve_channel(*channel, global),
                        resolved_note,
                        *pressure,
                    ])
                }
            }
        }
    }
}

#[allow(dead_code)]
pub fn pad_index_for_message(_settings: &Settings, _channel: u8, _note: u8) -> Option<usize> {
    // Real implementation arrives in Task 17.
    None
}

#[allow(dead_code)]
pub fn button_index_for_message(_settings: &Settings, _channel: u8, _cc: u8) -> Option<usize> {
    // Real implementation arrives in Task 17.
    None
}

pub fn button_brightness_from_value(
    value: u8,
    backlight_enabled: bool,
    backlight_brightness: Brightness,
) -> Brightness {
    let brightness = if value > 0 {
        match value {
            1..=42 => Brightness::Dim,
            43..=84 => Brightness::Normal,
            _ => Brightness::Bright,
        }
    } else {
        Brightness::Off
    };

    if backlight_enabled && brightness == Brightness::Off {
        backlight_brightness
    } else {
        brightness
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::ControlEvent;
    use crate::settings::actions::{
        ButtonPressAction, PadHitAction, PadPressureAction, SliderTouchAction,
    };
    use crate::settings::{MidiChannel, Settings};

    fn settings_with_pad_pressure_enabled(idx: usize, channel: u8, note: Option<u8>) -> Settings {
        let mut s = Settings::default();
        s.pads[idx].pressure = PadPressureAction::Poly {
            channel: MidiChannel::try_from(channel).ok(),
            note,
        };
        s
    }

    #[test]
    fn button_press_emits_cc_with_127() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: true,
            },
            &Settings::default(),
        );
        assert_eq!(bytes, Some([0xB0, 42, 127]));
    }

    #[test]
    fn button_release_emits_cc_with_0() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: false,
            },
            &Settings::default(),
        );
        assert_eq!(bytes, Some([0xB0, 42, 0]));
    }

    #[test]
    fn encoder_emits_cc_with_relative_offset_value() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::EncoderTurn {
                delta: 1,
                cc_value: 65,
            },
            &Settings::default(),
        );
        assert_eq!(bytes, Some([0xB0, 1, 65]));
    }

    #[test]
    fn slider_moved_emits_cc() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::SliderMoved {
                raw: 100,
                cc_value: 63,
            },
            &Settings::default(),
        );
        assert_eq!(bytes, Some([0xB0, 9, 63]));
    }

    #[test]
    fn slider_touch_disabled_drops_silently() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::SliderTouch { pressed: true },
            &Settings::default(),
        );
        assert_eq!(bytes, None);
    }

    #[test]
    fn slider_touch_as_note_emits_note_on_off_with_configured_values() {
        let mut s = Settings::default();
        s.slider.touch = SliderTouchAction::Note {
            channel: None,
            note: 60,
            on_value: 100,
            off_value: 10,
        };

        let press = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: true }, &s);
        let release = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: false }, &s);
        assert_eq!(press, Some([0x90, 60, 100]));
        assert_eq!(release, Some([0x80, 60, 10]));
    }

    #[test]
    fn slider_touch_as_cc_emits_cc_with_on_off_values() {
        let mut s = Settings::default();
        s.slider.touch = SliderTouchAction::Cc {
            channel: None,
            cc: 70,
            on_value: 127,
            off_value: 0,
        };

        let press = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: true }, &s);
        let release = event_to_midi_bytes(&ControlEvent::SliderTouch { pressed: false }, &s);
        assert_eq!(press, Some([0xB0, 70, 127]));
        assert_eq!(release, Some([0xB0, 70, 0]));
    }

    #[test]
    fn pad_aftertouch_disabled_drops_silently() {
        let bytes = event_to_midi_bytes(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 50,
            },
            &Settings::default(),
        );
        assert_eq!(bytes, None);
    }

    #[test]
    fn pad_aftertouch_enabled_emits_poly_pressure_with_inherited_note() {
        let s = settings_with_pad_pressure_enabled(0, 0, None);
        let bytes = event_to_midi_bytes(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 100,
            },
            &s,
        );
        // pads[0].hit.note default = 48
        assert_eq!(bytes, Some([0xA0, 48, 100]));
    }

    #[test]
    fn pad_aftertouch_enabled_respects_per_action_channel_and_note_override() {
        let s = settings_with_pad_pressure_enabled(0, 2, Some(60));
        let bytes = event_to_midi_bytes(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 100,
            },
            &s,
        );
        assert_eq!(bytes, Some([0xA2, 60, 100]));
    }

    #[test]
    fn channel_inherits_global_when_action_omits_it() {
        let mut s = Settings::default();
        s.global.midi_channel = MidiChannel::try_from(5).unwrap();
        let bytes = event_to_midi_bytes(
            &ControlEvent::ButtonChanged {
                index: 22,
                pressed: true,
            },
            &s,
        );
        assert_eq!(bytes, Some([0xB5, 42, 127]));
    }

    #[test]
    fn pad_note_on_off_emit_note_messages() {
        let s = Settings::default();
        let on = event_to_midi_bytes(
            &ControlEvent::PadNoteOn {
                index: 0,
                velocity: 64,
            },
            &s,
        );
        let off = event_to_midi_bytes(
            &ControlEvent::PadNoteOff {
                index: 0,
                velocity: 0,
            },
            &s,
        );
        assert_eq!(on, Some([0x90, 48, 64]));
        assert_eq!(off, Some([0x80, 48, 0]));
    }

    // Unused imports are kept to allow re-introducing per-test if needed.
    #[allow(dead_code)]
    fn _ensure_imports_used() {
        let _ = PadHitAction::Note {
            channel: None,
            note: 0,
        };
        let _ = ButtonPressAction::Cc {
            channel: None,
            cc: 0,
        };
    }
}
