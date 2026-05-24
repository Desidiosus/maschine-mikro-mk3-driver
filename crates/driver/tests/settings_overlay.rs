use driver::settings::actions::{
    ButtonPressAction, PadHitAction, PadPressureAction, SliderTouchAction,
};
use driver::settings::{MidiChannel, PartialSettings, Settings};
use maschine_library::controls::Buttons;

fn merged_from(toml_str: &str) -> Settings {
    let partial: PartialSettings = toml::from_str(toml_str).expect("partial deserialize");
    Settings::default().merge_overrides(partial)
}

#[test]
fn enabling_pad_5_aftertouch_leaves_others_disabled() {
    let merged = merged_from(
        r#"
[pads.5.pressure]
type = "poly"
"#,
    );

    assert_eq!(
        merged.pads[5].pressure,
        PadPressureAction::Poly {
            channel: None,
            note: None
        }
    );
    assert_eq!(merged.pads[0].pressure, PadPressureAction::Disabled);
    assert_eq!(merged.pads[15].pressure, PadPressureAction::Disabled);
}

#[test]
fn remapping_play_button_cc_leaves_others_at_default() {
    let merged = merged_from(
        r#"
[buttons.play.press]
type = "cc"
cc = 99
"#,
    );

    match &merged.buttons[Buttons::Play as usize].press {
        ButtonPressAction::Cc { cc, .. } => assert_eq!(*cc, 99),
    }
    match &merged.buttons[Buttons::Stop as usize].press {
        ButtonPressAction::Cc { cc, .. } => assert_eq!(*cc, 44),
    }
}

#[test]
fn enabling_slider_touch_as_cc_with_values() {
    let merged = merged_from(
        r#"
[slider.touch]
type = "cc"
cc = 70
on_value = 100
off_value = 5
"#,
    );

    assert_eq!(
        merged.slider.touch,
        SliderTouchAction::Cc {
            channel: None,
            cc: 70,
            on_value: 100,
            off_value: 5
        }
    );
}

#[test]
fn per_action_channel_override_does_not_disturb_global() {
    let merged = merged_from(
        r#"
[global]
midi_channel = 3

[pads.0.hit]
type = "note"
note = 48
channel = 1
"#,
    );

    assert_eq!(
        merged.global.midi_channel,
        MidiChannel::try_from(3).unwrap()
    );
    match &merged.pads[0].hit {
        PadHitAction::Note { channel, note } => {
            assert_eq!(*note, 48);
            assert_eq!(*channel, MidiChannel::try_from(1).ok());
        }
    }
}
