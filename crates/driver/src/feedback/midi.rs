use maschine_library::lights::PadColors;
use maschine_library::screen::{ScreenCommand, parse_sysex_command, render_centered_text};

use crate::outputs::DeviceOutputs;
use crate::settings::Settings;

pub fn apply_incoming_midi_message(
    _message: &[u8],
    _outputs: &DeviceOutputs,
    _settings: &Settings,
) {
    // Real implementation arrives in Task 17.
}

#[allow(dead_code)]
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

pub fn pad_color_from_velocity(velocity: u8) -> PadColors {
    match velocity {
        0 => PadColors::Off,
        1..=7 => PadColors::Red,
        8..=14 => PadColors::Orange,
        15..=21 => PadColors::LightOrange,
        22..=28 => PadColors::WarmYellow,
        29..=35 => PadColors::Yellow,
        36..=42 => PadColors::Lime,
        43..=49 => PadColors::Green,
        50..=56 => PadColors::Mint,
        57..=63 => PadColors::Cyan,
        64..=70 => PadColors::Turquoise,
        71..=77 => PadColors::Blue,
        78..=84 => PadColors::Plum,
        85..=91 => PadColors::Violet,
        92..=98 => PadColors::Purple,
        99..=105 => PadColors::Magenta,
        106..=112 => PadColors::Fuchsia,
        _ => PadColors::White,
    }
}

// Tests rewritten in Task 17.
