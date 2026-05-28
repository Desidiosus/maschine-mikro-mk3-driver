use num_derive::FromPrimitive;

#[derive(FromPrimitive, Debug, Clone, Copy, PartialEq)]
pub enum Buttons {
    Maschine = 0,
    Star = 1,
    Browse = 2,
    Volume = 3,

    Swing = 4,
    Tempo = 5,
    Plugin = 6,
    Sampling = 7,

    Left = 8,
    Right = 9,
    Pitch = 10,
    Mod = 11,

    Perform = 12,
    Notes = 13,
    Group = 14,
    Auto = 15,

    Lock = 16,
    NoteRepeat = 17,
    Restart = 18,
    Erase = 19,

    Tap = 20,
    Follow = 21,
    Play = 22,
    Rec = 23,

    Stop = 24,
    Shift = 25,
    FixedVol = 26,
    PadMode = 27,

    Keyboard = 28,
    Chords = 29,
    Step = 30,
    Scene = 31,

    Pattern = 32,
    Events = 33,
    Variation = 34,
    Duplicate = 35,

    Select = 36,
    Solo = 37,
    Mute = 38,

    EncoderPress = 39,
    EncoderTouch = 40,
}

/// Snake-case names for every `Buttons` variant, indexed by `Buttons as usize`.
/// Used as TOML keys in the driver's settings schema.
pub const BUTTON_NAMES: [&str; 41] = [
    "maschine",
    "star",
    "browse",
    "volume",
    "swing",
    "tempo",
    "plugin",
    "sampling",
    "left",
    "right",
    "pitch",
    "mod",
    "perform",
    "notes",
    "group",
    "auto",
    "lock",
    "note_repeat",
    "restart",
    "erase",
    "tap",
    "follow",
    "play",
    "rec",
    "stop",
    "shift",
    "fixed_vol",
    "pad_mode",
    "keyboard",
    "chords",
    "step",
    "scene",
    "pattern",
    "events",
    "variation",
    "duplicate",
    "select",
    "solo",
    "mute",
    "encoder_press",
    "encoder_touch",
];

pub fn button_name(button: Buttons) -> &'static str {
    BUTTON_NAMES[button as usize]
}

pub fn button_index_from_name(name: &str) -> Option<usize> {
    BUTTON_NAMES.iter().position(|n| *n == name)
}

#[derive(FromPrimitive, Debug, Clone, Copy, PartialEq)]
pub enum PadEventType {
    NoteOn = 0x10,
    NoteOff = 0x30,
    Aftertouch = 0x40,
    PressOff = 0x20,
    PressOn = 0x00,
}

#[cfg(test)]
mod button_name_tests {
    use super::{BUTTON_NAMES, Buttons, button_index_from_name, button_name};
    use num::FromPrimitive;

    #[test]
    fn button_name_is_unique_for_each_variant() {
        let mut seen = BUTTON_NAMES.to_vec();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), BUTTON_NAMES.len(), "duplicate button name");
    }

    #[test]
    fn button_name_round_trips() {
        for idx in 0..41u8 {
            let button = Buttons::from_u8(idx).unwrap();
            let name = button_name(button);
            let parsed = button_index_from_name(name).unwrap();
            assert_eq!(parsed, idx as usize);
        }
    }

    #[test]
    fn play_button_is_called_play() {
        assert_eq!(button_name(Buttons::Play), "play");
        assert_eq!(button_index_from_name("play"), Some(Buttons::Play as usize));
    }

    #[test]
    fn unknown_name_returns_none() {
        assert_eq!(button_index_from_name("totally_not_a_button"), None);
    }
}
