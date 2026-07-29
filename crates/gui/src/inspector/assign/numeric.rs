//! Numeric field identity enum and the `State` dispatch methods that drive it.

use std::ops::RangeInclusive;

use maschine_library::controls::Buttons;

use crate::app::State;
use crate::inspector::assign::mapping::{
    displayed_channel, slider_delta, touch_with_channel, touch_with_number, touch_with_off,
    touch_with_on, with_hi, with_lo, with_step,
};

/// Upper bound for the slider LED auto-off timeout (1 hour). `0` disables
/// auto-off; values are clamped to this so the unbounded `u64` text box can't
/// push an absurd timeout into the settings.
pub const MAX_AUTO_OFF_MS: u64 = 3_600_000;

/// Every numeric Assign field, across all control types. Drives `numeric_field`,
/// `apply_numeric`, and `current_numeric` in `app.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    PadHitChannel,
    PadHitNote,
    PadPressChannel,
    PadPressNote,
    ButtonChannel,
    ButtonCc,
    EncoderChannel,
    EncoderCc,
    EncoderLo,
    EncoderHi,
    EncoderStep,
    SliderChannel,
    SliderCc,
    SliderTouchChannel,
    SliderTouchNumber,
    SliderTouchOn,
    SliderTouchOff,
    EncoderPushChannel,
    EncoderPushCc,
    EncoderTouchChannel,
    EncoderTouchCc,
    /// Special: a `u64` millisecond value, handled outside `apply_numeric`.
    SliderAutoOff,
}

impl EditField {
    /// Inclusive clamp range for the *displayed* value. Channels show 1..=16
    /// (stored 0..=15); `EncoderStep` is signed and spans the full `i8` range
    /// (the per-mode clamp lives in `with_step`); all other fields are 0..=127.
    /// Returns `None` for `SliderAutoOff`, which is a `u64` (milliseconds)
    /// handled via its own path — so it can never be truncated through this clamp.
    pub fn range(self) -> Option<RangeInclusive<i8>> {
        if self == EditField::SliderAutoOff {
            return None;
        }
        if self == EditField::EncoderStep {
            return Some(i8::MIN..=i8::MAX);
        }
        // Channel fields display 1..=16.
        Some(if self.is_channel() { 1..=16 } else { 0..=127 })
    }

    /// True for channel fields (displayed 1..=16, stored as `MidiChannel` 0..=15).
    pub fn is_channel(self) -> bool {
        use EditField::*;
        matches!(
            self,
            PadHitChannel
                | PadPressChannel
                | ButtonChannel
                | EncoderChannel
                | SliderChannel
                | SliderTouchChannel
                | EncoderPushChannel
                | EncoderTouchChannel
        )
    }
}

// ---------------------------------------------------------------------------
// impl State methods
// ---------------------------------------------------------------------------

impl State {
    /// Current displayed value of a numeric field, read from the snapshot.
    pub(crate) fn current_numeric(&self, field: EditField) -> Option<i8> {
        use EditField::*;
        let s = self.settings.as_ref()?;
        // `EncoderStep` is the one signed field; it returns early below. Every
        // other arm yields a 0..=127 byte cast losslessly to `i8` at the end.
        let value: u8 = match field {
            PadHitChannel => self.first_pad_channel().map(|c| c + 1).unwrap_or(1),
            PadHitNote => self.first_pad_note()?,
            PadPressChannel => {
                let i = self.selected_pads().first().copied()? as usize;
                match s.active_pads()[i].pressure {
                    settings::PadPressureAction::Poly { channel, .. } => displayed_channel(channel),
                    settings::PadPressureAction::Disabled => 1,
                }
            }
            PadPressNote => {
                let i = self.selected_pads().first().copied()? as usize;
                match s.active_pads()[i].pressure {
                    settings::PadPressureAction::Poly { note, .. } => note.unwrap_or(0),
                    settings::PadPressureAction::Disabled => 0,
                }
            }
            ButtonChannel => self.first_button_channel().map(|c| c + 1).unwrap_or(1),
            ButtonCc => self.first_button_cc()?,
            EncoderChannel => match s.encoder.turn {
                settings::EncoderTurnAction::Cc { channel, .. } => displayed_channel(channel),
                settings::EncoderTurnAction::Off => return None,
            },
            EncoderCc => self.encoder_cc()?,
            EncoderLo => match self.encoder_mode()? {
                settings::CcValueMode::Absolute { lo, .. } => lo,
                _ => 0,
            },
            EncoderHi => match self.encoder_mode()? {
                settings::CcValueMode::Absolute { hi, .. } => hi,
                _ => 127,
            },
            EncoderStep => return Some(self.encoder_mode()?.step()),
            SliderChannel => match s.slider.position {
                settings::SliderPositionAction::Cc { channel, .. } => displayed_channel(channel),
                settings::SliderPositionAction::Off => return None,
            },
            SliderCc => match s.slider.position {
                settings::SliderPositionAction::Cc { cc, .. } => cc,
                settings::SliderPositionAction::Off => return None,
            },
            SliderTouchChannel => match &s.slider.touch {
                settings::SliderTouchAction::Note { channel, .. }
                | settings::SliderTouchAction::Cc { channel, .. } => displayed_channel(*channel),
                settings::SliderTouchAction::Disabled => 1,
            },
            SliderTouchNumber => match &s.slider.touch {
                settings::SliderTouchAction::Note { note, .. } => *note,
                settings::SliderTouchAction::Cc { cc, .. } => *cc,
                settings::SliderTouchAction::Disabled => 0,
            },
            SliderTouchOn => match &s.slider.touch {
                settings::SliderTouchAction::Note { on_value, .. }
                | settings::SliderTouchAction::Cc { on_value, .. } => *on_value,
                settings::SliderTouchAction::Disabled => 127,
            },
            SliderTouchOff => match &s.slider.touch {
                settings::SliderTouchAction::Note { off_value, .. }
                | settings::SliderTouchAction::Cc { off_value, .. } => *off_value,
                settings::SliderTouchAction::Disabled => 0,
            },
            SliderAutoOff => 0, // handled via the string buffer, not here
            EncoderPushChannel => self
                .button_channel_at(Buttons::EncoderPress as u8)
                .map(|c| c + 1)
                .unwrap_or(1),
            EncoderPushCc => self.button_cc_at(Buttons::EncoderPress as u8)?,
            EncoderTouchChannel => self
                .button_channel_at(Buttons::EncoderTouch as u8)
                .map(|c| c + 1)
                .unwrap_or(1),
            EncoderTouchCc => self.button_cc_at(Buttons::EncoderTouch as u8)?,
        };
        Some(value as i8)
    }

    /// Apply a numeric field edit (`value` is the displayed value; channels are
    /// 1..=16 here and converted to `MidiChannel`; `EncoderStep` is signed).
    pub(crate) fn apply_numeric(&mut self, field: EditField, value: i8, persist: bool) {
        use EditField::*;
        // Every field except the signed `EncoderStep` carries a 0..=127 / 1..=16 byte.
        let v = value as u8;
        match field {
            // Channel fields dispatch to apply_channel.
            PadHitChannel | PadPressChannel | ButtonChannel | EncoderChannel | SliderChannel
            | SliderTouchChannel | EncoderPushChannel | EncoderTouchChannel => {
                self.apply_channel(field, v, persist)
            }
            PadHitNote => {
                if let Some(delta) = self.pad_hit_delta(None, Some(v)) {
                    self.send_apply(delta, persist);
                }
            }
            PadPressNote => {
                if let Some(delta) = self.pad_pressure_delta(None, Some(Some(v))) {
                    self.send_apply(delta, persist);
                }
            }
            ButtonCc => {
                if let Some(delta) = self.button_press_delta(None, Some(v)) {
                    self.send_apply(delta, persist);
                }
            }
            EncoderCc => self.apply_encoder(Some(v), None, persist),
            EncoderLo => {
                if let Some(m) = self.encoder_mode() {
                    self.apply_encoder(None, Some(with_lo(&m, v)), persist);
                }
            }
            EncoderHi => {
                if let Some(m) = self.encoder_mode() {
                    self.apply_encoder(None, Some(with_hi(&m, v)), persist);
                }
            }
            EncoderStep => {
                if let Some(m) = self.encoder_mode() {
                    self.apply_encoder(None, Some(with_step(&m, value)), persist);
                }
            }
            SliderCc => {
                let channel = self
                    .slider_position()
                    .and_then(|(_, ch)| ch)
                    .and_then(settings::MidiChannel::try_from_opt);
                self.send_apply(
                    slider_delta(
                        Some(settings::SliderPositionAction::Cc { channel, cc: v }),
                        None,
                        None,
                    ),
                    persist,
                );
            }
            SliderTouchNumber => self.apply_touch_field(|t| touch_with_number(t, v), persist),
            SliderTouchOn => self.apply_touch_field(|t| touch_with_on(t, v), persist),
            SliderTouchOff => self.apply_touch_field(|t| touch_with_off(t, v), persist),
            SliderAutoOff => {} // handled via string buffer
            EncoderPushCc => {
                self.apply_encoder_button(Buttons::EncoderPress as u8, Some(v), None, persist)
            }
            EncoderTouchCc => {
                self.apply_encoder_button(Buttons::EncoderTouch as u8, Some(v), None, persist)
            }
        }
    }

    /// Apply a channel edit for the field's control(s), dispatched per control
    /// type. `displayed` is the 1..=16 channel to set.
    pub(crate) fn apply_channel(&mut self, field: EditField, displayed: u8, persist: bool) {
        use EditField::*;
        let stored = displayed.saturating_sub(1);
        let ch = settings::MidiChannel::try_from(stored).ok();
        match field {
            PadHitChannel => {
                if let Some(delta) = self.pad_hit_delta(Some(ch), None) {
                    self.send_apply(delta, persist);
                }
            }
            PadPressChannel => {
                if let Some(delta) = self.pad_pressure_delta(Some(ch), None) {
                    self.send_apply(delta, persist);
                }
            }
            ButtonChannel => {
                if let Some(delta) = self.button_press_delta(Some(ch), None) {
                    self.send_apply(delta, persist);
                }
            }
            EncoderChannel => self.set_encoder_channel(Some(stored), persist),
            SliderChannel => {
                if let Some((cc, _)) = self.slider_position() {
                    self.send_apply(
                        slider_delta(
                            Some(settings::SliderPositionAction::Cc { channel: ch, cc }),
                            None,
                            None,
                        ),
                        persist,
                    );
                }
            }
            SliderTouchChannel => {
                self.apply_touch_field(|t| touch_with_channel(t, Some(stored)), persist)
            }
            EncoderPushChannel => self.apply_encoder_button(
                Buttons::EncoderPress as u8,
                None,
                Some(Some(stored)),
                persist,
            ),
            EncoderTouchChannel => self.apply_encoder_button(
                Buttons::EncoderTouch as u8,
                None,
                Some(Some(stored)),
                persist,
            ),
            _ => {}
        }
    }

    /// Rebuild the slider touch action from the snapshot via `f`, and send it.
    pub(crate) fn apply_touch_field(
        &mut self,
        f: impl FnOnce(&settings::SliderTouchAction) -> settings::SliderTouchAction,
        persist: bool,
    ) {
        if let Some(t) = self.slider_touch() {
            self.send_apply(slider_delta(None, Some(f(&t)), None), persist);
        }
    }
}
