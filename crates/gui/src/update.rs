//! Top-level update handler, extracted from `app.rs`.

use iced::Task;
use maschine_library::controls::Buttons;
use std::sync::Arc;

use crate::app::State;
use crate::message::Message;

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    use crate::device::view::control_index_valid;
    use protocol::{DriverToGui, GuiToDriver};

    match message {
        Message::Ready(sender) => {
            let _ = sender.send(GuiToDriver::GetSettings);
            let _ = sender.send(GuiToDriver::SubscribeEvents);
            state.sender = Some(sender);
            state.status = "connected".to_string();
            // A reconnect resyncs via GetSettings; let that snapshot fully adopt
            // even if edits were in flight when the previous link dropped.
            state.last_acked_seq = state.seq;
            state.resync_pending = false;
        }
        Message::Frame(DriverToGui::Settings(snapshot)) => {
            let snapshot = Arc::from(*snapshot);
            state.authoritative = Some(snapshot.clone());
            // Adopt as the live view when no newer local edit is in flight (so a
            // snapshot for an older apply can't clobber a newer optimistic edit),
            // OR when a rejected apply requested a resync — that snapshot reflects
            // the driver's post-rejection state and must replace the stale
            // optimistic value even though `seq` has advanced past the rejection.
            if state.resync_pending || state.last_acked_seq >= state.seq {
                state.settings = Some(snapshot);
                state.resync_pending = false;
            }
        }
        Message::Frame(DriverToGui::Ack { seq, result }) => {
            state.last_acked_seq = state.last_acked_seq.max(seq);
            if let Err(message) = result {
                state.status = format!("apply rejected: {message}");
                if seq == state.seq && state.authoritative.is_some() {
                    // Latest apply rejected, nothing newer in flight: revert now.
                    state.settings = state.authoritative.clone();
                } else {
                    // An older apply was rejected while a newer edit is in flight:
                    // reverting to `authoritative` would lose the newer edit, so
                    // adopt the driver's fresh snapshot when the resync returns.
                    state.resync_pending = true;
                }
                if let Some(sender) = &state.sender {
                    let _ = sender.send(GuiToDriver::GetSettings);
                }
            }
        }
        Message::Frame(DriverToGui::ControlActuated { control }) => {
            // Guard against an out-of-range index from the driver: the
            // inspector indexes fixed-size arrays with it, so a stray value
            // would panic the GUI.
            if state.touch_select && control_index_valid(control) {
                state.reset_assign_edit();
                state.selection = vec![select_target(state, control)];
            }
        }
        Message::Frame(DriverToGui::MidiActivity { dir }) => match dir {
            protocol::MidiDir::In => state.last_in = Some(std::time::Instant::now()),
            protocol::MidiDir::Out => state.last_out = Some(std::time::Instant::now()),
        },
        Message::Tick => {}
        Message::Frame(DriverToGui::DeviceConnected(connected)) => {
            state.device_connected = connected;
        }
        Message::Disconnected => {
            state.sender = None;
            state.device_connected = false;
            state.status = "disconnected".to_string();
        }
        Message::Error(err) => {
            state.sender = None;
            state.device_connected = false;
            state.status = format!("error: {err}");
        }
        Message::SelectControl(control) => {
            if control_index_valid(control) {
                state.reset_assign_edit();
                state.selection = vec![select_target(state, control)];
            }
        }
        Message::SelectControls(controls) => {
            let filtered: Vec<_> = controls
                .into_iter()
                .filter(|c| control_index_valid(*c))
                .collect();
            // An empty drag (covered nothing) leaves the selection untouched.
            if !filtered.is_empty() {
                state.selection = filtered;
                state.reset_assign_edit();
            }
        }
        Message::ToggleControl(control) => {
            if control_index_valid(control) {
                if state.selection.is_empty() {
                    state.selection = vec![control];
                    state.reset_assign_edit();
                } else if same_control_kind(&state.selection[0], &control) {
                    if let Some(pos) = state.selection.iter().position(|c| *c == control) {
                        state.selection.remove(pos);
                    } else {
                        state.selection.push(control);
                    }
                    state.reset_assign_edit();
                }
                // Different kind than the current selection: ignored.
            }
        }
        Message::ToggleTouchSelect(on) => {
            state.touch_select = on;
            state.save_gui_prefs();
        }
        Message::PreviewPadSensitivity(v) => state.send_apply(
            crate::prefs::overrides::hardware_delta(|h| h.pad_sensitivity = Some(v)),
            false,
        ),
        Message::PreviewDisplayContrast(v) => state.send_apply(
            crate::prefs::overrides::hardware_delta(|h| h.display_contrast = Some(v)),
            false,
        ),
        Message::SetPadSensitivity => {
            if let Some(s) = &state.settings {
                let v = s.hardware.pad_sensitivity;
                state.send_apply(
                    crate::prefs::overrides::hardware_delta(|h| h.pad_sensitivity = Some(v)),
                    true,
                );
            }
        }
        Message::SetDisplayContrast => {
            if let Some(s) = &state.settings {
                let v = s.hardware.display_contrast;
                state.send_apply(
                    crate::prefs::overrides::hardware_delta(|h| h.display_contrast = Some(v)),
                    true,
                );
            }
        }
        Message::SetVelocityCurve(c) => state.send_apply(
            crate::prefs::overrides::hardware_delta(|h| h.pad_velocity_curve = Some(c)),
            true,
        ),
        Message::PreviewLedBrightness(v) => state.send_apply(
            crate::prefs::overrides::hardware_delta(|h| h.led_brightness = Some(v)),
            false,
        ),
        Message::SetLedBrightness => {
            if let Some(s) = &state.settings {
                let v = s.hardware.led_brightness;
                state.send_apply(
                    crate::prefs::overrides::hardware_delta(|h| h.led_brightness = Some(v)),
                    true,
                );
            }
        }
        Message::TogglePrefs => state.show_prefs = !state.show_prefs,
        Message::Ignore => {}
        Message::SetEncoderModeKind(kind) => {
            if let Some(prev) = state.encoder_mode() {
                state.apply_encoder(None, Some(kind.to_mode(&prev)), true);
            }
        }
        Message::SetEncoderWrap(wrap) => {
            if let Some(mode) = state.encoder_mode() {
                state.apply_encoder(
                    None,
                    Some(crate::inspector::assign::mapping::with_wrap(&mode, wrap)),
                    true,
                );
            }
        }
        Message::SetSliderTouchKind(kind) => {
            if let Some(prev) = state.slider_touch() {
                state.apply_slider_touch(kind.to_action(&prev), true);
            }
        }
        Message::SetSliderLedMode(mode) => state.apply_slider_led(
            settings::partial::PartialSliderLedSettings {
                mode: Some(mode),
                ..Default::default()
            },
            true,
        ),
        Message::SetSliderLedColor(color) => state.apply_slider_led(
            settings::partial::PartialSliderLedSettings {
                color: Some(color),
                ..Default::default()
            },
            true,
        ),
        Message::SetSliderLedStylized(on) => state.apply_slider_led(
            settings::partial::PartialSliderLedSettings {
                stylized: Some(on),
                ..Default::default()
            },
            true,
        ),
        Message::ToggleSoftOff(on) => state.send_apply(
            crate::prefs::overrides::driver_delta(|d| d.soft_off_enabled = Some(on)),
            true,
        ),
        Message::ToggleSelfTestAtLaunch(on) => state.send_apply(
            crate::prefs::overrides::driver_delta(|d| d.self_test_on_launch = Some(on)),
            true,
        ),
        Message::NumericInput(field, s) => {
            state.edit_field = Some(field);
            state.edit_text = s.clone();
            let applied = if field == crate::inspector::assign::numeric::EditField::SliderAutoOff {
                if let Ok(ms) = s.trim().parse::<u64>() {
                    state.apply_slider_led(
                        settings::partial::PartialSliderLedSettings {
                            auto_off_ms: Some(
                                ms.min(crate::inspector::assign::numeric::MAX_AUTO_OFF_MS),
                            ),
                            ..Default::default()
                        },
                        false,
                    );
                    true
                } else {
                    false
                }
            } else if let Some(range) = field.range()
                && let Some(v) = crate::widget::numeric_field::parse_clamped(&s, range)
            {
                state.apply_numeric(field, v, false);
                true
            } else {
                false
            };
            // Re-arm the debounce so the live edit persists after typing stops,
            // since iced gives no focus-lost callback to commit on.
            if applied {
                state.persist_debounce = crate::app::PERSIST_DEBOUNCE_TICKS;
            }
        }
        Message::PersistDebounce => {
            if state.persist_debounce > 0 {
                state.persist_debounce -= 1;
                if state.persist_debounce == 0 {
                    state.persist_current();
                }
            }
        }
        Message::NumericCommit(field) => {
            let s = std::mem::take(&mut state.edit_text);
            state.edit_field = None;
            // Enter persists immediately below; drop any pending debounced flush.
            state.persist_debounce = 0;
            if field == crate::inspector::assign::numeric::EditField::SliderAutoOff {
                if let Ok(ms) = s.trim().parse::<u64>() {
                    state.apply_slider_led(
                        settings::partial::PartialSliderLedSettings {
                            auto_off_ms: Some(
                                ms.min(crate::inspector::assign::numeric::MAX_AUTO_OFF_MS),
                            ),
                            ..Default::default()
                        },
                        true,
                    );
                }
            } else if let Some(range) = field.range()
                && let Some(v) = crate::widget::numeric_field::parse_clamped(&s, range)
            {
                state.apply_numeric(field, v, true);
            }
        }
        Message::NumericStep(field, dir) => {
            state.edit_field = None;
            state.edit_text.clear();
            // Scroll steps persist immediately below; drop any pending debounce.
            state.persist_debounce = 0;
            if let Some(range) = field.range()
                && let Some(cur) = state.current_numeric(field)
            {
                use crate::widget::numeric_field::step_value;
                let mut v = step_value(cur, dir, range.clone());
                // The encoder step has no valid 0 (it would freeze the encoder),
                // so a step that lands on 0 must continue past it — otherwise the
                // scroll wheel can never cross from +1 to -1 to reverse direction.
                if field == crate::inspector::assign::numeric::EditField::EncoderStep && v == 0 {
                    v = step_value(v, dir, range);
                }
                state.apply_numeric(field, v, true);
            }
        }
        Message::SetPadPressureKind(on) => {
            let pads = state.selected_pads();
            if !pads.is_empty() {
                let pressure = if on {
                    settings::PadPressureAction::Poly {
                        channel: None,
                        note: None,
                    }
                } else {
                    settings::PadPressureAction::Disabled
                };
                state.send_apply(
                    crate::inspector::assign::mapping::pad_delta(&pads, None, Some(pressure)),
                    true,
                );
            }
        }
        Message::SetAssignTab(tab) => state.assign_tab = tab,
        Message::ToggleShowAllLabels(on) => {
            state.show_all_labels = on;
            state.save_gui_prefs();
        }
        Message::SetPadHitType(t) => {
            use crate::inspector::assign::forms::PadHitType;
            let pads = state.selected_pads();
            if !pads.is_empty() {
                let delta = match t {
                    PadHitType::Note => {
                        crate::inspector::assign::mapping::default_pad_hit_delta(&pads)
                    }
                    PadHitType::Off => crate::inspector::assign::mapping::pad_delta(
                        &pads,
                        Some(settings::PadHitAction::Off),
                        None,
                    ),
                };
                state.send_apply(delta, true);
            }
        }
        Message::SetPadPressType(t) => {
            use crate::inspector::assign::forms::PadPressType;
            let pads = state.selected_pads();
            if !pads.is_empty() {
                let pressure = match t {
                    PadPressType::PolyPressure => settings::PadPressureAction::Poly {
                        channel: None,
                        note: None,
                    },
                    PadPressType::Off => settings::PadPressureAction::Disabled,
                };
                state.send_apply(
                    crate::inspector::assign::mapping::pad_delta(&pads, None, Some(pressure)),
                    true,
                );
            }
        }
        Message::SetButtonType(t) => {
            use crate::inspector::assign::forms::CcType;
            let buttons = state.selected_buttons();
            if !buttons.is_empty() {
                let delta = match t {
                    CcType::ControlChange => {
                        crate::inspector::assign::mapping::default_button_press_delta(&buttons)
                    }
                    CcType::Off => crate::inspector::assign::mapping::button_delta(
                        &buttons,
                        settings::ButtonPressAction::Off,
                    ),
                };
                state.send_apply(delta, true);
            }
        }
        Message::SetEncoderTurnType(t) => {
            use crate::inspector::assign::forms::CcType;
            let turn = match t {
                CcType::ControlChange => crate::inspector::assign::mapping::default_encoder_turn(),
                CcType::Off => settings::EncoderTurnAction::Off,
            };
            state.send_apply(crate::inspector::assign::mapping::encoder_delta(turn), true);
        }
        Message::SetEncoderPushType(t) => {
            state.set_encoder_button_type(Buttons::EncoderPress as u8, t)
        }
        Message::SetEncoderTouchType(t) => {
            state.set_encoder_button_type(Buttons::EncoderTouch as u8, t)
        }
        Message::SetSliderPositionType(t) => {
            use crate::inspector::assign::forms::CcType;
            let position = match t {
                CcType::ControlChange => {
                    crate::inspector::assign::mapping::default_slider_position()
                }
                CcType::Off => settings::SliderPositionAction::Off,
            };
            state.send_apply(
                crate::inspector::assign::mapping::slider_delta(Some(position), None, None),
                true,
            );
        }
        Message::SetPadLedSource(source) => state.apply_pad_led_source(source),
        Message::SetPadLedMode(tab, mode) => state.apply_pad_led_mode(tab, mode),
        Message::SetPadLedColor(tab, slot, color) => state.apply_pad_led_color(tab, slot, color),
        Message::SetInspectorTab(tab) => state.inspector_tab = tab,
        Message::SetPagingEnabled(enabled) => state.apply_pad_paging(true, |p| p.enabled = enabled),
        Message::SelectPage(index) => state.apply_pad_paging(true, |p| p.active = index),
        Message::SetDefaultPageColor(color) => {
            state.apply_pad_paging(true, |p| p.default_page_color = color)
        }
    }
    Task::none()
}
/// Resolve a freshly-selected control to its inspector target. Encoder push/touch
/// arrive as button slots 39/40 on the wire but are sub-actions of the Encoder;
/// map them to the Encoder control with the matching tab so they open the encoder
/// form's Push/Touch tab instead of a generic, unlabeled "Button" inspector.
fn select_target(state: &mut State, control: protocol::ControlRef) -> protocol::ControlRef {
    use crate::inspector::assign::forms::AssignTab;
    match control {
        protocol::ControlRef::Button(b) if b == Buttons::EncoderPress as u8 => {
            state.assign_tab = AssignTab::B;
            protocol::ControlRef::Encoder
        }
        protocol::ControlRef::Button(b) if b == Buttons::EncoderTouch as u8 => {
            state.assign_tab = AssignTab::C;
            protocol::ControlRef::Encoder
        }
        other => other,
    }
}

/// Whether two control refs are the same kind (Pad/Button/Encoder/Slider).
/// Selection is mutually exclusive across kinds, so Ctrl+click only toggles
/// within the kind already selected.
fn same_control_kind(a: &protocol::ControlRef, b: &protocol::ControlRef) -> bool {
    use protocol::ControlRef::*;
    matches!(
        (a, b),
        (Pad(_), Pad(_)) | (Button(_), Button(_)) | (Encoder, Encoder) | (Slider, Slider)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::State;
    use crate::prefs::overrides::hardware_delta;
    use protocol::{DriverToGui, GuiToDriver};
    use settings::Settings;

    /// A connected `State` with a snapshot already adopted, plus the channel the
    /// driver would read outgoing frames from.
    fn seeded() -> (State, std::sync::mpsc::Receiver<GuiToDriver>) {
        let mut state = State::default();
        let (tx, rx) = std::sync::mpsc::channel();
        state.sender = Some(tx);
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::default())),
        );
        (state, rx)
    }

    /// Drain every frame the GUI sent to the driver.
    fn drained_frames(rx: &std::sync::mpsc::Receiver<GuiToDriver>) -> Vec<GuiToDriver> {
        let mut out = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            out.push(frame);
        }
        out
    }

    fn is_live_apply(frame: &GuiToDriver) -> bool {
        matches!(frame, GuiToDriver::Apply { persist: false, .. })
    }

    fn is_persist(frame: &GuiToDriver) -> bool {
        matches!(frame, GuiToDriver::Persist { .. })
    }

    fn pad_note(state: &State, internal: usize) -> Option<u8> {
        match state.settings.as_ref()?.active_pads()[internal].hit {
            settings::PadHitAction::Note { note, .. } => Some(note),
            settings::PadHitAction::Off => None,
        }
    }

    #[test]
    fn typed_pad_note_persists_after_debounce_without_enter() {
        use crate::inspector::assign::numeric::EditField;
        use protocol::ControlRef;
        let (mut state, rx) = seeded();
        state.selection = vec![ControlRef::Pad(0)];

        // Type a new note but never press Enter.
        let _ = update(
            &mut state,
            Message::NumericInput(EditField::PadHitNote, "60".into()),
        );

        // The edit applies live (preview) and arms the debounce; nothing persisted yet.
        let frames = drained_frames(&rx);
        assert_eq!(frames.len(), 1, "one live apply for the keystroke");
        assert!(
            is_live_apply(&frames[0]),
            "keystroke applies live, not persisted"
        );
        assert_eq!(
            pad_note(&state, 0),
            Some(60),
            "GUI shows the typed value live"
        );
        assert_eq!(state.persist_debounce, crate::app::PERSIST_DEBOUNCE_TICKS);

        // The quiet-window ticks elapse with no further typing.
        for _ in 0..crate::app::PERSIST_DEBOUNCE_TICKS {
            let _ = update(&mut state, Message::PersistDebounce);
        }

        // A Persist request was flushed, so the value survives a restart.
        let flush = drained_frames(&rx);
        assert!(
            flush.iter().any(is_persist),
            "debounce must persist the typed edit even without Enter"
        );
        assert_eq!(state.persist_debounce, 0, "debounce cleared after flushing");
    }

    #[test]
    fn further_typing_resets_the_persist_debounce() {
        use crate::inspector::assign::numeric::EditField;
        use protocol::ControlRef;
        let (mut state, rx) = seeded();
        state.selection = vec![ControlRef::Pad(0)];

        let _ = update(
            &mut state,
            Message::NumericInput(EditField::PadHitNote, "6".into()),
        );
        // One quiet tick passes...
        let _ = update(&mut state, Message::PersistDebounce);
        assert_eq!(
            state.persist_debounce,
            crate::app::PERSIST_DEBOUNCE_TICKS - 1
        );
        // ...then the user types again, which must re-arm the full window.
        let _ = update(
            &mut state,
            Message::NumericInput(EditField::PadHitNote, "60".into()),
        );
        assert_eq!(state.persist_debounce, crate::app::PERSIST_DEBOUNCE_TICKS);

        let _ = drained_frames(&rx); // clear live previews
        // Only one tick has effectively passed since the last keystroke: no flush yet.
        let _ = update(&mut state, Message::PersistDebounce);
        assert!(
            !drained_frames(&rx).iter().any(is_persist),
            "must not persist until the window is quiet"
        );
    }

    #[test]
    fn enter_cancels_a_pending_persist_debounce() {
        use crate::inspector::assign::numeric::EditField;
        use protocol::ControlRef;
        let (mut state, rx) = seeded();
        state.selection = vec![ControlRef::Pad(0)];

        let _ = update(
            &mut state,
            Message::NumericInput(EditField::PadHitNote, "60".into()),
        );
        let _ = update(&mut state, Message::NumericCommit(EditField::PadHitNote));
        assert_eq!(
            state.persist_debounce, 0,
            "Enter persists and disarms the debounce"
        );

        let _ = drained_frames(&rx);
        // A late debounce tick must not fire a second, redundant persist.
        let _ = update(&mut state, Message::PersistDebounce);
        assert!(
            drained_frames(&rx).is_empty(),
            "no extra persist after Enter already committed"
        );
    }

    #[test]
    fn ctrl_click_toggles_same_kind_membership() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Button(1)];
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Button(2)));
        assert_eq!(
            state.selection,
            vec![ControlRef::Button(1), ControlRef::Button(2)]
        );
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Button(1)));
        assert_eq!(state.selection, vec![ControlRef::Button(2)]);
    }

    #[test]
    fn ctrl_click_different_kind_is_ignored() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Button(1)];
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Pad(12)));
        assert_eq!(state.selection, vec![ControlRef::Button(1)]);
    }

    #[test]
    fn ctrl_click_into_empty_selects_the_control() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        assert!(state.selection.is_empty());
        let _ = update(&mut state, Message::ToggleControl(ControlRef::Pad(12)));
        assert_eq!(state.selection, vec![ControlRef::Pad(12)]);
    }

    #[test]
    fn empty_drag_keeps_current_selection() {
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Pad(12)];
        let _ = update(&mut state, Message::SelectControls(vec![]));
        assert_eq!(state.selection, vec![ControlRef::Pad(12)]);
    }

    fn snapshot_with_sensitivity(v: u8) -> DriverToGui {
        DriverToGui::Settings(Box::new(
            Settings::default().merge_overrides(hardware_delta(|h| h.pad_sensitivity = Some(v))),
        ))
    }

    #[test]
    fn snapshot_for_older_apply_does_not_clobber_newer_edit() {
        let (mut state, _rx) = seeded();
        // Commit A (persist), then a newer preview B before A's snapshot lands.
        state.send_apply(hardware_delta(|h| h.pad_sensitivity = Some(10)), true);
        state.send_apply(hardware_delta(|h| h.pad_sensitivity = Some(20)), false);

        // A's Ack and its snapshot arrive while B (seq 2) is still un-acked.
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq: 1,
                result: Ok(()),
            }),
        );
        let _ = update(&mut state, Message::Frame(snapshot_with_sensitivity(10)));

        // The optimistic preview B must survive; only `authoritative` tracks A.
        assert_eq!(
            state.settings.as_ref().unwrap().hardware.pad_sensitivity,
            20
        );
        assert_eq!(
            state
                .authoritative
                .as_ref()
                .unwrap()
                .hardware
                .pad_sensitivity,
            10
        );
    }

    #[test]
    fn snapshot_is_adopted_once_acks_catch_up() {
        let (mut state, _rx) = seeded();
        state.send_apply(hardware_delta(|h| h.pad_sensitivity = Some(30)), true);
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq: 1,
                result: Ok(()),
            }),
        );
        let _ = update(&mut state, Message::Frame(snapshot_with_sensitivity(30)));
        assert_eq!(
            state.settings.as_ref().unwrap().hardware.pad_sensitivity,
            30
        );
    }

    #[test]
    fn button_off_then_on_resets_to_schema_default() {
        use crate::inspector::assign::forms::CcType;
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        // Give button 3 a non-default CC + channel.
        state.selection = vec![ControlRef::Button(3)];
        state.send_apply(
            crate::inspector::assign::mapping::button_delta(
                &[3],
                settings::ButtonPressAction::Cc {
                    channel: settings::MidiChannel::try_from(5).ok(),
                    cc: 77,
                },
            ),
            true,
        );
        // Switch Off, then back to CC: re-enabling resets to the button's schema
        // default (not the previous CC/channel) — there is no remembering.
        let _ = update(&mut state, Message::SetButtonType(CcType::Off));
        let _ = update(&mut state, Message::SetButtonType(CcType::ControlChange));
        assert_eq!(
            state.settings.as_ref().unwrap().buttons[3].press,
            Settings::default().buttons[3].press,
        );
    }

    #[test]
    fn scroll_step_crosses_zero_to_reverse_encoder_direction() {
        use crate::inspector::assign::numeric::EditField;
        let (mut state, _rx) = seeded();
        // Encoder in Relative mode, step +1.
        state.send_apply(
            crate::inspector::assign::mapping::encoder_delta(settings::EncoderTurnAction::Cc {
                channel: None,
                cc: 1,
                mode: settings::CcValueMode::Relative { step: 1 },
            }),
            true,
        );
        // Scrolling down from +1 must reach -1 (reverse), not stick at +1 because
        // the invalid 0 is coerced back to 1.
        let _ = update(&mut state, Message::NumericStep(EditField::EncoderStep, -1));
        assert_eq!(state.current_numeric(EditField::EncoderStep), Some(-1));
        // And scrolling back up crosses zero the other way.
        let _ = update(&mut state, Message::NumericStep(EditField::EncoderStep, 1));
        assert_eq!(state.current_numeric(EditField::EncoderStep), Some(1));
    }

    #[test]
    fn rejected_apply_resyncs_from_driver() {
        let (mut state, rx) = seeded();
        // Drain the GetSettings/SubscribeEvents from the initial seed if any.
        while rx.try_recv().is_ok() {}
        state.send_apply(hardware_delta(|h| h.pad_sensitivity = Some(99)), true);
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq: 1,
                result: Err("bad".to_string()),
            }),
        );
        // The latest edit was rejected → revert to authoritative + resync.
        assert_eq!(
            state.settings.as_ref().unwrap().hardware.pad_sensitivity,
            Settings::default().hardware.pad_sensitivity
        );
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok()).any(|m| m == GuiToDriver::GetSettings),
            "a resync GetSettings must be sent after a rejected apply"
        );
    }

    #[test]
    fn rejected_older_apply_adopts_resync_snapshot_despite_newer_in_flight() {
        let (mut state, _rx) = seeded();
        // Commit A (seq 1) then preview B (seq 2), both optimistically merged.
        state.send_apply(hardware_delta(|h| h.pad_sensitivity = Some(10)), true);
        state.send_apply(hardware_delta(|h| h.pad_sensitivity = Some(20)), false);
        assert_eq!(
            state.settings.as_ref().unwrap().hardware.pad_sensitivity,
            20
        );

        // A is rejected while B (seq 2) is still in flight: reverting to
        // `authoritative` would drop B, so a resync is requested instead.
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq: 1,
                result: Err("bad".to_string()),
            }),
        );
        assert!(state.resync_pending, "older-apply rejection marks a resync");

        // The resync snapshot reflects the driver's post-rejection state and must
        // be adopted even though last_acked_seq (1) < seq (2), clearing the stale
        // optimistic value.
        let _ = update(&mut state, Message::Frame(snapshot_with_sensitivity(55)));
        assert_eq!(
            state.settings.as_ref().unwrap().hardware.pad_sensitivity,
            55
        );
        assert!(!state.resync_pending, "resync flag clears once adopted");
    }

    #[test]
    fn touch_selecting_encoder_push_or_touch_opens_the_encoder_form() {
        use crate::inspector::assign::forms::AssignTab;
        use protocol::ControlRef;
        let (mut state, _rx) = seeded();
        state.touch_select = true;

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::ControlActuated {
                control: ControlRef::Button(Buttons::EncoderPress as u8),
            }),
        );
        assert_eq!(state.selection, vec![ControlRef::Encoder]);
        assert_eq!(state.assign_tab, AssignTab::B);

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::ControlActuated {
                control: ControlRef::Button(Buttons::EncoderTouch as u8),
            }),
        );
        assert_eq!(state.selection, vec![ControlRef::Encoder]);
        assert_eq!(state.assign_tab, AssignTab::C);
    }

    #[test]
    fn set_pad_led_source_updates_selected_pads() {
        use protocol::ControlRef;
        use settings::PadLedSource;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Pad(0), ControlRef::Pad(3)];
        let _ = update(&mut state, Message::SetPadLedSource(PadLedSource::MidiIn));
        let s = state.settings.as_ref().unwrap();
        assert_eq!(s.active_pads()[0].led.source, PadLedSource::MidiIn);
        assert_eq!(s.active_pads()[3].led.source, PadLedSource::MidiIn);
    }

    #[test]
    fn set_pad_led_source_noop_when_already_current() {
        use protocol::ControlRef;
        use settings::PadLedSource;
        let (mut state, rx) = seeded();
        state.selection = vec![ControlRef::Pad(0)];
        // Default pad source is MidiOut; re-selecting it must not send an Apply.
        let _ = update(&mut state, Message::SetPadLedSource(PadLedSource::MidiOut));
        assert!(
            rx.try_recv().is_err(),
            "re-selecting the current source sends no Apply"
        );
    }

    #[test]
    fn set_pad_led_mode_preserves_stored_colors() {
        use crate::inspector::assign::forms::LedTab;
        use protocol::ControlRef;
        use settings::PadLedMode;
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Pad(0)];
        // Switching mode changes only `mode`; every mode's stored colors are kept.
        let before = state.settings.as_ref().unwrap().active_pads()[0]
            .led
            .midi_out;
        let _ = update(
            &mut state,
            Message::SetPadLedMode(LedTab::Out, PadLedMode::Single),
        );
        let after = state.settings.as_ref().unwrap().active_pads()[0]
            .led
            .midi_out;
        assert_eq!(after.mode, PadLedMode::Single);
        assert_eq!(after.single, before.single);
        assert_eq!(after.dual_on, before.dual_on);
        assert_eq!(after.dual_off, before.dual_off);
    }

    #[test]
    fn pad_led_color_survives_round_trip_through_velocity() {
        use crate::inspector::assign::forms::LedTab;
        use crate::inspector::assign::mapping::PadLedColorSlot;
        use protocol::ControlRef;
        use settings::{PadColors, PadLedMode};
        let (mut state, _rx) = seeded();
        state.selection = vec![ControlRef::Pad(0)];

        // Make Out a Single Red.
        let _ = update(
            &mut state,
            Message::SetPadLedMode(LedTab::Out, PadLedMode::Single),
        );
        let _ = update(
            &mut state,
            Message::SetPadLedColor(LedTab::Out, PadLedColorSlot::Single, PadColors::Red),
        );

        // Single -> Velocity -> Single keeps Red: every mode's colors persist in
        // the stored struct, so nothing is dropped on the round-trip.
        let _ = update(
            &mut state,
            Message::SetPadLedMode(LedTab::Out, PadLedMode::Velocity),
        );
        let _ = update(
            &mut state,
            Message::SetPadLedMode(LedTab::Out, PadLedMode::Single),
        );
        let out = state.settings.as_ref().unwrap().active_pads()[0]
            .led
            .midi_out;
        assert_eq!(out.mode, PadLedMode::Single);
        assert_eq!(out.single, PadColors::Red);
    }
}
