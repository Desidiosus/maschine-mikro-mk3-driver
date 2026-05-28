use maschine_library::lights::Brightness;
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::midi::apply_incoming_midi_message;
use crate::outputs::DeviceOutputs;
use crate::settings::{MidiMapping, Settings};
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
    midi_mapping: MidiMapping,
    sink: S,
    _input: Option<MidiInputConnection<DeviceOutputs>>,
}

impl MidiBackend {
    pub fn new(settings: &Settings, outputs: &DeviceOutputs) -> DriverResult<Self> {
        let sink = MidiOutput::new(&settings.client_name)
            .map_err(|err| DriverError::Midi(format!("couldn't open MIDI output: {err}")))?
            .create_virtual(&settings.port_name)
            .map_err(|err| {
                DriverError::Midi(format!("couldn't create virtual output port: {err}"))
            })?;

        let input = create_midi_input(settings, outputs.clone())?;

        if settings.midi_bridge_virmidi && settings.autoconnect_virmidi {
            try_autoconnect_virmidi(settings)?;
        }

        Ok(Self {
            midi_mapping: settings.midi.clone(),
            sink,
            _input: Some(input),
        })
    }
}

impl<S: MidiSink> MidiBackend<S> {
    /// Construct a backend around an arbitrary sink, without a MIDI input port.
    pub fn with_sink(midi_mapping: MidiMapping, sink: S) -> Self {
        Self {
            midi_mapping,
            sink,
            _input: None,
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn handle_event(&mut self, event: &ControlEvent) -> DriverResult<()> {
        let bytes = event_to_midi_bytes(event, &self.midi_mapping)
            .ok_or_else(|| DriverError::Midi(format!("unsupported control event: {event:?}")))?;

        self.sink.send(&bytes)
    }
}

fn create_midi_input(
    settings: &Settings,
    outputs: DeviceOutputs,
) -> DriverResult<MidiInputConnection<DeviceOutputs>> {
    let midi_mapping = settings.midi.clone();
    let backlight_enabled = settings.backlight_buttons;
    let backlight_brightness = settings.backlight_brightness.as_light_brightness();
    let client_name = format!("{} In", settings.client_name);

    MidiInput::new(&client_name)
        .map_err(|err| DriverError::Midi(format!("couldn't open MIDI input: {err}")))?
        .create_virtual(
            &settings.port_name_in,
            move |_timestamp, message, outputs| {
                apply_incoming_midi_message(
                    message,
                    outputs,
                    &midi_mapping,
                    backlight_enabled,
                    backlight_brightness,
                );
            },
            outputs,
        )
        .map_err(|err| DriverError::Midi(format!("couldn't create virtual input port: {err}")))
}

pub fn event_to_midi_bytes(event: &ControlEvent, mapping: &MidiMapping) -> Option<[u8; 3]> {
    let status_base = 0xB0 | mapping.channel.as_u8();
    let note_on_status = 0x90 | mapping.channel.as_u8();
    let note_off_status = 0x80 | mapping.channel.as_u8();

    match event {
        ControlEvent::ButtonChanged { index, pressed } => Some([
            status_base,
            *mapping.button_ccs.get(*index)?,
            if *pressed { 127 } else { 0 },
        ]),
        ControlEvent::EncoderTurn { cc_value, .. } => {
            Some([status_base, mapping.encoder_cc, *cc_value])
        }
        ControlEvent::SliderMoved { cc_value, .. } => {
            Some([status_base, mapping.slider_cc, *cc_value])
        }
        ControlEvent::PadNoteOn { index, velocity } => {
            Some([note_on_status, *mapping.pad_notes.get(*index)?, *velocity])
        }
        ControlEvent::PadNoteOff { index, velocity } => {
            Some([note_off_status, *mapping.pad_notes.get(*index)?, *velocity])
        }
    }
}

pub fn pad_index_for_message(mapping: &MidiMapping, channel: u8, note: u8) -> Option<usize> {
    message_index(&mapping.pad_notes, mapping.channel.as_u8(), channel, note)
}

pub fn button_index_for_message(mapping: &MidiMapping, channel: u8, cc: u8) -> Option<usize> {
    message_index(&mapping.button_ccs, mapping.channel.as_u8(), channel, cc)
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

fn message_index(
    values: &[u8],
    expected_channel: u8,
    channel: u8,
    message_value: u8,
) -> Option<usize> {
    (channel == expected_channel)
        .then_some(values)
        .and_then(|values| values.iter().position(|value| *value == message_value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct CapturingSink {
        sent: Vec<Vec<u8>>,
    }

    impl MidiSink for CapturingSink {
        fn send(&mut self, bytes: &[u8]) -> DriverResult<()> {
            self.sent.push(bytes.to_vec());
            Ok(())
        }
    }

    struct FailingSink;

    impl MidiSink for FailingSink {
        fn send(&mut self, _bytes: &[u8]) -> DriverResult<()> {
            Err(DriverError::Midi("simulated send failure".into()))
        }
    }

    fn default_mapping() -> MidiMapping {
        Settings::default().midi
    }

    #[test]
    fn handle_event_sends_button_cc_to_sink() {
        let mut backend = MidiBackend::with_sink(default_mapping(), CapturingSink::default());

        backend
            .handle_event(&ControlEvent::ButtonChanged {
                index: 22,
                pressed: true,
            })
            .unwrap();

        assert_eq!(backend.sink().sent, vec![vec![0xB0, 42, 127]]);
    }

    #[test]
    fn handle_event_sends_pad_note_on_off_with_velocity() {
        let mut backend = MidiBackend::with_sink(default_mapping(), CapturingSink::default());

        backend
            .handle_event(&ControlEvent::PadNoteOn {
                index: 0,
                velocity: 64,
            })
            .unwrap();
        backend
            .handle_event(&ControlEvent::PadNoteOff {
                index: 0,
                velocity: 0,
            })
            .unwrap();

        assert_eq!(
            backend.sink().sent,
            vec![vec![0x90, 48, 64], vec![0x80, 48, 0]]
        );
    }

    #[test]
    fn handle_event_honors_configured_midi_channel() {
        let mapping = MidiMapping {
            channel: 2u8.try_into().unwrap(),
            ..default_mapping()
        };
        let mut backend = MidiBackend::with_sink(mapping, CapturingSink::default());

        backend
            .handle_event(&ControlEvent::EncoderTurn {
                delta: 1,
                cc_value: 65,
            })
            .unwrap();

        assert_eq!(backend.sink().sent, vec![vec![0xB2, 1, 65]]);
    }

    #[test]
    fn handle_event_returns_midi_error_when_sink_fails() {
        let mut backend = MidiBackend::with_sink(default_mapping(), FailingSink);

        let err = backend
            .handle_event(&ControlEvent::SliderMoved {
                raw: 100,
                cc_value: 63,
            })
            .unwrap_err();

        match err {
            DriverError::Midi(message) => assert!(message.contains("simulated send failure")),
            other => panic!("expected DriverError::Midi, got {other:?}"),
        }
    }
}
