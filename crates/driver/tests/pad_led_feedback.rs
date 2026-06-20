use driver::feedback::local::apply_local_output_feedback;
use driver::feedback::midi::apply_incoming_midi_message;
use driver::outputs::DeviceOutputs;
use driver::runtime_state::RuntimeState;
use maschine_library::lights::{Brightness, PadColors};
use settings::{PadLedColorMode, PadLedSource, Settings};

fn settings_with_pad0_source(source: PadLedSource) -> Settings {
    let mut s = Settings::default();
    s.pads[0].led.source = source;
    s
}

#[test]
fn out_source_lights_pad_on_local_hit() {
    let outputs = DeviceOutputs::new();
    let settings = settings_with_pad0_source(PadLedSource::MidiOut);
    apply_local_output_feedback(
        &outputs,
        &settings,
        &driver::events::ControlEvent::PadNoteOn {
            index: 0,
            velocity: 100,
        },
    )
    .unwrap();
    // default midi_out = Dual { on: Blue, off: Off }.
    assert_eq!(
        outputs.with_lights(|l| l.get_pad(0)),
        (PadColors::Blue, Brightness::Normal)
    );
}

#[test]
fn in_source_pad_ignores_local_hit() {
    let outputs = DeviceOutputs::new();
    let settings = settings_with_pad0_source(PadLedSource::MidiIn);
    apply_local_output_feedback(
        &outputs,
        &settings,
        &driver::events::ControlEvent::PadNoteOn {
            index: 0,
            velocity: 100,
        },
    )
    .unwrap();
    assert!(
        !outputs.lights_dirty(),
        "MidiIn pad must not light on a local hit"
    );
}

#[test]
fn in_source_lights_pad_on_incoming_note() {
    let outputs = DeviceOutputs::new();
    let mut settings = settings_with_pad0_source(PadLedSource::MidiIn);
    settings.pads[0].led.midi_in = PadLedColorMode::single(PadColors::Red);
    // default pad 0 note = 48 on channel 0.
    apply_incoming_midi_message(
        &[0x90, 48, 64],
        &outputs,
        &settings,
        &RuntimeState::default(),
    );
    assert_eq!(
        outputs.with_lights(|l| l.get_pad(0)),
        (PadColors::Red, Brightness::Normal)
    );
}

#[test]
fn out_source_pad_ignores_incoming_note() {
    let outputs = DeviceOutputs::new();
    let settings = settings_with_pad0_source(PadLedSource::MidiOut);
    apply_incoming_midi_message(
        &[0x90, 48, 64],
        &outputs,
        &settings,
        &RuntimeState::default(),
    );
    assert!(
        !outputs.lights_dirty(),
        "MidiOut pad must not light on incoming MIDI"
    );
}
