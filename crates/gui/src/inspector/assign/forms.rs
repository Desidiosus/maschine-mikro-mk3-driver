//! Per-control Assign form type definitions: tab enum, pick-list variants,
//! and helpers that classify action enums into their pick-list equivalents.

use settings::{
    ButtonPressAction, CcValueMode, EncoderTurnAction, PadHitAction, PadPressureAction,
    SliderPositionAction, SliderTouchAction,
};

/// Which sub-action slot the Assign form is showing.
/// A = Turn/Hit/Position, B = Push/Press/Touch, C = encoder Touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignTab {
    A,
    B,
    C,
}

/// Encoder turn mode `pick_list` options (the `CcValueMode` discriminant).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderModeKind {
    Absolute,
    Relative,
    RelativeOffset,
}

impl EncoderModeKind {
    pub const ALL: [EncoderModeKind; 3] = [
        EncoderModeKind::Absolute,
        EncoderModeKind::Relative,
        EncoderModeKind::RelativeOffset,
    ];

    pub fn of(mode: &CcValueMode) -> EncoderModeKind {
        match mode {
            CcValueMode::Absolute { .. } => EncoderModeKind::Absolute,
            CcValueMode::Relative { .. } => EncoderModeKind::Relative,
            CcValueMode::RelativeOffset { .. } => EncoderModeKind::RelativeOffset,
        }
    }

    pub fn to_mode(self, prev: &CcValueMode) -> CcValueMode {
        // Carry the previous step across the switch, then clamp it into the new
        // variant's range (a switch to Absolute also resets the lo/hi window).
        let step = prev.step();
        match self {
            EncoderModeKind::Absolute => CcValueMode::Absolute {
                lo: 0,
                hi: 127,
                step,
                wrap: false,
            },
            EncoderModeKind::Relative => CcValueMode::Relative { step },
            EncoderModeKind::RelativeOffset => CcValueMode::RelativeOffset { step },
        }
        .with_clamped_step()
    }
}

impl std::fmt::Display for EncoderModeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            EncoderModeKind::Absolute => "Absolute",
            EncoderModeKind::Relative => "Relative",
            EncoderModeKind::RelativeOffset => "Relative-Offset",
        })
    }
}

/// Slider touch `pick_list` options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderTouchKind {
    Disabled,
    Note,
    Cc,
}

impl SliderTouchKind {
    pub const ALL: [SliderTouchKind; 3] = [
        SliderTouchKind::Disabled,
        SliderTouchKind::Note,
        SliderTouchKind::Cc,
    ];

    pub fn of(action: &SliderTouchAction) -> SliderTouchKind {
        match action {
            SliderTouchAction::Disabled => SliderTouchKind::Disabled,
            SliderTouchAction::Note { .. } => SliderTouchKind::Note,
            SliderTouchAction::Cc { .. } => SliderTouchKind::Cc,
        }
    }

    pub fn to_action(self, prev: &SliderTouchAction) -> SliderTouchAction {
        let (channel, number, on, off) = match prev {
            SliderTouchAction::Disabled => (None, 0, 127, 0),
            SliderTouchAction::Note {
                channel,
                note,
                on_value,
                off_value,
            } => (*channel, *note, *on_value, *off_value),
            SliderTouchAction::Cc {
                channel,
                cc,
                on_value,
                off_value,
            } => (*channel, *cc, *on_value, *off_value),
        };
        match self {
            SliderTouchKind::Disabled => SliderTouchAction::Disabled,
            SliderTouchKind::Note => SliderTouchAction::Note {
                channel,
                note: number,
                on_value: on,
                off_value: off,
            },
            SliderTouchKind::Cc => SliderTouchAction::Cc {
                channel,
                cc: number,
                on_value: on,
                off_value: off,
            },
        }
    }
}

impl std::fmt::Display for SliderTouchKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SliderTouchKind::Disabled => "Off",
            SliderTouchKind::Note => "Note",
            SliderTouchKind::Cc => "CC",
        })
    }
}

/// `Type` options for a Control-Change-or-Off slot (button, encoder turn,
/// encoder push/touch, slider position).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CcType {
    ControlChange,
    Off,
}
impl CcType {
    pub const ALL: [CcType; 2] = [CcType::ControlChange, CcType::Off];
}
impl std::fmt::Display for CcType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CcType::ControlChange => "Control Change",
            CcType::Off => "Off",
        })
    }
}

/// `Type` options for the pad Hit slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadHitType {
    Note,
    Off,
}
impl PadHitType {
    pub const ALL: [PadHitType; 2] = [PadHitType::Note, PadHitType::Off];
    pub fn of(a: &PadHitAction) -> PadHitType {
        match a {
            PadHitAction::Note { .. } => PadHitType::Note,
            PadHitAction::Off => PadHitType::Off,
        }
    }
}
impl std::fmt::Display for PadHitType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PadHitType::Note => "Note",
            PadHitType::Off => "Off",
        })
    }
}

/// `Type` options for the pad Press slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PadPressType {
    PolyPressure,
    Off,
}
impl PadPressType {
    pub const ALL: [PadPressType; 2] = [PadPressType::PolyPressure, PadPressType::Off];
    pub fn of(a: &PadPressureAction) -> PadPressType {
        match a {
            PadPressureAction::Poly { .. } => PadPressType::PolyPressure,
            PadPressureAction::Disabled => PadPressType::Off,
        }
    }
}
impl std::fmt::Display for PadPressType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PadPressType::PolyPressure => "Poly Pressure",
            PadPressType::Off => "Off",
        })
    }
}

/// Which LED source the pad Assign form is editing. Independent of `AssignTab`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LedTab {
    In,
    Out,
}

pub fn cc_type_of_button(a: &ButtonPressAction) -> CcType {
    match a {
        ButtonPressAction::Cc { .. } => CcType::ControlChange,
        ButtonPressAction::Off => CcType::Off,
    }
}
pub fn cc_type_of_encoder(a: &EncoderTurnAction) -> CcType {
    match a {
        EncoderTurnAction::Cc { .. } => CcType::ControlChange,
        EncoderTurnAction::Off => CcType::Off,
    }
}
pub fn cc_type_of_position(a: &SliderPositionAction) -> CcType {
    match a {
        SliderPositionAction::Cc { .. } => CcType::ControlChange,
        SliderPositionAction::Off => CcType::Off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::SliderTouchAction;

    #[test]
    fn encoder_kind_roundtrips_and_carries_step() {
        let rel = CcValueMode::Relative { step: 9 };
        assert_eq!(EncoderModeKind::of(&rel), EncoderModeKind::Relative);
        assert_eq!(
            EncoderModeKind::RelativeOffset.to_mode(&rel),
            CcValueMode::RelativeOffset { step: 9 }
        );
        let clamped = EncoderModeKind::Relative.to_mode(&CcValueMode::Absolute {
            lo: 0,
            hi: 127,
            step: 100,
            wrap: false,
        });
        assert_eq!(
            clamped,
            CcValueMode::Relative {
                step: CcValueMode::RELATIVE_STEP_MAX
            }
        );

        // A negative step (reversed encoder) is preserved through a mode switch.
        let reversed = CcValueMode::Relative { step: -9 };
        assert_eq!(
            EncoderModeKind::RelativeOffset.to_mode(&reversed),
            CcValueMode::RelativeOffset { step: -9 }
        );
    }

    #[test]
    fn slider_touch_kind_carries_params() {
        let note = SliderTouchAction::Note {
            channel: None,
            note: 64,
            on_value: 100,
            off_value: 3,
        };
        assert_eq!(SliderTouchKind::of(&note), SliderTouchKind::Note);
        assert_eq!(
            SliderTouchKind::Cc.to_action(&note),
            SliderTouchAction::Cc {
                channel: None,
                cc: 64,
                on_value: 100,
                off_value: 3
            }
        );
    }
}
