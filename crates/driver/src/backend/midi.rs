use maschine_library::lights::Brightness;
use midir::os::unix::{VirtualInput, VirtualOutput};
use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::error::{DriverError, DriverResult};
use crate::events::ControlEvent;
use crate::feedback::midi::apply_incoming_midi_message;
use crate::outputs::DeviceOutputs;
use crate::settings::Settings;
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

#[allow(dead_code)]
pub fn event_to_midi_bytes(_event: &ControlEvent, _settings: &Settings) -> Option<[u8; 3]> {
    // Real implementation arrives in Task 15.
    Some([0, 0, 0])
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

// Tests rewritten in Tasks 15 / 16.
