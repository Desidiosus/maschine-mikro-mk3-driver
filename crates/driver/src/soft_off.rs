use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

use maschine_library::controls::Buttons;
use maschine_library::lights::Lights;
use maschine_library::screen::Screen;

use crate::events::ControlEvent;
use crate::outputs::DeviceOutputs;

/// Shared soft-off flag + gate. Cloned into the MIDI input thread so its
/// callback can synchronize with `SoftOffState` toggles in the main loop:
/// the gate is held during sleep/wake transitions, and the flag tells the
/// callback to drop DAW feedback while soft-off is active.
#[derive(Clone)]
pub struct SoftOffSync {
    active: Arc<AtomicBool>,
    gate: Arc<Mutex<()>>,
}

impl SoftOffSync {
    pub fn new() -> Self {
        Self {
            active: Arc::new(AtomicBool::new(false)),
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub fn lock(&self) -> MutexGuard<'_, ()> {
        self.gate.lock().unwrap()
    }
}

impl Default for SoftOffSync {
    fn default() -> Self {
        Self::new()
    }
}

/// Outcome of `SoftOffState::observe_event`: whether the event should
/// continue through the runtime pipeline (local feedback + backend
/// dispatch) or be dropped because soft-off swallowed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftOffOutcome {
    Forward,
    Swallow,
}

/// Tracks the `Shift+Maschine` toggle, blanks/restores outputs, and
/// suppresses the release events of the wake combo so the next press
/// is not immediately re-toggled.
pub struct SoftOffState {
    active: bool,
    shift_pressed: bool,
    maschine_pressed: bool,
    suppress_combo_releases_until_clear: bool,
    sync: SoftOffSync,
    snapshot: Option<OutputSnapshot>,
}

impl SoftOffState {
    pub fn new(sync: SoftOffSync) -> Self {
        Self {
            active: false,
            shift_pressed: false,
            maschine_pressed: false,
            suppress_combo_releases_until_clear: false,
            sync,
            snapshot: None,
        }
    }

    pub fn sync(&self) -> SoftOffSync {
        self.sync.clone()
    }

    #[cfg(test)]
    fn is_active(&self) -> bool {
        self.active
    }

    pub fn observe_event(
        &mut self,
        outputs: &DeviceOutputs,
        event: &ControlEvent,
    ) -> SoftOffOutcome {
        if self.suppress_combo_releases_until_clear && self.is_combo_event(event) {
            self.track_combo_state(event);
            if !self.shift_pressed && !self.maschine_pressed {
                self.suppress_combo_releases_until_clear = false;
            }
            return SoftOffOutcome::Swallow;
        }

        let was_active = self.active;
        let toggled = self.track_combo_event(outputs, event);

        if toggled || was_active || self.active {
            SoftOffOutcome::Swallow
        } else {
            SoftOffOutcome::Forward
        }
    }

    fn track_combo_event(&mut self, outputs: &DeviceOutputs, event: &ControlEvent) -> bool {
        if !self.is_combo_event(event) {
            return false;
        }

        let combo_was_pressed = self.shift_pressed && self.maschine_pressed;
        let pressed = self.track_combo_state(event);

        let combo_is_pressed = self.shift_pressed && self.maschine_pressed;
        if !combo_was_pressed && combo_is_pressed && pressed {
            self.toggle(outputs);
            return true;
        }

        false
    }

    fn toggle(&mut self, outputs: &DeviceOutputs) {
        if self.active {
            self.wake(outputs);
        } else {
            self.sleep(outputs);
        }
    }

    fn sleep(&mut self, outputs: &DeviceOutputs) {
        let _guard = self.sync.lock();
        self.active = true;
        self.sync.active.store(true, Ordering::SeqCst);
        self.snapshot = Some(OutputSnapshot::capture(outputs));
        blank_outputs(outputs);
    }

    fn wake(&mut self, outputs: &DeviceOutputs) {
        self.wake_inner(outputs, || {});
    }

    fn wake_inner(&mut self, outputs: &DeviceOutputs, on_restored: impl FnOnce()) {
        let _guard = self.sync.lock();
        self.active = false;
        if let Some(snapshot) = self.snapshot.take() {
            snapshot.restore(outputs);
        }
        self.suppress_combo_releases_until_clear = true;
        on_restored();
        self.sync.active.store(false, Ordering::SeqCst);
    }

    fn is_combo_event(&self, event: &ControlEvent) -> bool {
        let ControlEvent::ButtonChanged { index, .. } = event else {
            return false;
        };

        *index == Buttons::Shift as usize || *index == Buttons::Maschine as usize
    }

    fn track_combo_state(&mut self, event: &ControlEvent) -> bool {
        let ControlEvent::ButtonChanged { index, pressed } = event else {
            return false;
        };

        if *index == Buttons::Shift as usize {
            self.shift_pressed = *pressed;
        }
        if *index == Buttons::Maschine as usize {
            self.maschine_pressed = *pressed;
        }

        *pressed
    }
}

struct OutputSnapshot {
    lights: Lights,
    screen: Screen,
}

impl OutputSnapshot {
    fn capture(outputs: &DeviceOutputs) -> Self {
        Self {
            lights: outputs.with_lights(Clone::clone),
            screen: outputs.with_screen(Clone::clone),
        }
    }

    fn restore(self, outputs: &DeviceOutputs) {
        outputs.with_lights_mut(|lights| *lights = self.lights);
        outputs.with_screen_mut(|screen| *screen = self.screen);
    }
}

fn blank_outputs(outputs: &DeviceOutputs) {
    outputs.with_lights_mut(Lights::reset);
    outputs.with_screen_mut(Screen::reset);
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use maschine_library::controls::Buttons;
    use maschine_library::lights::{Brightness, PadColors};

    use super::{SoftOffOutcome, SoftOffState, SoftOffSync};
    use crate::events::ControlEvent;
    use crate::outputs::DeviceOutputs;

    fn button_event(button: Buttons, pressed: bool) -> ControlEvent {
        ControlEvent::ButtonChanged {
            index: button as usize,
            pressed,
        }
    }

    #[test]
    fn combo_toggles_only_on_press_edge() {
        let outputs = DeviceOutputs::new();
        let mut soft_off = SoftOffState::new(SoftOffSync::new());

        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Shift, true)),
            SoftOffOutcome::Forward
        );
        assert!(!soft_off.is_active());

        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true)),
            SoftOffOutcome::Swallow
        );
        assert!(soft_off.is_active());

        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, false)),
            SoftOffOutcome::Swallow
        );
        assert!(soft_off.is_active());

        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true)),
            SoftOffOutcome::Swallow
        );
        assert!(!soft_off.is_active());
    }

    #[test]
    fn blanks_outputs_and_restores_snapshot_on_wake() {
        let outputs = DeviceOutputs::new();
        let mut soft_off = SoftOffState::new(SoftOffSync::new());

        outputs.with_lights_mut(|lights| {
            lights.set_button(Buttons::Play, Brightness::Bright);
            lights.set_pad(0, PadColors::Cyan, Brightness::Normal);
        });
        outputs.with_screen_mut(|screen| screen.set(0, 0, true));

        soft_off.observe_event(&outputs, &button_event(Buttons::Shift, true));
        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true));

        assert_eq!(
            outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
            Brightness::Off
        );
        assert_eq!(
            outputs.with_lights(|lights| lights.get_pad(0)),
            (PadColors::Off, Brightness::Off)
        );
        assert!(!outputs.with_screen(|screen| screen.get(0, 0)));

        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, false));
        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true));

        assert_eq!(
            outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
            Brightness::Bright
        );
        assert_eq!(
            outputs.with_lights(|lights| lights.get_pad(0)),
            (PadColors::Cyan, Brightness::Normal)
        );
        assert!(outputs.with_screen(|screen| screen.get(0, 0)));
    }

    #[test]
    fn wake_restores_snapshot_before_clearing_shared_flag() {
        let outputs = DeviceOutputs::new();
        let sync = SoftOffSync::new();
        let mut soft_off = SoftOffState::new(sync.clone());

        outputs.with_lights_mut(|lights| {
            lights.set_button(Buttons::Play, Brightness::Bright);
        });

        soft_off.observe_event(&outputs, &button_event(Buttons::Shift, true));
        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true));

        let flag_seen_during_restore = Cell::new(false);
        soft_off.wake_inner(&outputs, || {
            flag_seen_during_restore.set(sync.is_active());
        });

        assert!(flag_seen_during_restore.get());
        assert!(!sync.is_active());
        assert_eq!(
            outputs.with_lights(|lights| lights.get_button(Buttons::Play)),
            Brightness::Bright
        );
    }

    #[test]
    fn release_events_are_suppressed_until_combo_buttons_clear() {
        let outputs = DeviceOutputs::new();
        let mut soft_off = SoftOffState::new(SoftOffSync::new());

        soft_off.observe_event(&outputs, &button_event(Buttons::Shift, true));
        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true));
        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, false));
        soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, true));

        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Maschine, false)),
            SoftOffOutcome::Swallow
        );
        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Shift, false)),
            SoftOffOutcome::Swallow
        );

        assert_eq!(
            soft_off.observe_event(&outputs, &button_event(Buttons::Play, false)),
            SoftOffOutcome::Forward
        );
    }
}
