//! Top-level update handler, extracted from `app.rs`.

use iced::Task;
use maschine_library::controls::Buttons;
use std::sync::Arc;

use crate::app::State;
use crate::message::Message;

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    use crate::device::view::control_index_valid;
    use protocol::{DriverToGui, GuiToDriver};

    // Almost every arm below only mutates `state`; `BeginRenamePage` is the
    // sole exception that needs to return a real `Task` (focusing the
    // now-visible `text_input`), so it assigns into this instead of every
    // other arm having to end in an explicit `Task::none()`.
    let mut task = Task::none();

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
                // Every persisted apply is answered with an ack *and* a full
                // snapshot, so most adoptions are the driver echoing back the
                // rows the GUI already shows. Row gestures address rows by
                // index, so they only have to be dropped when the adopted row
                // list itself differs — a hardware page switch pushes a
                // snapshot per pad tap while Group is held, and that moves
                // `active` without invalidating a single index.
                let rows_changed = state.settings.as_ref().is_none_or(|current| {
                    current.pad_paging.enabled != snapshot.pad_paging.enabled
                        || current.pad_paging.pages != snapshot.pad_paging.pages
                });
                let first_snapshot = state.settings.is_none();
                state.settings = Some(snapshot);
                state.resync_pending = false;
                if rows_changed {
                    clear_page_gestures(state);
                }
                // Overrides persisted before pages carried names (or partials
                // that omitted them) load as unnamed pages. Names are assigned
                // at creation and never derived from position, so fill the gaps
                // with fresh default letter names and persist — but only from
                // the session's first snapshot, which is the only one that can
                // be a migration. Every later snapshot is the driver echoing
                // state the GUI already produced, where an unnamed page means
                // the user is mid-rename with the field cleared; renaming it
                // out from under them would fill the `text_input` they are
                // typing in. A rejected persist also resyncs the same unnamed
                // pages straight back, which would re-fire this apply forever.
                let has_unnamed = state
                    .settings
                    .as_ref()
                    .is_some_and(|s| s.pad_paging.pages.iter().any(|p| p.name.is_none()));
                if first_snapshot && has_unnamed {
                    state.apply_pad_paging(true, |pp| {
                        for i in 0..pp.pages.len() {
                            if pp.pages[i].name.is_none() {
                                let name = pp.next_page_name();
                                pp.pages[i].name = Some(name);
                            }
                        }
                    });
                }
            }
        }
        Message::Frame(DriverToGui::Ack { seq, result }) => {
            state.last_acked_seq = state.last_acked_seq.max(seq);
            if let Err(message) = result {
                state.status = format!("apply rejected: {message}");
                if seq == state.seq && state.authoritative.is_some() {
                    // Latest apply rejected, nothing newer in flight: revert now.
                    // The revert can shrink the row list (a rejected `AddPage`),
                    // so the index-addressed gestures go with it.
                    state.settings = state.authoritative.clone();
                    clear_page_gestures(state);
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
            if state.page_name_debounce > 0 {
                state.page_name_debounce -= 1;
                if state.page_name_debounce == 0 {
                    flush_page_name(state);
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
        Message::SetInspectorTab(tab) => {
            // Leaving the Pages tab destroys the rename `text_input` and every
            // row `mouse_area`, so none of them can ever report back: commit the
            // open rename now, and drop the gestures no row is left to end.
            commit_open_page_rename(state);
            clear_page_gestures(state);
            state.inspector_tab = tab;
        }
        Message::SetPagingEnabled(enabled) => {
            commit_open_page_rename(state);
            state.apply_pad_paging(true, |p| p.enabled = enabled);
            // Disabling paging removes the row list from the view entirely,
            // leaving every row gesture with no row able to end it. Clearing
            // unconditionally (not just on disable) is simplest and always
            // safe: a real gesture mid-toggle has lost its visual footing
            // either way.
            clear_page_gestures(state);
        }
        Message::SelectPage(index) => {
            // Switching the active page must not leave an in-progress Assign
            // edit (or a debounce armed to flush it) pointed at the page that
            // was active when the user started typing.
            commit_open_page_rename(state);
            state.reset_assign_edit();
            state.persist_debounce = 0;
            state.apply_pad_paging(true, |p| p.active = index);
        }
        Message::SetDefaultPageColor(color) => {
            state.apply_pad_paging(true, |p| p.default_page_color = color)
        }
        Message::AddPage => state.apply_pad_paging(true, crate::app::page_ops::add),
        Message::DuplicatePage(i) => {
            state.apply_pad_paging(true, move |pp| crate::app::page_ops::duplicate(pp, i))
        }
        Message::RequestDeletePage(i) => state.confirm_delete_page = Some(i),
        Message::ConfirmDeletePage => {
            if let Some(i) = state.confirm_delete_page.take() {
                state.apply_pad_paging(true, move |pp| crate::app::page_ops::delete(pp, i));
            }
        }
        Message::CancelDeletePage => state.confirm_delete_page = None,
        Message::SetPageName(i, name) => {
            // Keystrokes stay in the GUI: the field renders from `page_name_text`,
            // so the driver hears one applied name per quiet window instead of one
            // per character. Untrimmed here — the raw text is what the user is
            // typing into; only what gets stored is trimmed.
            if state.editing_page_name == Some(i) {
                state.page_name_text = name;
                state.page_name_debounce = crate::app::PERSIST_DEBOUNCE_TICKS;
            }
        }
        Message::CommitPageName(i) => commit_page_name(state, i),
        Message::BeginRenamePage(i) => {
            // Seed the field from the stored name; from here on it is the text
            // buffer, not `settings`, that the row renders.
            state.page_name_text = state
                .settings
                .as_ref()
                .and_then(|s| s.pad_paging.pages.get(i))
                .and_then(|page| page.name.clone())
                .unwrap_or_default();
            state.page_name_debounce = 0;
            state.editing_page_name = Some(i);
            task = iced::widget::operation::focus(
                crate::inspector::pages::view::page_name_input_id(i),
            );
        }
        Message::PageDragStart(i) => {
            state.page_drag = Some(crate::app::PageDrag {
                from: i,
                over: None,
            })
        }
        Message::PageRowEntered(j) => {
            state.hovered_page = Some(j);
            if let Some(drag) = state.page_drag.as_mut() {
                drag.over = Some(j);
            }
        }
        Message::PageRowExited(j) => {
            // Guarded so the exit of the row just left can't erase the enter
            // of the row just crossed into — event order between adjacent
            // mouse_areas isn't guaranteed.
            if state.hovered_page == Some(j) {
                state.hovered_page = None;
            }
        }
        Message::PageDragDrop => {
            if let Some(drag) = state.page_drag.take() {
                // Defense in depth: `page_drag` is cleared wherever it could
                // otherwise be orphaned (a settings resync, paging toggled
                // off), but a stale `from`/`over` must never be able to
                // mutate page order even if some future path misses that.
                let page_count = state
                    .settings
                    .as_ref()
                    .map_or(0, |s| s.pad_paging.pages.len());
                let in_range = drag.from < page_count && drag.over.is_none_or(|to| to < page_count);
                if in_range {
                    match drag.over {
                        Some(to) if to != drag.from => {
                            // `editing_page_name` is a raw slot index, which a
                            // reorder invalidates: the row now open for rename
                            // may hold a different page after the move. Commit
                            // and close it against the pre-reorder indices
                            // rather than reindex it.
                            commit_open_page_rename(state);
                            state.apply_pad_paging(true, move |pp| {
                                crate::app::page_ops::reorder(pp, drag.from, to)
                            });
                        }
                        // No effective move (plain click, or a drag that
                        // returned to its origin row): treat the release as a
                        // selection, since the row's own button no longer
                        // carries `on_press`.
                        _ => {
                            // Keep a rename the double-click ending in this
                            // very release just opened on the same row; a
                            // click landing on any other row commits it.
                            if state.editing_page_name != Some(drag.from) {
                                commit_open_page_rename(state);
                            }
                            state.reset_assign_edit();
                            state.persist_debounce = 0;
                            state.apply_pad_paging(true, move |pp| pp.active = drag.from);
                        }
                    }
                }
            }
        }
        Message::PageDragCancel => clear_page_drag(state),
    }
    task
}
/// Drop every in-progress page-row gesture. Each addresses its row by raw slot
/// index, so any path that replaces or hides the row list has to drop them
/// rather than let them survive into freshly-rendered rows holding other pages.
fn clear_page_gestures(state: &mut State) {
    clear_page_drag(state);
    state.confirm_delete_page = None;
    state.editing_page_name = None;
    state.page_name_text.clear();
    state.page_name_debounce = 0;
}

/// End a pointer gesture over the row list. Hover and drag are set and cleared
/// by the same pointer movements, so whatever ends one ends the other: a drag
/// abandoned past the panel edge leaves no row to report an `on_exit`, and a
/// highlight would otherwise stay lit with the pointer nowhere near the list.
fn clear_page_drag(state: &mut State) {
    state.page_drag = None;
    state.hovered_page = None;
}

/// Store row `i`'s typed name, trimmed, without closing the rename field. Runs
/// when typing settles, so an edit abandoned by quitting the GUI (or by any path
/// that never delivers a commit) is not lost. An empty field is left alone: it
/// only resolves to a name on commit, and storing `None` meanwhile would show
/// the user a default name in the row they are still typing into.
fn store_page_name(state: &mut State, i: usize) {
    let trimmed = state.page_name_text.trim().to_string();
    if trimmed.is_empty() {
        return;
    }
    state.apply_pad_paging(true, move |pp| {
        if let Some(page) = pp.pages.get_mut(i) {
            page.name = Some(trimmed);
        }
    });
}

/// Flush whichever row is open for rename once its typing has settled.
fn flush_page_name(state: &mut State) {
    if let Some(i) = state.editing_page_name {
        store_page_name(state, i);
    }
}

/// Store row `i`'s typed page name and close the rename field. Committing an
/// emptied name is a deliberate reset, not a return to a placeholder: the page
/// gets a fresh default letter name so it is never blank and never
/// position-derived.
fn commit_page_name(state: &mut State, i: usize) {
    if state.page_name_text.trim().is_empty() {
        state.apply_pad_paging(true, move |pp| {
            let name = pp.next_page_name();
            if let Some(page) = pp.pages.get_mut(i) {
                page.name = Some(name);
            }
        });
    } else {
        store_page_name(state, i);
    }
    state.page_name_text.clear();
    state.page_name_debounce = 0;
    state.editing_page_name = None;
}

/// Commit whichever row is open for rename, if any. iced's `text_input` has no
/// focus-lost event, so every path that closes the field by user action has to
/// call this — otherwise abandoning a rename by clicking elsewhere persists the
/// raw, untrimmed text (or an empty name) that only Enter would have fixed up.
fn commit_open_page_rename(state: &mut State) {
    if let Some(i) = state.editing_page_name {
        commit_page_name(state, i);
    }
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

    /// A connected `State` that has not yet seen a settings snapshot, plus the
    /// channel the driver would read outgoing frames from.
    fn connected() -> (State, std::sync::mpsc::Receiver<GuiToDriver>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let state = State {
            sender: Some(tx),
            ..State::default()
        };
        (state, rx)
    }

    /// A connected `State` with the default snapshot already adopted.
    fn seeded() -> (State, std::sync::mpsc::Receiver<GuiToDriver>) {
        let (mut state, rx) = connected();
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

    /// Three named pad pages so drag assertions are about page *identity*,
    /// not index — mirrors `app::page_ops_tests::named_pages`.
    fn named_pad_pages(state: &mut State, names: [&str; 3]) {
        let _ = update(state, Message::AddPage);
        let _ = update(state, Message::AddPage);
        // Renaming goes through the open field, exactly as the UI drives it:
        // `SetPageName` outside a rename is not a state the view can produce.
        for (i, name) in names.into_iter().enumerate() {
            let _ = update(state, Message::BeginRenamePage(i));
            let _ = update(state, Message::SetPageName(i, name.to_string()));
            let _ = update(state, Message::CommitPageName(i));
        }
    }

    fn page_names(state: &State) -> Vec<Option<String>> {
        state
            .settings
            .as_ref()
            .unwrap()
            .pad_paging
            .pages
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }

    #[test]
    fn typing_a_page_name_sends_nothing_until_it_settles() {
        let (mut state, rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);
        while rx.try_recv().is_ok() {}

        let _ = update(&mut state, Message::BeginRenamePage(0));
        for typed in ["D", "Dr", "Dru", "Drum", "Drum ", "Drum Bus "] {
            let _ = update(&mut state, Message::SetPageName(0, typed.to_string()));
        }

        assert_eq!(
            state.page_name_text, "Drum Bus ",
            "the field keeps the raw text, spaces and all, so a second word can be typed"
        );
        assert_eq!(
            page_names(&state)[0],
            Some("A".to_string()),
            "the stored name is untouched until typing settles"
        );
        assert!(
            drained_frames(&rx).is_empty(),
            "keystrokes must not each cost the driver an apply"
        );

        // Typing settles: one apply carries the trimmed name.
        assert_eq!(state.page_name_debounce, crate::app::PERSIST_DEBOUNCE_TICKS);
        for _ in 0..crate::app::PERSIST_DEBOUNCE_TICKS {
            let _ = update(&mut state, Message::PersistDebounce);
        }
        assert_eq!(page_names(&state)[0], Some("Drum Bus".to_string()));
        assert_eq!(
            drained_frames(&rx)
                .iter()
                .filter(|f| matches!(f, GuiToDriver::Apply { persist: true, .. }))
                .count(),
            1,
            "six keystrokes cost exactly one persisted apply"
        );
        assert_eq!(
            state.editing_page_name,
            Some(0),
            "settling stores the name without closing the field"
        );
        assert_eq!(
            state.page_name_text, "Drum Bus ",
            "storing a trimmed name must not rewrite what the user is still typing"
        );
    }

    #[test]
    fn committing_a_page_name_persists_it() {
        let (mut state, rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);
        while rx.try_recv().is_ok() {}

        // Enter before the debounce ever fires: the commit is the only thing
        // that can get the typed name onto disk.
        let _ = update(&mut state, Message::BeginRenamePage(0));
        let _ = update(&mut state, Message::SetPageName(0, "Kick".to_string()));
        let _ = update(&mut state, Message::CommitPageName(0));

        assert_eq!(page_names(&state)[0], Some("Kick".to_string()));
        assert!(
            drained_frames(&rx)
                .iter()
                .any(|f| matches!(f, GuiToDriver::Apply { persist: true, .. })),
            "the committed name must be persisted, not left live-only"
        );
        assert_eq!(state.page_name_debounce, 0);
        assert_eq!(state.editing_page_name, None);
    }

    #[test]
    fn committing_an_empty_name_resets_to_a_concrete_name_not_blank() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        // Clear page 1's name (index 1, currently "B") and commit.
        let _ = update(&mut state, Message::BeginRenamePage(1));
        let _ = update(&mut state, Message::SetPageName(1, String::new()));
        let _ = update(&mut state, Message::CommitPageName(1));

        // Never None (that would fall back to a position-derived label that
        // could later go stale) and never blank — a fresh default letter name
        // is stored immediately.
        assert_eq!(
            page_names(&state)[1],
            Some("Pad Page A".to_string()),
            "an empty commit resets to a fresh letter name, not None or blank"
        );
    }

    #[test]
    fn begin_rename_opens_the_row_and_commit_closes_it() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(1));
        assert_eq!(
            state.editing_page_name,
            Some(1),
            "the pencil button opens that row's text_input"
        );

        let _ = update(&mut state, Message::CommitPageName(1));
        assert_eq!(
            state.editing_page_name, None,
            "committing the name closes the row back to plain text"
        );
    }

    #[test]
    fn selecting_a_page_closes_any_open_rename() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(0));
        assert_eq!(state.editing_page_name, Some(0));

        let _ = update(&mut state, Message::SelectPage(2));
        assert_eq!(
            state.editing_page_name, None,
            "switching pages must not leave a stale row stuck in rename mode"
        );
    }

    #[test]
    fn drag_start_over_drop_reorders_the_page_and_clears_drag_state() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::PageDragStart(0));
        let _ = update(&mut state, Message::PageRowEntered(2));
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            page_names(&state),
            vec![Some("B".into()), Some("C".into()), Some("A".into())],
            "page A moved from index 0 to index 2"
        );
        assert!(
            state.page_drag.is_none(),
            "drag state clears once the drop is committed"
        );
    }

    #[test]
    fn drop_with_no_crossing_is_a_reorder_no_op_and_selects_the_origin_row() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        // Press on row 1 and release without ever entering another row: a
        // plain click, not a drag.
        let _ = update(&mut state, Message::PageDragStart(1));
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "no reorder for a plain click"
        );
        assert_eq!(
            state.settings.as_ref().unwrap().pad_paging.active,
            1,
            "the release selects the row that was pressed"
        );
        assert!(state.page_drag.is_none());
    }

    #[test]
    fn drop_back_onto_the_origin_row_is_also_a_reorder_no_op() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::PageDragStart(0));
        let _ = update(&mut state, Message::PageRowEntered(2));
        let _ = update(&mut state, Message::PageRowEntered(0)); // dragged back home
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "over == from must not reorder"
        );
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 0);
        assert!(state.page_drag.is_none());
    }

    #[test]
    fn double_click_rename_survives_the_release_that_ends_it() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        // iced mouse_area double-click order: press → release → press →
        // double-click → release, modeled message-for-message.
        let _ = update(&mut state, Message::PageDragStart(1));
        let _ = update(&mut state, Message::PageDragDrop);
        let _ = update(&mut state, Message::PageDragStart(1));
        let _ = update(&mut state, Message::BeginRenamePage(1));
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            state.editing_page_name,
            Some(1),
            "the release completing a double-click must not close the rename"
        );
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 1);
    }

    #[test]
    fn clicking_another_row_closes_an_open_rename() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(0));
        let _ = update(&mut state, Message::PageDragStart(2));
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            state.editing_page_name, None,
            "clicking a different row must not leave a stale rename open"
        );
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 2);
    }

    #[test]
    fn reordering_rows_closes_an_open_rename() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(0));
        assert_eq!(state.editing_page_name, Some(0));

        // Drag C (2) onto A's row (0): reorder(pp, 2, 0) removes index 2 and
        // inserts at 0, so index 0 now holds C, not the page that was open
        // for rename.
        let _ = update(&mut state, Message::PageDragStart(2));
        let _ = update(&mut state, Message::PageRowEntered(0));
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            state.editing_page_name, None,
            "a raw slot index can't be trusted to still point at the same page after a reorder"
        );
        assert_eq!(
            page_names(&state),
            vec![Some("C".into()), Some("A".into()), Some("B".into())],
            "the reorder itself still happens"
        );
    }

    #[test]
    fn drag_over_is_ignored_without_an_active_drag() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        // No PageDragStart preceded this: a stray hover must not create drag state.
        let _ = update(&mut state, Message::PageRowEntered(2));
        assert!(state.page_drag.is_none());
    }

    #[test]
    fn drag_cancel_clears_state_without_sending_an_apply() {
        let (mut state, rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);
        while rx.try_recv().is_ok() {}

        let _ = update(&mut state, Message::PageDragStart(0));
        let _ = update(&mut state, Message::PageRowEntered(2));
        let _ = update(&mut state, Message::PageDragCancel);

        assert!(
            state.page_drag.is_none(),
            "cancel clears an in-progress drag"
        );
        assert_eq!(
            state.hovered_page, None,
            "the pointer left the list, so no row may stay highlighted — leaving \
             the panel past its bottom edge fires no row `on_exit`"
        );
        assert!(
            rx.try_recv().is_err(),
            "cancelling a drag sends no settings apply"
        );

        // A subsequent drop must not reorder using the cancelled drag's state.
        let _ = update(&mut state, Message::PageDragDrop);
        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "a cancelled drag leaves nothing for a later drop to commit"
        );
    }

    #[test]
    fn switching_inspector_tabs_drops_a_held_drag() {
        // The Assign tab renders no rows, so nothing is left to deliver the
        // release that would otherwise end the gesture.
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::PageDragStart(0));
        let _ = update(&mut state, Message::PageRowEntered(2));
        let _ = update(
            &mut state,
            Message::SetInspectorTab(crate::message::InspectorTab::Assign),
        );

        assert!(state.page_drag.is_none());
        assert_eq!(state.hovered_page, None);
    }

    #[test]
    fn adopted_settings_snapshot_clears_an_in_progress_drag() {
        use std::sync::Arc;
        let (mut state, _rx) = seeded();
        // Keep `seq`/`last_acked_seq` both at 0 (no local edits in between) so
        // the snapshot below is guaranteed to be adopted, matching the real
        // bug: paging can be disabled (or otherwise changed) out from under a
        // held drag by a snapshot that lands mid-gesture.
        let mut settings = Settings::default();
        crate::app::page_ops::add(&mut settings.pad_paging);
        state.settings = Some(Arc::new(settings));

        let _ = update(&mut state, Message::PageDragStart(0));
        assert!(state.page_drag.is_some());

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::default())),
        );

        assert!(
            state.page_drag.is_none(),
            "an adopted settings snapshot must not leave a drag orphaned \
             — the row list it belonged to may no longer exist"
        );
    }

    #[test]
    fn adopted_settings_snapshot_clears_an_in_progress_rename() {
        use std::sync::Arc;
        let (mut state, _rx) = seeded();
        let mut settings = Settings::default();
        crate::app::page_ops::add(&mut settings.pad_paging);
        state.settings = Some(Arc::new(settings));

        let _ = update(&mut state, Message::BeginRenamePage(0));
        assert!(state.editing_page_name.is_some());

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::default())),
        );

        assert!(
            state.editing_page_name.is_none(),
            "an adopted settings snapshot must not leave a rename open on a \
             row that may have moved or no longer exists"
        );
    }

    #[test]
    fn double_click_rename_survives_the_drivers_echo_of_the_select_apply() {
        let (mut state, rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);
        let _ = drained_frames(&rx);

        // A double-click is press/release/press/double-click/release: the final
        // release lands as a no-move `PageDragDrop`, which selects the page with
        // a persisted apply *after* the rename field has already opened.
        let _ = update(&mut state, Message::PageDragStart(1));
        let _ = update(&mut state, Message::PageDragDrop);
        let _ = update(&mut state, Message::PageDragStart(1));
        let _ = update(&mut state, Message::BeginRenamePage(1));
        let _ = update(&mut state, Message::PageDragDrop);
        assert_eq!(state.editing_page_name, Some(1));

        let seq = drained_frames(&rx)
            .iter()
            .filter_map(|frame| match frame {
                GuiToDriver::Apply { seq, .. } => Some(*seq),
                _ => None,
            })
            .max()
            .expect("the selection applies");

        // The driver acks and then echoes the state it just stored back.
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq,
                result: Ok(()),
            }),
        );
        let echo = (**state.settings.as_ref().unwrap()).clone();
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::new(echo))),
        );

        assert_eq!(
            state.editing_page_name,
            Some(1),
            "the driver echoing back the state the GUI already shows must not \
             close the rename the same gesture just opened"
        );
    }

    #[test]
    fn the_first_snapshot_names_unnamed_pages_and_persists() {
        let (mut state, rx) = connected();

        let mut settings = Settings::default();
        settings
            .pad_paging
            .pages
            .push(settings::pad_paging::default_page());
        settings.pad_paging.pages[0].name = None;
        settings.pad_paging.pages[1].name = Some("Kick".to_string());

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::new(settings))),
        );

        assert_eq!(
            page_names(&state),
            vec![Some("Pad Page A".to_string()), Some("Kick".to_string())],
            "a migrated config's unnamed pages get fresh default letter names"
        );
        assert!(
            drained_frames(&rx)
                .iter()
                .any(|f| matches!(f, GuiToDriver::Apply { persist: true, .. })),
            "the assigned names persist"
        );
    }

    #[test]
    fn the_default_snapshot_writes_nothing_on_launch() {
        let (mut state, rx) = connected();

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::default())),
        );

        assert!(
            rx.try_recv().is_err(),
            "a fresh default config has no unnamed page, so merely launching the \
             GUI must not write a settings file the user never edited"
        );
    }

    #[test]
    fn a_snapshot_never_disturbs_a_rename_in_progress() {
        let (mut state, rx) = seeded();

        // The user opens a rename and clears the field. The stored name stays
        // put — an empty field is a state of the field, not of the page, so no
        // page is ever momentarily unnamed for the migration to "fix".
        let _ = update(&mut state, Message::BeginRenamePage(0));
        let _ = update(&mut state, Message::SetPageName(0, String::new()));
        state.last_acked_seq = state.seq;
        while rx.try_recv().is_ok() {}

        assert_eq!(page_names(&state)[0], Some("Pad Page A".to_string()));

        // The driver echoes back exactly what the GUI already shows.
        let echo = (**state.settings.as_ref().unwrap()).clone();
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::new(echo))),
        );

        assert_eq!(
            state.editing_page_name,
            Some(0),
            "an echoed snapshot must not close the field the user is typing in"
        );
        assert_eq!(state.page_name_text, "");
        assert!(rx.try_recv().is_err(), "and must not apply anything either");
    }

    #[test]
    fn normalization_is_attempted_once_not_looped_on_rejection() {
        let (mut state, rx) = connected();

        let mut settings = Settings::default();
        settings.pad_paging.pages[0].name = None;

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::new(settings.clone()))),
        );

        let frames = drained_frames(&rx);
        assert_eq!(
            frames.len(),
            1,
            "exactly one normalization apply is sent for the unnamed page"
        );
        let seq = match frames[0] {
            GuiToDriver::Apply {
                seq, persist: true, ..
            } => seq,
            ref other => panic!("expected a persisted Apply, got {other:?}"),
        };

        // The driver rejects the persist (e.g. disk full): settings revert to
        // the still-unnamed authoritative snapshot and a resync is requested.
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq,
                result: Err("disk full".to_string()),
            }),
        );
        let _ = drained_frames(&rx); // the resync GetSettings

        // The resync snapshot comes back just as unnamed as before, since the
        // persist never landed.
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::new(settings))),
        );

        assert!(
            drained_frames(&rx).is_empty(),
            "normalization must not retry after its one attempt was rejected"
        );
    }

    #[test]
    fn disabling_paging_clears_an_in_progress_drag() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::PageDragStart(0));
        assert!(state.page_drag.is_some());

        let _ = update(&mut state, Message::SetPagingEnabled(false));

        assert!(
            state.page_drag.is_none(),
            "disabling paging removes the row list; a held drag can't survive it"
        );
    }

    #[test]
    fn disabling_paging_clears_an_in_progress_rename() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(0));
        assert!(state.editing_page_name.is_some());

        let _ = update(&mut state, Message::SetPagingEnabled(false));

        assert!(
            state.editing_page_name.is_none(),
            "disabling paging removes the row list; an open rename can't survive it"
        );
    }

    #[test]
    fn drop_with_out_of_range_over_performs_no_reorder() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        // A stale drag whose `over` no longer fits the current pages (e.g.
        // if some future path failed to clear `page_drag` before the list
        // shrank). `PageDragDrop` must re-validate rather than trust it.
        state.page_drag = Some(crate::app::PageDrag {
            from: 0,
            over: Some(99),
        });
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "an out-of-range `over` must not reorder or change `active`"
        );
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 0);
        assert!(
            state.page_drag.is_none(),
            "the drop still clears drag state"
        );
    }

    #[test]
    fn drop_with_out_of_range_from_performs_no_reorder() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        state.page_drag = Some(crate::app::PageDrag {
            from: 99,
            over: Some(1),
        });
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "an out-of-range `from` must not reorder or change `active`"
        );
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 0);
    }

    #[test]
    fn select_page_resets_an_in_progress_assign_edit() {
        use crate::inspector::assign::numeric::EditField;
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        state.edit_field = Some(EditField::PadHitNote);
        state.edit_text = "6".to_string();
        state.persist_debounce = crate::app::PERSIST_DEBOUNCE_TICKS;

        let _ = update(&mut state, Message::SelectPage(1));

        assert!(
            state.edit_field.is_none(),
            "switching pages must clear an in-progress edit field"
        );
        assert!(
            state.edit_text.is_empty(),
            "switching pages must clear in-progress edit text"
        );
        assert_eq!(
            state.persist_debounce, 0,
            "switching pages must clear any armed persist debounce so it can't \
             flush an edit meant for the old page onto the new one"
        );
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 1);
    }

    #[test]
    fn page_drag_drop_select_resets_an_in_progress_assign_edit() {
        use crate::inspector::assign::numeric::EditField;
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        state.edit_field = Some(EditField::PadHitNote);
        state.edit_text = "6".to_string();
        state.persist_debounce = crate::app::PERSIST_DEBOUNCE_TICKS;

        // A plain click (press + release with no crossing) on another row
        // takes the `PageDragDrop` select branch, not the reorder branch.
        let _ = update(&mut state, Message::PageDragStart(1));
        let _ = update(&mut state, Message::PageDragDrop);

        assert!(
            state.edit_field.is_none(),
            "selecting a page via the drag/click path must clear an in-progress edit field"
        );
        assert_eq!(state.persist_debounce, 0);
        assert_eq!(state.settings.as_ref().unwrap().pad_paging.active, 1);
    }

    #[test]
    fn request_delete_page_opens_the_dialog_without_deleting() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::RequestDeletePage(1));

        assert_eq!(
            state.confirm_delete_page,
            Some(1),
            "the row action opens the confirmation dialog"
        );
        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "nothing is deleted until the dialog is confirmed"
        );
    }

    #[test]
    fn confirm_delete_page_deletes_and_closes_the_dialog() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::RequestDeletePage(1));
        let _ = update(&mut state, Message::ConfirmDeletePage);

        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("C".into())],
            "confirming deletes the page the dialog was opened for"
        );
        assert!(
            state.confirm_delete_page.is_none(),
            "confirming closes the dialog"
        );
    }

    #[test]
    fn cancel_delete_page_closes_the_dialog_without_deleting() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::RequestDeletePage(1));
        let _ = update(&mut state, Message::CancelDeletePage);

        assert!(state.confirm_delete_page.is_none());
        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())],
            "cancelling deletes nothing"
        );
    }

    #[test]
    fn confirm_delete_page_without_a_pending_request_is_a_noop() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::ConfirmDeletePage);

        assert_eq!(
            page_names(&state),
            vec![Some("A".into()), Some("B".into()), Some("C".into())]
        );
    }

    #[test]
    fn disabling_paging_clears_an_open_delete_confirmation() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::RequestDeletePage(1));
        assert!(state.confirm_delete_page.is_some());

        let _ = update(&mut state, Message::SetPagingEnabled(false));

        assert!(
            state.confirm_delete_page.is_none(),
            "disabling paging removes the row list; an open dialog can't survive it"
        );
    }

    #[test]
    fn a_hardware_page_switch_keeps_open_row_gestures() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);
        state.last_acked_seq = state.seq;

        let _ = update(&mut state, Message::BeginRenamePage(0));
        let _ = update(&mut state, Message::RequestDeletePage(0));

        // Holding Group and tapping a pad makes the driver push a full snapshot
        // per tap, differing from what the GUI already shows only in `active`.
        let mut echo = (**state.settings.as_ref().unwrap()).clone();
        echo.pad_paging.active = 2;
        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::new(echo))),
        );

        assert_eq!(
            state.settings.as_ref().unwrap().pad_paging.active,
            2,
            "the snapshot is still adopted"
        );
        assert_eq!(
            state.editing_page_name,
            Some(0),
            "a page switch moves no row, so it must not close an open rename"
        );
        assert_eq!(state.confirm_delete_page, Some(0));
    }

    #[test]
    fn clicking_another_row_commits_the_typed_name() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(0));
        let _ = update(&mut state, Message::SetPageName(0, "Drum Bus ".to_string()));

        // Abandoning the rename by clicking another row: `text_input` reports no
        // focus loss, so without an explicit commit the untrimmed text persists.
        let _ = update(&mut state, Message::PageDragStart(2));
        let _ = update(&mut state, Message::PageDragDrop);

        assert_eq!(
            page_names(&state)[0],
            Some("Drum Bus".to_string()),
            "closing the field trims exactly like Enter does"
        );
        assert_eq!(state.editing_page_name, None);
        assert_eq!(
            state.persist_debounce, 0,
            "the commit flushes rather than leaving a debounce armed"
        );
    }

    #[test]
    fn abandoning_a_cleared_name_resets_it_instead_of_leaving_it_blank() {
        use crate::message::InspectorTab;
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::BeginRenamePage(1));
        let _ = update(&mut state, Message::SetPageName(1, String::new()));
        let _ = update(&mut state, Message::SetInspectorTab(InspectorTab::Assign));

        assert_eq!(
            page_names(&state)[1],
            Some("Pad Page A".to_string()),
            "leaving the tab resets an emptied name, never storing None"
        );
        assert_eq!(state.editing_page_name, None);
    }

    #[test]
    fn leaving_the_pages_tab_drops_a_stale_row_hover() {
        use crate::message::InspectorTab;
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);

        let _ = update(&mut state, Message::PageRowEntered(1));
        assert_eq!(state.hovered_page, Some(1));

        let _ = update(&mut state, Message::SetInspectorTab(InspectorTab::Assign));

        assert!(
            state.hovered_page.is_none(),
            "the row's `on_exit` can never fire once the row is gone, so the \
             hover would otherwise persist into the next visit to the tab"
        );
    }

    #[test]
    fn a_rejected_apply_rollback_clears_page_gestures() {
        let (mut state, _rx) = seeded();
        named_pad_pages(&mut state, ["A", "B", "C"]);
        // The driver has confirmed the three pages.
        state.authoritative = state.settings.clone();

        let _ = update(&mut state, Message::RequestDeletePage(2));
        let _ = update(&mut state, Message::BeginRenamePage(2));
        let _ = update(&mut state, Message::AddPage);
        let seq = state.seq;

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Ack {
                seq,
                result: Err("read-only config directory".to_string()),
            }),
        );

        assert_eq!(
            page_names(&state).len(),
            3,
            "the rejected page is rolled back"
        );
        assert!(
            state.confirm_delete_page.is_none() && state.editing_page_name.is_none(),
            "gestures addressing a row the rollback removed must go with it"
        );
    }

    #[test]
    fn adopted_settings_snapshot_clears_an_open_delete_confirmation() {
        use std::sync::Arc;
        let (mut state, _rx) = seeded();
        let mut settings = Settings::default();
        crate::app::page_ops::add(&mut settings.pad_paging);
        state.settings = Some(Arc::new(settings));

        let _ = update(&mut state, Message::RequestDeletePage(0));
        assert!(state.confirm_delete_page.is_some());

        let _ = update(
            &mut state,
            Message::Frame(DriverToGui::Settings(Box::default())),
        );

        assert!(
            state.confirm_delete_page.is_none(),
            "an adopted settings snapshot must not leave a delete confirmation \
             orphaned — the page it named may no longer exist"
        );
    }
}
