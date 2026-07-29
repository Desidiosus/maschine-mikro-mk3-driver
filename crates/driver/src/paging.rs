use maschine_library::controls::Buttons;
use maschine_library::lights::{Brightness, PadColors};
use settings::PadPaging;
use settings::pads_by_index::internal_to_config_key;

use crate::events::ControlEvent;
use crate::outputs::DeviceOutputs;

/// What the loop must do with an event while paging is enabled. Every variant
/// except `None` means the event was swallowed (it does not reach the backend or
/// the `ControlActuated` stream).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingAction {
    /// Not a paging event — forward it through the normal pipeline.
    None,
    /// Swallowed, no picker transition.
    Swallow,
    /// `Group` pressed: the page picker just opened.
    OpenPicker,
    /// A pad selected page `usize` (in range) while the picker is open.
    SelectPage(usize),
    /// `Group` released: the picker closed on active page `usize`.
    ClosePicker(usize),
}

/// Tracks the `Group`-held page picker. Pure: the only side effect is
/// `render_picker`, which the loop calls separately.
pub struct PagingState {
    group_pressed: bool,
    pending_active: Option<usize>,
}

impl PagingState {
    pub fn new() -> Self {
        Self {
            group_pressed: false,
            pending_active: None,
        }
    }

    /// Drop any in-progress hold. Called when paging is disabled mid-session so a
    /// later re-enable starts clean (the release edge is never observed once
    /// disabled).
    pub fn reset(&mut self) {
        self.group_pressed = false;
        self.pending_active = None;
    }

    pub fn is_picking(&self) -> bool {
        self.group_pressed
    }

    pub fn pending_active(&self) -> Option<usize> {
        self.pending_active
    }

    pub fn observe_event(
        &mut self,
        active: usize,
        page_count: usize,
        event: &ControlEvent,
    ) -> PagingAction {
        match event {
            ControlEvent::ButtonChanged { index, pressed } if *index == Buttons::Group as usize => {
                if *pressed {
                    if self.group_pressed {
                        PagingAction::Swallow
                    } else {
                        self.group_pressed = true;
                        self.pending_active = Some(active);
                        PagingAction::OpenPicker
                    }
                } else if self.group_pressed {
                    let final_page = self.pending_active.take().unwrap_or(active);
                    self.group_pressed = false;
                    PagingAction::ClosePicker(final_page)
                } else {
                    // A release with no hold latched: the press was swallowed and
                    // then the hold was dropped (soft-off, or paging re-enabled
                    // mid-hold). `Group` is reserved while paging is enabled, so
                    // forwarding this would emit a CC release with no press.
                    PagingAction::Swallow
                }
            }
            ControlEvent::PadNoteOn { index, .. } if self.group_pressed => {
                match page_for_pad(*index) {
                    // Out of range, or already the pending page — nothing changes.
                    Some(page) if page < page_count && self.pending_active != Some(page) => {
                        self.pending_active = Some(page);
                        PagingAction::SelectPage(page)
                    }
                    _ => PagingAction::Swallow,
                }
            }
            // NOTE: pad NoteOff is deliberately NOT swallowed while the picker is
            // open. A pad pressed before the hold has an outstanding note that must
            // still be released; the backend only emits a NoteOff for pads with a
            // held note, so picker taps (whose NoteOn was swallowed) stay silent.
            //
            // Aftertouch is different: the backend now falls back to resolving
            // aftertouch against the current page when there is no held note (the
            // device ramps pressure before/after a hit), so a picker tap's
            // aftertouch would otherwise resolve and emit MIDI. Swallow it here to
            // keep picker taps silent; a note held from before the Group hold
            // simply stops receiving pressure updates for the duration of the hold.
            ControlEvent::PadAftertouch { .. } if self.group_pressed => PagingAction::Swallow,
            _ => PagingAction::None,
        }
    }
}

impl Default for PagingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Page shown on the pad at native LED index `pad`, laid out in the device's
/// printed pad numbering: page 1 on physical pad 1 (bottom-left), running left to
/// right and bottom to top. `None` for an index outside the 4x4 grid.
fn page_for_pad(pad: usize) -> Option<usize> {
    (pad < 16).then(|| internal_to_config_key(pad) - 1)
}

/// Render the page picker onto the 16 pad LEDs: physical pad `N` shows page `N`'s
/// resolved color, the `pending_active` pad is bright and other in-range pads are
/// dim, and pads past the page count are off.
pub fn render_picker(outputs: &DeviceOutputs, paging: &PadPaging, pending_active: usize) {
    outputs.with_lights_mut(|lights| {
        for pad in 0..16 {
            let page_index = page_for_pad(pad).expect("pad index is within the 4x4 grid");
            match paging.pages.get(page_index) {
                Some(page) => {
                    let color = paging.page_color(page);
                    let brightness = if page_index == pending_active {
                        Brightness::Bright
                    } else {
                        Brightness::Dim
                    };
                    lights.set_pad(pad, color, brightness);
                }
                None => lights.set_pad(pad, PadColors::Off, Brightness::Off),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::pads_by_index::config_key_to_internal;

    fn group(pressed: bool) -> ControlEvent {
        ControlEvent::ButtonChanged {
            index: Buttons::Group as usize,
            pressed,
        }
    }

    fn pad_on(index: usize) -> ControlEvent {
        ControlEvent::PadNoteOn {
            index,
            velocity: 100,
        }
    }

    #[test]
    fn group_hold_then_pad_selects_page_and_swallows() {
        let mut p = PagingState::new();
        // active=0, 4 pages.
        assert_eq!(
            p.observe_event(0, 4, &group(true)),
            PagingAction::OpenPicker
        );
        assert!(p.is_picking());
        assert_eq!(
            p.observe_event(0, 4, &pad_on(config_key_to_internal(3))),
            PagingAction::SelectPage(2)
        );
        assert_eq!(p.pending_active(), Some(2));
        // Release commits page 2.
        assert_eq!(
            p.observe_event(0, 4, &group(false)),
            PagingAction::ClosePicker(2)
        );
        assert!(!p.is_picking());
    }

    #[test]
    fn release_without_selection_closes_on_current_active() {
        let mut p = PagingState::new();
        p.observe_event(3, 4, &group(true));
        assert_eq!(
            p.observe_event(3, 4, &group(false)),
            PagingAction::ClosePicker(3)
        );
    }

    #[test]
    fn pad_beyond_page_count_is_swallowed_not_selected() {
        let mut p = PagingState::new();
        p.observe_event(0, 2, &group(true));
        // Only 2 pages exist; physical pad 5 must not select a nonexistent page.
        assert_eq!(
            p.observe_event(0, 2, &pad_on(config_key_to_internal(5))),
            PagingAction::Swallow
        );
        assert_eq!(p.pending_active(), Some(0));
    }

    #[test]
    fn pad_release_is_forwarded_while_picking_so_held_notes_can_stop() {
        let mut p = PagingState::new();
        p.observe_event(0, 4, &group(true));
        let off = ControlEvent::PadNoteOff {
            index: 3,
            velocity: 0,
        };
        assert_eq!(
            p.observe_event(0, 4, &off),
            PagingAction::None,
            "a pad release must reach the backend so an outstanding note is released"
        );
    }

    #[test]
    fn aftertouch_is_swallowed_while_picking_but_note_off_still_forwards() {
        let mut p = PagingState::new();
        p.observe_event(0, 4, &group(true));

        let aftertouch = ControlEvent::PadAftertouch {
            index: 3,
            pressure: 50,
        };
        assert_eq!(
            p.observe_event(0, 4, &aftertouch),
            PagingAction::Swallow,
            "a picker tap's aftertouch must not resolve to MIDI"
        );

        let off = ControlEvent::PadNoteOff {
            index: 3,
            velocity: 0,
        };
        assert_eq!(
            p.observe_event(0, 4, &off),
            PagingAction::None,
            "a pad release must still reach the backend so an outstanding note is released"
        );
    }

    #[test]
    fn events_forward_when_not_holding_group() {
        let mut p = PagingState::new();
        assert_eq!(p.observe_event(0, 4, &pad_on(1)), PagingAction::None);
        assert_eq!(
            p.observe_event(0, 4, &ControlEvent::EncoderTurn { delta: 1 }),
            PagingAction::None
        );
    }

    #[test]
    fn reset_drops_the_hold() {
        let mut p = PagingState::new();
        p.observe_event(0, 4, &group(true));
        assert!(p.is_picking());

        // Soft-off (or a disable) tears the hold down without a release edge.
        p.reset();
        assert!(!p.is_picking());
        assert_eq!(p.pending_active(), None);

        // The physical release arrives later; it must not close a picker that
        // is no longer open, must not be treated as a page commit, and must not
        // reach the backend as an unpaired `Group` CC release.
        assert_eq!(p.observe_event(0, 4, &group(false)), PagingAction::Swallow);
        assert!(!p.is_picking());
    }

    #[test]
    fn tapping_the_already_selected_page_does_not_reselect() {
        let mut p = PagingState::new();
        // Opens with active page 2 pending.
        assert_eq!(
            p.observe_event(2, 4, &group(true)),
            PagingAction::OpenPicker
        );
        // Tapping physical pad 3 (already pending) must not emit another SelectPage.
        assert_eq!(
            p.observe_event(2, 4, &pad_on(config_key_to_internal(3))),
            PagingAction::Swallow
        );
        // Tapping a different page still selects.
        assert_eq!(
            p.observe_event(2, 4, &pad_on(config_key_to_internal(2))),
            PagingAction::SelectPage(1)
        );
        // Tapping it again is now a no-op too.
        assert_eq!(
            p.observe_event(2, 4, &pad_on(config_key_to_internal(2))),
            PagingAction::Swallow
        );
    }

    #[test]
    fn the_first_page_sits_on_physical_pad_1() {
        let outputs = DeviceOutputs::new();
        let mut paging = settings::pad_paging::default_pad_paging();
        paging.default_page_color = PadColors::Cyan;

        render_picker(&outputs, &paging, 0);

        assert_eq!(
            outputs.with_lights(|l| l.get_pad(config_key_to_internal(1))),
            (PadColors::Cyan, Brightness::Bright),
            "page 1 belongs on physical pad 1 (bottom-left)"
        );
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(config_key_to_internal(13))),
            (PadColors::Off, Brightness::Off),
            "physical pad 13 (top-left) has no page in a one-page config"
        );
    }

    #[test]
    fn tapping_physical_pad_1_selects_the_first_page() {
        let mut p = PagingState::new();
        p.observe_event(1, 4, &group(true));
        assert_eq!(
            p.observe_event(1, 4, &pad_on(config_key_to_internal(1))),
            PagingAction::SelectPage(0),
            "the bottom-left pad selects page 1"
        );
    }

    #[test]
    fn render_picker_lights_pages_and_darkens_the_rest() {
        let outputs = DeviceOutputs::new();
        let mut paging = settings::pad_paging::default_pad_paging();
        paging.default_page_color = PadColors::Cyan;
        paging.pages.push(paging.new_page()); // 2 pages now

        render_picker(&outputs, &paging, 1);

        // Page 0 on physical pad 1: in range, not pending → dim, default color.
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(config_key_to_internal(1))),
            (PadColors::Cyan, Brightness::Dim)
        );
        // Page 1 on physical pad 2: pending → bright.
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(config_key_to_internal(2))),
            (PadColors::Cyan, Brightness::Bright)
        );
        // Physical pad 3: no page → off.
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(config_key_to_internal(3))),
            (PadColors::Off, Brightness::Off)
        );
    }
}
