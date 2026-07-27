use std::path::Path;

fn test_settings() -> Settings {
    let mut s = Settings::default();
    s.global.client_name = "Client".into();
    s.global.port_name = "Port".into();
    s.global.port_name_in = "Input".into();
    s.bridge.autoconnect_virmidi = false;
    s
}

#[test]
fn runtime_constructor_creates_midi_backend_when_seq_available() {
    if !Path::new("/dev/snd/seq").exists() {
        return;
    }

    let outputs = driver::outputs::DeviceOutputs::new();
    let soft_off = driver::soft_off::SoftOffSync::new();
    MidiBackend::new(
        &driver::shared_settings::new_shared(test_settings()),
        &outputs,
        soft_off,
        RuntimeState::default(),
        driver::ipc::new_subscriber(),
    )
    .unwrap();
}

use driver::backend::midi::{MidiBackend, MidiSink};
use driver::error::DriverResult;
use driver::events::ControlEvent;
use driver::runtime_state::RuntimeState;
use driver::settings::actions::{PadPressureAction, SliderTouchAction};
use driver::settings::{MidiChannel, Settings};

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

#[test]
fn pad_aftertouch_event_does_not_emit_when_pressure_disabled() {
    // Disabled aftertouch drops silently whether or not the pad has a held
    // note: `handle_event` resolves via stored held state when a note is
    // sounding, and falls back to the current page (same `Disabled` check)
    // otherwise. No NoteOn prelude is needed to reach either branch.
    let mut backend = MidiBackend::with_sink(Settings::default(), CapturingSink::default());
    backend
        .handle_event(
            &ControlEvent::PadAftertouch {
                index: 0,
                pressure: 100,
            },
            &RuntimeState::default(),
        )
        .unwrap();
    assert!(
        !backend.sink().sent.iter().any(|msg| msg[0] & 0xF0 == 0xA0),
        "pressure disabled must not emit an aftertouch message, got {:?}",
        backend.sink().sent
    );
}

#[test]
fn pad_aftertouch_event_emits_poly_pressure_when_enabled() {
    // Real captures (wireshark/finger_drumming_on_windows.pcapng, report id
    // 0x02) show the device ramping pressure up before crossing the NoteOn
    // threshold and back down after NoteOff, including a trailing pressure-0
    // reading with no outstanding note. `handle_event` resolves aftertouch
    // for a held note against the target recorded at press time (so a page
    // switch mid-note can't retarget it), and falls back to resolving
    // against the current page whenever there is no held note — which is
    // what this test exercises, with no NoteOn prelude needed.
    let mut settings = Settings::default();
    settings.active_pads_mut()[3].pressure = PadPressureAction::Poly {
        channel: MidiChannel::try_from(1).ok(),
        note: Some(60),
    };
    let mut backend = MidiBackend::with_sink(settings, CapturingSink::default());
    backend
        .handle_event(
            &ControlEvent::PadAftertouch {
                index: 3,
                pressure: 99,
            },
            &RuntimeState::default(),
        )
        .unwrap();
    assert_eq!(
        backend.sink().sent,
        vec![vec![0xA1, 60, 99]],
        "aftertouch resolves against the current page even with no held note"
    );
}

#[test]
fn slider_touch_event_does_not_emit_when_disabled() {
    let mut backend = MidiBackend::with_sink(Settings::default(), CapturingSink::default());
    backend
        .handle_event(
            &ControlEvent::SliderTouch { pressed: true },
            &RuntimeState::default(),
        )
        .unwrap();
    assert!(backend.sink().sent.is_empty());
}

#[test]
fn slider_touch_event_emits_note_on_and_off_when_enabled() {
    let mut settings = Settings::default();
    settings.slider.touch = SliderTouchAction::Note {
        channel: None,
        note: 60,
        on_value: 100,
        off_value: 10,
    };
    let mut backend = MidiBackend::with_sink(settings, CapturingSink::default());
    backend
        .handle_event(
            &ControlEvent::SliderTouch { pressed: true },
            &RuntimeState::default(),
        )
        .unwrap();
    backend
        .handle_event(
            &ControlEvent::SliderTouch { pressed: false },
            &RuntimeState::default(),
        )
        .unwrap();
    assert_eq!(
        backend.sink().sent,
        vec![vec![0x90, 60, 100], vec![0x80, 60, 10]]
    );
}
