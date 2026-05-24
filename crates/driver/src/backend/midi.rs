use maschine_library::lights::Brightness;
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::midi::apply_incoming_midi_message;
use crate::outputs::DeviceOutputs;
use crate::settings::{MidiMapping, Settings};
use crate::virmidi_bridge::try_autoconnect_virmidi;

pub struct MidiBackend {
    midi_mapping: MidiMapping,
    output: MidiOutputConnection,
    _input: MidiInputConnection<DeviceOutputs>,
}

impl MidiBackend {
    pub fn new(settings: &Settings, outputs: &DeviceOutputs) -> DriverResult<Self> {
        let output = MidiOutput::new(&settings.client_name)
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
            output,
            _input: input,
        })
    }

    pub fn handle_event(&mut self, event: &ControlEvent) -> DriverResult<()> {
        let bytes = event_to_midi_bytes(event, &self.midi_mapping)
            .ok_or_else(|| DriverError::Midi(format!("unsupported control event: {event:?}")))?;

        self.output
            .send(&bytes)
            .map_err(|err| DriverError::Midi(format!("failed to send MIDI message: {err}")))
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
