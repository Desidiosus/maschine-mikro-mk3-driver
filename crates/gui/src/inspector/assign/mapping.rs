//! Pure assign delta builders and the `State` methods that call them.
//!
//! Free functions (`with_*`, `touch_*`, `pad_delta`, `pads_map`, `buttons_map`,
//! `button_delta`, `encoder_delta`, `slider_delta`) build sparse
//! `PartialSettings` values. The `impl State` block wires them to the current
//! settings snapshot.

use settings::PartialSettings;
use settings::partial::{
    PartialButtonConfig, PartialEncoderConfig, PartialPadConfig, PartialSliderConfig,
    PartialSliderLedSettings,
};
use settings::{
    ButtonPressAction, EncoderTurnAction, PadHitAction, PadPressureAction, SliderPositionAction,
    SliderTouchAction,
};

use crate::app::State;
use crate::inspector::assign::forms::CcType;
use crate::inspector::assign::multi::{MultiValue, fold};

// ---------------------------------------------------------------------------
// Free functions: CcValueMode helpers
// ---------------------------------------------------------------------------

pub(crate) fn with_step(mode: &settings::CcValueMode, step: i8) -> settings::CcValueMode {
    use settings::CcValueMode::*;
    // Keep the variant (and Absolute's lo/hi/wrap), swap in the new step, then
    // clamp it into that variant's range via the schema's single source of truth.
    match mode {
        Absolute { lo, hi, wrap, .. } => Absolute {
            lo: *lo,
            hi: *hi,
            step,
            wrap: *wrap,
        },
        Relative { .. } => Relative { step },
        RelativeOffset { .. } => RelativeOffset { step },
    }
    .with_clamped_step()
}

/// Displayed channel (1..=16) for a per-control override, defaulting an unset
/// channel to 1.
pub(crate) fn displayed_channel(channel: Option<settings::MidiChannel>) -> u8 {
    channel.map(|c| c.as_u8() + 1).unwrap_or(1)
}

/// Rebuild an `Absolute` mode, mutating its `(lo, hi, step, wrap)` via `f`; any
/// other variant is returned unchanged.
pub(crate) fn with_absolute(
    mode: &settings::CcValueMode,
    f: impl FnOnce(&mut u8, &mut u8, &mut i8, &mut bool),
) -> settings::CcValueMode {
    if let settings::CcValueMode::Absolute { lo, hi, step, wrap } = mode {
        let (mut lo, mut hi, mut step, mut wrap) = (*lo, *hi, *step, *wrap);
        f(&mut lo, &mut hi, &mut step, &mut wrap);
        settings::CcValueMode::Absolute { lo, hi, step, wrap }
    } else {
        mode.clone()
    }
}

pub(crate) fn with_lo(mode: &settings::CcValueMode, lo: u8) -> settings::CcValueMode {
    with_absolute(mode, |l, hi, _, _| *l = lo.min(*hi))
}

pub(crate) fn with_hi(mode: &settings::CcValueMode, hi: u8) -> settings::CcValueMode {
    with_absolute(mode, |lo, h, _, _| *h = hi.max(*lo))
}

pub(crate) fn with_wrap(mode: &settings::CcValueMode, wrap: bool) -> settings::CcValueMode {
    with_absolute(mode, |_, _, _, w| *w = wrap)
}

// ---------------------------------------------------------------------------
// Free functions: SliderTouchAction helpers
// ---------------------------------------------------------------------------

/// Rebuild a touch action, mutating its `(channel, number, on_value, off_value)`
/// via `f` — `number` is the note for `Note` and the cc for `Cc`. `Disabled` is
/// returned unchanged.
pub(crate) fn touch_map(
    t: &settings::SliderTouchAction,
    f: impl FnOnce(&mut Option<settings::MidiChannel>, &mut u8, &mut u8, &mut u8),
) -> settings::SliderTouchAction {
    use settings::SliderTouchAction::*;
    match t {
        Disabled => Disabled,
        Note {
            channel,
            note,
            on_value,
            off_value,
        } => {
            let (mut channel, mut note, mut on_value, mut off_value) =
                (*channel, *note, *on_value, *off_value);
            f(&mut channel, &mut note, &mut on_value, &mut off_value);
            Note {
                channel,
                note,
                on_value,
                off_value,
            }
        }
        Cc {
            channel,
            cc,
            on_value,
            off_value,
        } => {
            let (mut channel, mut cc, mut on_value, mut off_value) =
                (*channel, *cc, *on_value, *off_value);
            f(&mut channel, &mut cc, &mut on_value, &mut off_value);
            Cc {
                channel,
                cc,
                on_value,
                off_value,
            }
        }
    }
}

pub(crate) fn touch_with_number(
    t: &settings::SliderTouchAction,
    n: u8,
) -> settings::SliderTouchAction {
    touch_map(t, |_, num, _, _| *num = n)
}

pub(crate) fn touch_with_on(t: &settings::SliderTouchAction, v: u8) -> settings::SliderTouchAction {
    touch_map(t, |_, _, on, _| *on = v)
}

pub(crate) fn touch_with_off(
    t: &settings::SliderTouchAction,
    v: u8,
) -> settings::SliderTouchAction {
    touch_map(t, |_, _, _, off| *off = v)
}

pub(crate) fn touch_with_channel(
    t: &settings::SliderTouchAction,
    ch: Option<u8>,
) -> settings::SliderTouchAction {
    let channel = ch.and_then(|v| settings::MidiChannel::try_from(v).ok());
    touch_map(t, |c, _, _, _| *c = channel)
}

// ---------------------------------------------------------------------------
// Free functions: PartialSettings delta builders
// ---------------------------------------------------------------------------

/// Delta built per-pad: `f(internal_index)` yields that pad's partial config, or
/// `None` to leave it untouched.
pub fn pads_map(indices: &[u8], f: impl Fn(u8) -> Option<PartialPadConfig>) -> PartialSettings {
    let mut pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
    for &i in indices {
        if (i as usize) < 16 {
            pads[i as usize] = f(i);
        }
    }
    PartialSettings {
        pads: Some(pads),
        ..Default::default()
    }
}

/// Delta built per-button: `f(index)` yields that button's partial config, or
/// `None` to leave it untouched.
pub fn buttons_map(
    indices: &[u8],
    f: impl Fn(u8) -> Option<PartialButtonConfig>,
) -> PartialSettings {
    let mut buttons: [Option<PartialButtonConfig>; 41] = std::array::from_fn(|_| None);
    for &i in indices {
        if (i as usize) < 41 {
            buttons[i as usize] = f(i);
        }
    }
    PartialSettings {
        buttons: Some(buttons),
        ..Default::default()
    }
}

/// Delta setting the given internal pad indices' hit/pressure (whichever is `Some`).
pub fn pad_delta(
    indices: &[u8],
    hit: Option<PadHitAction>,
    pressure: Option<PadPressureAction>,
) -> PartialSettings {
    pads_map(indices, |_| {
        Some(PartialPadConfig {
            hit: hit.clone(),
            pressure: pressure.clone(),
            led: None,
        })
    })
}

/// Delta setting the given button indices' press action.
pub fn button_delta(indices: &[u8], press: ButtonPressAction) -> PartialSettings {
    buttons_map(indices, |_| {
        Some(PartialButtonConfig {
            press: Some(press.clone()),
        })
    })
}

/// Delta setting the encoder's turn action.
pub fn encoder_delta(turn: EncoderTurnAction) -> PartialSettings {
    PartialSettings {
        encoder: Some(PartialEncoderConfig { turn: Some(turn) }),
        ..Default::default()
    }
}

/// Delta setting whichever of the slider's leaves are `Some`.
pub fn slider_delta(
    position: Option<SliderPositionAction>,
    touch: Option<SliderTouchAction>,
    led: Option<PartialSliderLedSettings>,
) -> PartialSettings {
    PartialSettings {
        slider: Some(PartialSliderConfig {
            position,
            touch,
            led,
        }),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Free functions: canonical per-control defaults
//
// Switching a control into a param-carrying mode resets it to that control's
// schema default (`Settings::default()`), so e.g. enabling an Off pad gives its
// intended note rather than a flat literal.
// ---------------------------------------------------------------------------

/// Delta resetting each given pad's Hit to that pad's schema-default note.
pub(crate) fn default_pad_hit_delta(pads: &[u8]) -> PartialSettings {
    let def = settings::Settings::default();
    pads_map(pads, |i| {
        Some(PartialPadConfig {
            hit: Some(def.pads[i as usize].hit.clone()),
            pressure: None,
            led: None,
        })
    })
}

/// Delta resetting each given button's press to that button's schema-default CC.
pub(crate) fn default_button_press_delta(buttons: &[u8]) -> PartialSettings {
    let def = settings::Settings::default();
    buttons_map(buttons, |i| {
        Some(PartialButtonConfig {
            press: Some(def.buttons[i as usize].press.clone()),
        })
    })
}

/// The schema-default press action for a single button (its default CC).
pub(crate) fn default_button_press(btn: u8) -> ButtonPressAction {
    settings::Settings::default().buttons[btn as usize]
        .press
        .clone()
}

/// The schema-default encoder turn action (default CC + Relative mode).
pub(crate) fn default_encoder_turn() -> EncoderTurnAction {
    settings::Settings::default().encoder.turn
}

/// The schema-default slider position action (its default CC).
pub(crate) fn default_slider_position() -> SliderPositionAction {
    settings::Settings::default().slider.position
}

// ---------------------------------------------------------------------------
// impl State methods
// ---------------------------------------------------------------------------

impl State {
    pub(crate) fn first_pad_note(&self) -> Option<u8> {
        let i = self.selected_pads().first().copied()? as usize;
        let s = self.settings.as_ref()?;
        match s.pads[i].hit {
            settings::PadHitAction::Note { note, .. } => Some(note),
            settings::PadHitAction::Off => None,
        }
    }

    pub(crate) fn first_pad_channel(&self) -> Option<u8> {
        let i = self.selected_pads().first().copied()? as usize;
        let s = self.settings.as_ref()?;
        match s.pads[i].hit {
            settings::PadHitAction::Note { channel, .. } => channel.map(|c| c.as_u8()),
            settings::PadHitAction::Off => None,
        }
    }

    pub(crate) fn first_button_cc(&self) -> Option<u8> {
        let i = self.selected_buttons().first().copied()? as usize;
        let s = self.settings.as_ref()?;
        match s.buttons[i].press {
            settings::ButtonPressAction::Cc { cc, .. } => Some(cc),
            settings::ButtonPressAction::Off => None,
        }
    }

    pub(crate) fn first_button_channel(&self) -> Option<u8> {
        let i = self.selected_buttons().first().copied()? as usize;
        let s = self.settings.as_ref()?;
        match s.buttons[i].press {
            settings::ButtonPressAction::Cc { channel, .. } => channel.map(|c| c.as_u8()),
            settings::ButtonPressAction::Off => None,
        }
    }

    /// Fold one field across a selection into a shared value or `Differ`.
    /// `extract` returns `None` for a control whose action doesn't carry the
    /// field (so a Type row that reads it folds an all-Off selection to `Differ`).
    fn fold_selected<T: PartialEq>(
        &self,
        selection: &[u8],
        extract: impl Fn(&settings::Settings, usize) -> Option<T>,
    ) -> MultiValue<T> {
        let Some(s) = self.settings.as_ref() else {
            return MultiValue::Differ;
        };
        fold(selection.iter().filter_map(|&i| extract(s, i as usize)))
    }

    /// The shared button action "type" across the selection: `Same(true)`=all CC,
    /// `Same(false)`=all Off, `Differ`=mixed.
    pub(crate) fn buttons_cc_type(&self) -> MultiValue<bool> {
        self.fold_selected(&self.selected_buttons(), |s, i| {
            Some(matches!(
                s.buttons[i].press,
                settings::ButtonPressAction::Cc { .. }
            ))
        })
    }

    /// Shared CC number across selected buttons (only meaningful when all are CC).
    pub(crate) fn buttons_cc(&self) -> MultiValue<u8> {
        self.fold_selected(&self.selected_buttons(), |s, i| match s.buttons[i].press {
            settings::ButtonPressAction::Cc { cc, .. } => Some(cc),
            settings::ButtonPressAction::Off => None,
        })
    }

    /// Shared channel (displayed 1..=16) across selected buttons.
    pub(crate) fn buttons_channel(&self) -> MultiValue<u8> {
        self.fold_selected(&self.selected_buttons(), |s, i| match s.buttons[i].press {
            settings::ButtonPressAction::Cc { channel, .. } => Some(displayed_channel(channel)),
            settings::ButtonPressAction::Off => None,
        })
    }

    /// The shared pad Hit "type" across the selection: `Same(true)`=all Note,
    /// `Same(false)`=all Off, `Differ`=mixed.
    pub(crate) fn pads_hit_type(&self) -> MultiValue<bool> {
        self.fold_selected(&self.selected_pads(), |s, i| {
            Some(matches!(s.pads[i].hit, settings::PadHitAction::Note { .. }))
        })
    }

    /// Shared Hit note across selected pads (only meaningful when all are Note).
    pub(crate) fn pads_hit_note(&self) -> MultiValue<u8> {
        self.fold_selected(&self.selected_pads(), |s, i| match s.pads[i].hit {
            settings::PadHitAction::Note { note, .. } => Some(note),
            settings::PadHitAction::Off => None,
        })
    }

    /// Shared Hit channel (displayed 1..=16) across selected pads.
    pub(crate) fn pads_hit_channel(&self) -> MultiValue<u8> {
        self.fold_selected(&self.selected_pads(), |s, i| match s.pads[i].hit {
            settings::PadHitAction::Note { channel, .. } => Some(displayed_channel(channel)),
            settings::PadHitAction::Off => None,
        })
    }

    /// The shared pad Press "type" across the selection: `Same(true)`=all Poly,
    /// `Same(false)`=all Disabled, `Differ`=mixed.
    pub(crate) fn pads_press_type(&self) -> MultiValue<bool> {
        self.fold_selected(&self.selected_pads(), |s, i| {
            Some(matches!(
                s.pads[i].pressure,
                settings::PadPressureAction::Poly { .. }
            ))
        })
    }

    /// Shared Press note (displayed; `None` → 0) across selected pads.
    pub(crate) fn pads_press_note(&self) -> MultiValue<u8> {
        self.fold_selected(&self.selected_pads(), |s, i| match s.pads[i].pressure {
            settings::PadPressureAction::Poly { note, .. } => Some(note.unwrap_or(0)),
            settings::PadPressureAction::Disabled => None,
        })
    }

    /// Shared Press channel (displayed 1..=16) across selected pads.
    pub(crate) fn pads_press_channel(&self) -> MultiValue<u8> {
        self.fold_selected(&self.selected_pads(), |s, i| match s.pads[i].pressure {
            settings::PadPressureAction::Poly { channel, .. } => Some(displayed_channel(channel)),
            settings::PadPressureAction::Disabled => None,
        })
    }

    pub(crate) fn encoder_cc(&self) -> Option<u8> {
        let s = self.settings.as_ref()?;
        match s.encoder.turn {
            settings::EncoderTurnAction::Cc { cc, .. } => Some(cc),
            settings::EncoderTurnAction::Off => None,
        }
    }

    pub(crate) fn encoder_mode(&self) -> Option<settings::CcValueMode> {
        let s = self.settings.as_ref()?;
        match &s.encoder.turn {
            settings::EncoderTurnAction::Cc { mode, .. } => Some(mode.clone()),
            settings::EncoderTurnAction::Off => None,
        }
    }

    /// Rebuild the encoder turn action from the snapshot, replacing `cc` and/or
    /// `mode` where given, and send it.
    pub(crate) fn apply_encoder(
        &mut self,
        cc: Option<u8>,
        mode: Option<settings::CcValueMode>,
        persist: bool,
    ) {
        let Some(s) = self.settings.as_ref() else {
            return;
        };
        let settings::EncoderTurnAction::Cc {
            channel,
            cc: cur_cc,
            mode: cur_mode,
        } = &s.encoder.turn
        else {
            return;
        };
        let turn = settings::EncoderTurnAction::Cc {
            channel: *channel,
            cc: cc.unwrap_or(*cur_cc),
            mode: mode.unwrap_or_else(|| cur_mode.clone()),
        };
        self.send_apply(encoder_delta(turn), persist);
    }

    pub(crate) fn set_encoder_channel(&mut self, ch: Option<u8>, persist: bool) {
        let Some(s) = self.settings.as_ref() else {
            return;
        };
        let settings::EncoderTurnAction::Cc { cc, mode, .. } = &s.encoder.turn else {
            return;
        };
        let channel = ch.and_then(|v| settings::MidiChannel::try_from(v).ok());
        let turn = settings::EncoderTurnAction::Cc {
            channel,
            cc: *cc,
            mode: mode.clone(),
        };
        self.send_apply(encoder_delta(turn), persist);
    }

    pub(crate) fn slider_position(&self) -> Option<(u8, Option<u8>)> {
        let s = self.settings.as_ref()?;
        match s.slider.position {
            settings::SliderPositionAction::Cc { channel, cc } => {
                Some((cc, channel.map(|c| c.as_u8())))
            }
            settings::SliderPositionAction::Off => None,
        }
    }

    pub(crate) fn slider_touch(&self) -> Option<settings::SliderTouchAction> {
        Some(self.settings.as_ref()?.slider.touch.clone())
    }

    pub(crate) fn apply_slider_touch(
        &mut self,
        action: settings::SliderTouchAction,
        persist: bool,
    ) {
        self.send_apply(slider_delta(None, Some(action), None), persist);
    }

    pub(crate) fn apply_slider_led(
        &mut self,
        edit: settings::partial::PartialSliderLedSettings,
        persist: bool,
    ) {
        self.send_apply(slider_delta(None, None, Some(edit)), persist);
    }

    pub(crate) fn set_encoder_button_type(&mut self, btn: u8, t: CcType) {
        let press = match t {
            CcType::ControlChange => default_button_press(btn),
            CcType::Off => settings::ButtonPressAction::Off,
        };
        self.send_apply(button_delta(&[btn], press), true);
    }

    pub(crate) fn button_cc_at(&self, btn: u8) -> Option<u8> {
        match self.settings.as_ref()?.buttons.0[btn as usize].press {
            settings::ButtonPressAction::Cc { cc, .. } => Some(cc),
            settings::ButtonPressAction::Off => None,
        }
    }

    pub(crate) fn button_channel_at(&self, btn: u8) -> Option<u8> {
        match self.settings.as_ref()?.buttons.0[btn as usize].press {
            settings::ButtonPressAction::Cc { channel, .. } => channel.map(|c| c.as_u8()),
            settings::ButtonPressAction::Off => None,
        }
    }

    pub(crate) fn apply_encoder_button(
        &mut self,
        btn: u8,
        cc: Option<u8>,
        channel: Option<Option<u8>>,
        persist: bool,
    ) {
        let cur_cc = self.button_cc_at(btn).unwrap_or(0);
        let cur_ch = self.button_channel_at(btn);
        let cc = cc.unwrap_or(cur_cc);
        let ch = channel.unwrap_or(cur_ch);
        let channel = ch.and_then(settings::MidiChannel::try_from_opt);
        self.send_apply(
            button_delta(&[btn], settings::ButtonPressAction::Cc { channel, cc }),
            persist,
        );
    }

    /// Per-pad hit delta changing only `channel` and/or `note`, preserving each
    /// selected pad's other field.
    pub(crate) fn pad_hit_delta(
        &self,
        set_channel: Option<Option<settings::MidiChannel>>,
        set_note: Option<u8>,
    ) -> Option<PartialSettings> {
        let s = self.settings.as_ref()?;
        Some(pads_map(&self.selected_pads(), |i| {
            match &s.pads[i as usize].hit {
                settings::PadHitAction::Note { channel, note } => {
                    Some(settings::partial::PartialPadConfig {
                        hit: Some(settings::PadHitAction::Note {
                            channel: set_channel.unwrap_or(*channel),
                            note: set_note.unwrap_or(*note),
                        }),
                        pressure: None,
                        led: None,
                    })
                }
                settings::PadHitAction::Off => None,
            }
        }))
    }

    /// Per-pad pressure delta changing only `channel` and/or `note`, preserving
    /// each pad's other field.
    pub(crate) fn pad_pressure_delta(
        &self,
        set_channel: Option<Option<settings::MidiChannel>>,
        set_note: Option<Option<u8>>,
    ) -> Option<PartialSettings> {
        let s = self.settings.as_ref()?;
        Some(pads_map(&self.selected_pads(), |i| {
            match &s.pads[i as usize].pressure {
                settings::PadPressureAction::Poly { channel, note } => {
                    Some(settings::partial::PartialPadConfig {
                        hit: None,
                        pressure: Some(settings::PadPressureAction::Poly {
                            channel: set_channel.unwrap_or(*channel),
                            note: set_note.unwrap_or(*note),
                        }),
                        led: None,
                    })
                }
                settings::PadPressureAction::Disabled => None,
            }
        }))
    }

    /// Per-button press delta changing only `channel` and/or `cc`, preserving
    /// each button's other field.
    pub(crate) fn button_press_delta(
        &self,
        set_channel: Option<Option<settings::MidiChannel>>,
        set_cc: Option<u8>,
    ) -> Option<PartialSettings> {
        let s = self.settings.as_ref()?;
        Some(buttons_map(&self.selected_buttons(), |i| {
            match &s.buttons[i as usize].press {
                settings::ButtonPressAction::Cc { channel, cc } => {
                    Some(settings::partial::PartialButtonConfig {
                        press: Some(settings::ButtonPressAction::Cc {
                            channel: set_channel.unwrap_or(*channel),
                            cc: set_cc.unwrap_or(*cc),
                        }),
                    })
                }
                settings::ButtonPressAction::Off => None,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::Settings;

    #[test]
    fn pads_map_preserves_each_pads_own_field() {
        use settings::{MidiChannel, PadHitAction};
        let notes = [60u8, 62, 64];
        let delta = pads_map(&[0, 1, 2], |i| {
            Some(PartialPadConfig {
                hit: Some(PadHitAction::Note {
                    channel: MidiChannel::try_from(3).ok(),
                    note: notes[i as usize],
                }),
                pressure: None,
                led: None,
            })
        });
        let merged = Settings::default().merge_overrides(delta);
        for (i, expected) in notes.iter().enumerate() {
            match merged.pads[i].hit {
                PadHitAction::Note { channel, note } => {
                    assert_eq!(note, *expected, "pad {i} keeps its own note");
                    assert_eq!(channel, MidiChannel::try_from(3).ok());
                }
                PadHitAction::Off => panic!("pad {i} should be a note"),
            }
        }
        assert_eq!(merged.pads[3], Settings::default().pads[3]);
    }

    #[test]
    fn buttons_map_skips_unlisted_buttons() {
        use settings::ButtonPressAction;
        let delta = buttons_map(&[5], |_| {
            Some(PartialButtonConfig {
                press: Some(ButtonPressAction::Cc {
                    channel: None,
                    cc: 77,
                }),
            })
        });
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(
            merged.buttons[5].press,
            ButtonPressAction::Cc {
                channel: None,
                cc: 77
            }
        );
        assert_eq!(merged.buttons[0], Settings::default().buttons[0]);
    }

    #[test]
    fn pad_delta_sets_only_listed_pads() {
        use settings::{PadHitAction, Settings};
        let delta = pad_delta(
            &[2, 5],
            Some(PadHitAction::Note {
                channel: None,
                note: 60,
            }),
            None,
        );
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(
            merged.pads[2].hit,
            PadHitAction::Note {
                channel: None,
                note: 60
            }
        );
        assert_eq!(
            merged.pads[5].hit,
            PadHitAction::Note {
                channel: None,
                note: 60
            }
        );
        assert_eq!(merged.pads[0], Settings::default().pads[0]);
    }

    #[test]
    fn button_delta_sets_only_listed_buttons() {
        use settings::{ButtonPressAction, Settings};
        let delta = button_delta(
            &[3],
            ButtonPressAction::Cc {
                channel: None,
                cc: 99,
            },
        );
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(
            merged.buttons[3].press,
            ButtonPressAction::Cc {
                channel: None,
                cc: 99
            }
        );
        assert_eq!(merged.buttons[0], Settings::default().buttons[0]);
    }

    #[test]
    fn encoder_delta_sets_turn() {
        use settings::{CcValueMode, EncoderTurnAction, Settings};
        let delta = encoder_delta(EncoderTurnAction::Cc {
            channel: None,
            cc: 22,
            mode: CcValueMode::Relative { step: 1 },
        });
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(
            merged.encoder.turn,
            EncoderTurnAction::Cc {
                channel: None,
                cc: 22,
                mode: CcValueMode::Relative { step: 1 }
            }
        );
    }

    #[test]
    fn slider_delta_sets_only_present_leaves() {
        use settings::partial::PartialSliderLedSettings;
        use settings::{Settings, SliderLedMode, SliderPositionAction};
        let delta = slider_delta(
            Some(SliderPositionAction::Cc {
                channel: None,
                cc: 7,
            }),
            None,
            Some(PartialSliderLedSettings {
                mode: Some(SliderLedMode::Dot),
                ..Default::default()
            }),
        );
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(
            merged.slider.position,
            SliderPositionAction::Cc {
                channel: None,
                cc: 7
            }
        );
        assert_eq!(merged.slider.led.mode, SliderLedMode::Dot);
        assert_eq!(merged.slider.touch, Settings::default().slider.touch);
    }
}
