use maschine_library::controls::Buttons;
use maschine_library::lights::{Brightness, PadColors};

fn test_settings(midi_bridge_virmidi: bool) -> driver::settings::Settings {
    driver::settings::Settings {
        midi_bridge_virmidi,
        backlight_buttons: true,
        backlight_brightness: driver::settings::BacklightBrightness::Dim,
        client_name: "Client".into(),
        port_name: "Port".into(),
        port_name_in: "Input".into(),
        ..driver::settings::Settings::default()
    }
}

#[test]
fn midi_button_release_applies_local_backlight() {
    let settings = test_settings(false);
    let outputs = driver::outputs::DeviceOutputs::new();
    let event = driver::events::ControlEvent::ButtonChanged {
        index: Buttons::Play as usize,
        pressed: false,
    };

    driver::feedback::local::apply_local_output_feedback(&outputs, &settings, &event).unwrap();

    assert!(outputs.lights_dirty());
    assert_eq!(
        outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
        Brightness::Dim
    );
}

#[test]
fn bridged_midi_button_release_does_not_apply_local_backlight() {
    let settings = test_settings(true);
    let outputs = driver::outputs::DeviceOutputs::new();
    let event = driver::events::ControlEvent::ButtonChanged {
        index: Buttons::Play as usize,
        pressed: false,
    };

    driver::feedback::local::apply_local_output_feedback(&outputs, &settings, &event).unwrap();

    assert!(!outputs.lights_dirty());
    assert_eq!(
        outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
        Brightness::Off
    );
}

#[test]
fn pad_note_on_applies_local_pad_feedback() {
    let settings = test_settings(false);
    let outputs = driver::outputs::DeviceOutputs::new();
    let event = driver::events::ControlEvent::PadNoteOn {
        index: 0,
        velocity: 64,
    };

    driver::feedback::local::apply_local_output_feedback(&outputs, &settings, &event).unwrap();

    assert!(outputs.lights_dirty());
    assert_eq!(
        outputs.with_lights(|lights| lights.get_pad(0)),
        (PadColors::Blue, Brightness::Normal)
    );
}

#[test]
fn pad_note_off_clears_local_pad_feedback() {
    let settings = test_settings(false);
    let outputs = driver::outputs::DeviceOutputs::new();

    driver::feedback::local::apply_local_output_feedback(
        &outputs,
        &settings,
        &driver::events::ControlEvent::PadNoteOn {
            index: 0,
            velocity: 64,
        },
    )
    .unwrap();
    let _ = outputs.take_lights_dirty();

    driver::feedback::local::apply_local_output_feedback(
        &outputs,
        &settings,
        &driver::events::ControlEvent::PadNoteOff {
            index: 0,
            velocity: 0,
        },
    )
    .unwrap();

    assert!(outputs.lights_dirty());
    assert_eq!(
        outputs.with_lights(|lights| lights.get_pad(0)),
        (PadColors::Off, Brightness::Off)
    );
}
