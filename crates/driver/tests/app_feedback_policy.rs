use maschine_library::controls::Buttons;
use maschine_library::lights::{BUTTON_BACKLIGHT_LEVEL, Brightness, PadColors};

fn test_settings(midi_bridge_virmidi: bool) -> settings::Settings {
    let mut s = settings::Settings::default();
    s.bridge.midi_bridge_virmidi = midi_bridge_virmidi;
    s.hardware.led_brightness = 5;
    s.global.client_name = "Client".into();
    s.global.port_name = "Port".into();
    s.global.port_name_in = "Input".into();
    s
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
        BUTTON_BACKLIGHT_LEVEL
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

#[test]
fn slider_move_renders_bar_mode_with_default_color() {
    let settings = test_settings(false);
    let outputs = driver::outputs::DeviceOutputs::new();
    let event = driver::events::ControlEvent::SliderMoved {
        raw: 200,
        cc_value: 127,
    };

    driver::feedback::local::apply_local_output_feedback(&outputs, &settings, &event).unwrap();

    assert!(outputs.lights_dirty());
    outputs.with_lights(|lights| {
        for i in 0..25 {
            assert_eq!(lights.slider_byte(i), 0x7e, "led {i} should be 0x7e");
        }
    });
}

#[test]
fn slider_move_with_pan_mode_lights_around_center() {
    let mut settings = test_settings(false);
    settings.slider.led.mode = settings::SliderLedMode::Pan;
    settings.slider.led.color = PadColors::Cyan;

    let outputs = driver::outputs::DeviceOutputs::new();
    let event = driver::events::ControlEvent::SliderMoved {
        raw: 150,
        cc_value: 90,
    };

    driver::feedback::local::apply_local_output_feedback(&outputs, &settings, &event).unwrap();

    let lit = ((PadColors::Cyan as u8) << 2) | (Brightness::Normal as u8 & 0b11);

    outputs.with_lights(|lights| {
        for i in 0..12 {
            assert_eq!(lights.slider_byte(i), 0, "led {i} below center");
        }
        assert_eq!(lights.slider_byte(12), lit, "center");
    });
}
