use maschine_library::controls::Buttons;
use maschine_library::lights::{Brightness, PadColors};
use settings::PadPaging;

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
                if *index < page_count {
                    self.pending_active = Some(*index);
                    PagingAction::SelectPage(*index)
                } else {
                    PagingAction::Swallow
                }
            }
            ControlEvent::PadNoteOff { .. } | ControlEvent::PadAftertouch { .. }
                if self.group_pressed =>
            {
                PagingAction::Swallow
            }
            _ => PagingAction::None,
        }
    }
}

impl Default for PagingState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the page picker onto the 16 pad LEDs: pad `N` shows page `N`'s resolved
/// color, the `pending_active` pad is bright and other in-range pads are dim, and
/// pads past the page count are off.
pub fn render_picker(outputs: &DeviceOutputs, paging: &PadPaging, pending_active: usize) {
    outputs.with_lights_mut(|lights| {
        for index in 0..16 {
            match paging.pages.get(index) {
                Some(page) => {
                    let color = paging.page_color(page);
                    let brightness = if index == pending_active {
                        Brightness::Bright
                    } else {
                        Brightness::Dim
                    };
                    lights.set_pad(index, color, brightness);
                }
                None => lights.set_pad(index, PadColors::Off, Brightness::Off),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
            p.observe_event(0, 4, &pad_on(2)),
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
        // Only 2 pages exist; pad 5 must not select a nonexistent page.
        assert_eq!(p.observe_event(0, 2, &pad_on(5)), PagingAction::Swallow);
        assert_eq!(p.pending_active(), Some(0));
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
        p.reset();
        assert!(!p.is_picking());
        assert_eq!(p.pending_active(), None);
    }

    #[test]
    fn render_picker_lights_pages_and_darkens_the_rest() {
        let outputs = DeviceOutputs::new();
        let mut paging = settings::pad_paging::default_pad_paging();
        paging.default_page_color = PadColors::Cyan;
        paging.pages.push(paging.new_page()); // 2 pages now

        render_picker(&outputs, &paging, 1);

        // Page 0: in range, not pending → dim, default color.
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(0)),
            (PadColors::Cyan, Brightness::Dim)
        );
        // Page 1: pending → bright.
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(1)),
            (PadColors::Cyan, Brightness::Bright)
        );
        // Pad 2: no page → off.
        assert_eq!(
            outputs.with_lights(|l| l.get_pad(2)),
            (PadColors::Off, Brightness::Off)
        );
    }
}
