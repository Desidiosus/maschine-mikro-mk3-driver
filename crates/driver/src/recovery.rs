use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use protocol::DriverToGui;

use crate::device::{DeviceController, RecoveryTuning};
use crate::hid::ControlState;
use crate::ipc::{EventSubscriber, emit_event};
use crate::outputs::DeviceOutputs;
use crate::soft_off::SoftOffState;

/// Outcome of a recovery attempt.
pub(crate) enum RecoveryResult<D> {
    /// The device came back; the new handle is returned to swap in.
    Recovered(D),
    /// The device did not come back within the escalation budget; the caller
    /// should surface the device as lost and fall back to the re-acquire loop.
    GaveUp,
    /// A shutdown was requested mid-recovery; the caller should stop.
    Shutdown,
}

/// Recover unless the session has exceeded `max_consecutive_recoveries`.
/// `consecutive` counts recoveries since the last successful flush.
#[allow(clippy::too_many_arguments)]
pub(crate) fn recover_or_lost<C: DeviceController>(
    device: C::Device,
    controller: &C,
    outputs: &DeviceOutputs,
    state: &mut ControlState,
    soft_off: &mut SoftOffState,
    subscriber: &EventSubscriber,
    device_present: &AtomicBool,
    shutdown_requested: &AtomicBool,
    tuning: &RecoveryTuning,
    consecutive: &mut u32,
    watchdog_already_reset: bool,
) -> RecoveryResult<C::Device> {
    *consecutive += 1;
    if *consecutive > tuning.max_consecutive_recoveries {
        eprintln!(
            "device wedged across {} recoveries without recovering output; treating as lost",
            *consecutive - 1
        );
        return RecoveryResult::GaveUp;
    }
    recover_device(
        device,
        controller,
        outputs,
        state,
        soft_off,
        subscriber,
        device_present,
        shutdown_requested,
        tuning,
        watchdog_already_reset,
    )
}

/// Reset and reopen with capped backoff. Returns `GaveUp` after
/// `max_reopen_attempts` failed cycles so the outer re-acquire loop handles a
/// real unplug.
#[allow(clippy::too_many_arguments)]
fn recover_device<C: DeviceController>(
    old_device: C::Device,
    controller: &C,
    outputs: &DeviceOutputs,
    state: &mut ControlState,
    soft_off: &mut SoftOffState,
    subscriber: &EventSubscriber,
    device_present: &AtomicBool,
    shutdown_requested: &AtomicBool,
    tuning: &RecoveryTuning,
    watchdog_already_reset: bool,
) -> RecoveryResult<C::Device> {
    // Tell the GUI now so it doesn't show a healthy device during a long recovery.
    device_present.store(false, Ordering::Release);
    emit_event(subscriber, DriverToGui::DeviceConnected(false));

    // Don't hold the old hidraw node open across the USB reset and re-enumeration.
    drop(old_device);

    let mut backoff = tuning.backoff_initial;
    let mut resets_issued = 0;

    // The watchdog's reset may already have cleared the wedge, so try the device
    // before resetting the bus again.
    if watchdog_already_reset {
        if !sleep_interruptible(tuning.reset_settle, shutdown_requested) {
            eprintln!("recovery aborted: shutdown requested");
            return RecoveryResult::Shutdown;
        }
        match controller.open() {
            Ok(new_device) => {
                refresh_after_reopen(state, soft_off, outputs);
                device_present.store(true, Ordering::Release);
                emit_event(subscriber, DriverToGui::DeviceConnected(true));
                eprintln!("device recovered by the write watchdog's reset; no further resets");
                return RecoveryResult::Recovered(new_device);
            }
            Err(err) => eprintln!("reopen after the watchdog's reset failed: {err}"),
        }
    }

    for cycle in 1..=tuning.max_reopen_attempts {
        // `reset_and_reopen` checks for shutdown before each reset.
        match reset_and_reopen(controller, shutdown_requested, tuning, &mut resets_issued) {
            ResetOutcome::Shutdown => {
                eprintln!("recovery aborted: shutdown requested");
                return RecoveryResult::Shutdown;
            }
            ResetOutcome::Opened(new_device) => {
                refresh_after_reopen(state, soft_off, outputs);
                device_present.store(true, Ordering::Release);
                emit_event(subscriber, DriverToGui::DeviceConnected(true));
                eprintln!(
                    "device recovered after {cycle} cycle(s) and {resets_issued} USB reset(s)"
                );
                return RecoveryResult::Recovered(new_device);
            }
            ResetOutcome::StillWedged => {
                eprintln!(
                    "recovery cycle {cycle}/{}: still wedged after {resets_issued} reset(s); \
                     retrying in {} ms",
                    tuning.max_reopen_attempts,
                    backoff.as_millis()
                );
                if !sleep_interruptible(backoff, shutdown_requested) {
                    return RecoveryResult::Shutdown;
                }
                backoff = backoff.saturating_mul(2).min(tuning.backoff_cap);
            }
        }
    }

    eprintln!(
        "device did not reopen after {} attempts; treating as lost",
        tuning.max_reopen_attempts
    );
    RecoveryResult::GaveUp
}

/// Outcome of one reset-and-reopen cycle.
enum ResetOutcome<D> {
    /// The device reopened; `resets_issued` records how many resets it took.
    Opened(D),
    /// The cycle's reset budget was spent without the device reopening.
    StillWedged,
    /// A shutdown was requested partway through.
    Shutdown,
}

/// Issue up to `reset_attempts` USB resets, reopening after each one. This
/// shows how many resets a wedge actually needs and returns the device a settle
/// interval sooner when fewer are enough.
///
/// A successful `open` is a real liveness check, not just enumeration: it
/// applies the startup hardware preferences, which are HID writes on the same
/// OUT endpoint the wedge kills.
fn reset_and_reopen<C: DeviceController>(
    controller: &C,
    shutdown_requested: &AtomicBool,
    tuning: &RecoveryTuning,
    resets_issued: &mut u32,
) -> ResetOutcome<C::Device> {
    for attempt in 1..=tuning.reset_attempts {
        if shutdown_requested.load(Ordering::Relaxed) {
            return ResetOutcome::Shutdown;
        }
        match controller.reset() {
            Ok(()) => eprintln!("USB reset {attempt}/{} issued", tuning.reset_attempts),
            Err(err) => eprintln!(
                "USB reset {attempt}/{} failed: {err}",
                tuning.reset_attempts
            ),
        }
        *resets_issued += 1;
        if !sleep_interruptible(tuning.reset_settle, shutdown_requested) {
            return ResetOutcome::Shutdown;
        }
        match controller.open() {
            Ok(device) => return ResetOutcome::Opened(device),
            Err(err) => eprintln!(
                "reopen after reset {attempt}/{} failed: {err}",
                tuning.reset_attempts
            ),
        }
    }
    ResetOutcome::StillWedged
}

/// Sleep `total`, waking early on shutdown so SIGINT/SIGTERM doesn't wait out a
/// settle or backoff. Returns `false` if a shutdown was observed.
fn sleep_interruptible(total: Duration, shutdown_requested: &AtomicBool) -> bool {
    const SLICE: Duration = Duration::from_millis(50);
    let mut remaining = total;
    while remaining > Duration::ZERO {
        if shutdown_requested.load(Ordering::Relaxed) {
            return false;
        }
        let nap = remaining.min(SLICE);
        thread::sleep(nap);
        remaining = remaining.saturating_sub(nap);
    }
    !shutdown_requested.load(Ordering::Relaxed)
}

/// The reopened device starts fresh, so stale input and soft-off baselines
/// would emit spurious events, and every output needs a repaint.
fn refresh_after_reopen(
    state: &mut ControlState,
    soft_off: &mut SoftOffState,
    outputs: &DeviceOutputs,
) {
    *state = ControlState::new();
    soft_off.reset_input_tracking();
    outputs.mark_all_dirty();
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use hidapi::HidResult;
    use maschine_library::hid::HidIo;

    use super::*;
    use crate::error::{DriverError, DriverResult};
    use crate::soft_off::SoftOffSync;

    struct NullDevice;

    impl HidIo for NullDevice {
        fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            Ok(0)
        }
        fn write(&self, data: &[u8]) -> HidResult<usize> {
            Ok(data.len())
        }
        fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
            Ok(())
        }
    }

    /// Hands out scripted `open` outcomes (true = succeed, false = fail); once the
    /// script is exhausted every further `open` uses `default_open_ok`.
    struct FakeController {
        opens: Mutex<VecDeque<bool>>,
        default_open_ok: bool,
        open_calls: AtomicUsize,
        reset_calls: AtomicUsize,
    }

    impl FakeController {
        fn new(opens: Vec<bool>, default_open_ok: bool) -> Self {
            Self {
                opens: Mutex::new(opens.into()),
                default_open_ok,
                open_calls: AtomicUsize::new(0),
                reset_calls: AtomicUsize::new(0),
            }
        }
    }

    impl DeviceController for FakeController {
        type Device = NullDevice;

        fn open(&self) -> DriverResult<NullDevice> {
            self.open_calls.fetch_add(1, Ordering::SeqCst);
            let ok = self
                .opens
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(self.default_open_ok);
            if ok {
                Ok(NullDevice)
            } else {
                Err(DriverError::Settings("fake open failure".into()))
            }
        }

        fn reset(&self) -> DriverResult<()> {
            self.reset_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn tuning() -> RecoveryTuning {
        RecoveryTuning {
            enabled: true,
            quick_retries: 0,
            quick_retry_delay: Duration::ZERO,
            reset_attempts: 2,
            reset_settle: Duration::ZERO,
            backoff_initial: Duration::ZERO,
            backoff_cap: Duration::ZERO,
            max_reopen_attempts: 3,
            max_consecutive_recoveries: 2,
            write_stall_timeout: Duration::from_millis(500),
            write_stall_poll: Duration::from_millis(100),
        }
    }

    struct Harness {
        outputs: DeviceOutputs,
        state: ControlState,
        soft_off: SoftOffState,
        subscriber: EventSubscriber,
        present: AtomicBool,
        shutdown: AtomicBool,
    }

    impl Harness {
        fn new() -> Self {
            Self {
                outputs: DeviceOutputs::new(),
                state: ControlState::new(),
                soft_off: SoftOffState::new(SoftOffSync::new()),
                subscriber: crate::ipc::new_subscriber(),
                present: AtomicBool::new(true),
                shutdown: AtomicBool::new(false),
            }
        }

        fn recover(
            &mut self,
            controller: &FakeController,
            tuning: &RecoveryTuning,
            consecutive: &mut u32,
        ) -> RecoveryResult<NullDevice> {
            self.recover_after_watchdog(controller, tuning, consecutive, false)
        }

        fn recover_after_watchdog(
            &mut self,
            controller: &FakeController,
            tuning: &RecoveryTuning,
            consecutive: &mut u32,
            watchdog_already_reset: bool,
        ) -> RecoveryResult<NullDevice> {
            recover_or_lost(
                NullDevice,
                controller,
                &self.outputs,
                &mut self.state,
                &mut self.soft_off,
                &self.subscriber,
                &self.present,
                &self.shutdown,
                tuning,
                consecutive,
                watchdog_already_reset,
            )
        }
    }

    #[test]
    fn refresh_after_reopen_marks_outputs_dirty() {
        let outputs = DeviceOutputs::new();
        let mut state = ControlState::new();
        let mut soft_off = SoftOffState::new(SoftOffSync::new());
        assert!(!outputs.lights_dirty());
        assert!(!outputs.screen_dirty());

        refresh_after_reopen(&mut state, &mut soft_off, &outputs);

        assert!(outputs.lights_dirty());
        assert!(outputs.screen_dirty());
    }

    #[test]
    fn sleep_interruptible_returns_false_when_shutdown_already_set() {
        let shutdown = AtomicBool::new(true);
        assert!(!sleep_interruptible(Duration::from_secs(10), &shutdown));
    }

    #[test]
    fn sleep_interruptible_completes_zero_duration() {
        let shutdown = AtomicBool::new(false);
        assert!(sleep_interruptible(Duration::ZERO, &shutdown));
    }

    #[test]
    fn stops_at_the_first_reset_that_reopens_the_device() {
        let controller = FakeController::new(vec![true], true);
        let mut h = Harness::new();
        let mut consecutive = 0;

        let result = h.recover(&controller, &tuning(), &mut consecutive);

        assert!(matches!(result, RecoveryResult::Recovered(_)));
        assert_eq!(
            controller.reset_calls.load(Ordering::SeqCst),
            1,
            "a wedge that clears on the first reset must not be reset again"
        );
        assert_eq!(controller.open_calls.load(Ordering::SeqCst), 1);
        assert!(h.present.load(Ordering::SeqCst));
    }

    #[test]
    fn skips_the_first_reset_when_the_watchdog_already_reset_the_device() {
        let controller = FakeController::new(vec![true], true);
        let mut h = Harness::new();
        let mut consecutive = 0;

        let result = h.recover_after_watchdog(&controller, &tuning(), &mut consecutive, true);

        assert!(matches!(result, RecoveryResult::Recovered(_)));
        assert_eq!(
            controller.reset_calls.load(Ordering::SeqCst),
            0,
            "the watchdog's reset already cleared the wedge; recovery must not reset again"
        );
        assert_eq!(controller.open_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn still_resets_when_the_watchdog_reset_did_not_clear_the_wedge() {
        let controller = FakeController::new(vec![false, true], true);
        let mut h = Harness::new();
        let mut consecutive = 0;

        let result = h.recover_after_watchdog(&controller, &tuning(), &mut consecutive, true);

        assert!(matches!(result, RecoveryResult::Recovered(_)));
        assert_eq!(
            controller.reset_calls.load(Ordering::SeqCst),
            1,
            "the post-watchdog reopen failed, so recovery falls back to its own reset"
        );
        assert_eq!(controller.open_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn escalates_to_a_second_reset_when_the_first_does_not_reopen() {
        let controller = FakeController::new(vec![false, true], true);
        let mut h = Harness::new();
        let mut consecutive = 0;

        let result = h.recover(&controller, &tuning(), &mut consecutive);

        assert!(matches!(result, RecoveryResult::Recovered(_)));
        assert_eq!(controller.reset_calls.load(Ordering::SeqCst), 2);
        assert_eq!(controller.open_calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn gives_up_after_max_reopen_attempts_when_device_never_returns() {
        let controller = FakeController::new(vec![], false);
        let mut h = Harness::new();
        let mut consecutive = 0;
        let tuning = tuning();

        let result = h.recover(&controller, &tuning, &mut consecutive);

        assert!(matches!(result, RecoveryResult::GaveUp));
        let budget = (tuning.max_reopen_attempts * tuning.reset_attempts) as usize;
        assert_eq!(
            controller.open_calls.load(Ordering::SeqCst),
            budget,
            "every reset in the budget gets its own reopen attempt"
        );
        assert_eq!(controller.reset_calls.load(Ordering::SeqCst), budget);
        assert!(
            !h.present.load(Ordering::SeqCst),
            "presence stays false once the device is given up as lost"
        );
    }

    #[test]
    fn gives_up_without_resetting_once_consecutive_budget_is_exhausted() {
        let controller = FakeController::new(vec![true], true);
        let mut h = Harness::new();
        let t = tuning();
        // Already at the budget: the next recovery exceeds it.
        let mut consecutive = t.max_consecutive_recoveries;

        let result = h.recover(&controller, &t, &mut consecutive);

        assert!(matches!(result, RecoveryResult::GaveUp));
        assert_eq!(
            controller.open_calls.load(Ordering::SeqCst),
            0,
            "a device past the recovery budget is surfaced as lost without another reset"
        );
        assert_eq!(controller.reset_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn aborts_with_shutdown_when_requested() {
        let controller = FakeController::new(vec![], false);
        let mut h = Harness::new();
        h.shutdown.store(true, Ordering::SeqCst);
        let mut consecutive = 0;

        let result = h.recover(&controller, &tuning(), &mut consecutive);

        assert!(matches!(result, RecoveryResult::Shutdown));
    }
}
