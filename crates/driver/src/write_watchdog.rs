use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use crate::device::ResetAction;

/// Sentinel for "no write in flight". Timestamps are milliseconds since the
/// watchdog's origin, so a real one never reaches this.
const IDLE: u64 = u64::MAX;

/// Watches the in-flight HID write and resets the device when one stops
/// completing.
///
/// hidraw writes block in `usb_interrupt_msg` for up to 5 s on a wedged OUT
/// endpoint, and the blocked runtime thread cannot notice its own stall.
///
/// The reset also completes the pending URB, so the blocked write returns at
/// once. Recovery then skips its own first reset.
pub struct WriteWatchdog {
    origin: Instant,
    started_at_ms: AtomicU64,
    stall_threshold_ms: u64,
    /// Set once a reset has been issued for the current stall. Stops the
    /// watchdog resetting again every poll, and tells recovery the device has
    /// already been reset.
    reset_issued: AtomicBool,
    /// Cleared while recovery owns the device, so the watchdog cannot reset out
    /// from under a reopen in progress.
    armed: AtomicBool,
    stopped: AtomicBool,
}

impl WriteWatchdog {
    pub fn new(stall_threshold: Duration) -> Self {
        Self {
            origin: Instant::now(),
            started_at_ms: AtomicU64::new(IDLE),
            stall_threshold_ms: stall_threshold.as_millis() as u64,
            reset_issued: AtomicBool::new(false),
            armed: AtomicBool::new(true),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }

    pub fn begin_write(&self, now_ms: u64) {
        self.started_at_ms.store(now_ms, Ordering::Release);
    }

    pub fn end_write(&self) {
        self.started_at_ms.store(IDLE, Ordering::Release);
    }

    /// How long the current write has been stalled, if a reset is due. Pure in
    /// `now_ms` so the escalation rule is testable without sleeping.
    pub fn stall_due(&self, now_ms: u64) -> Option<u64> {
        if !self.armed.load(Ordering::Acquire) || self.reset_issued.load(Ordering::Acquire) {
            return None;
        }
        let started = self.started_at_ms.load(Ordering::Acquire);
        if started == IDLE {
            return None;
        }
        let stalled_for = now_ms.saturating_sub(started);
        (stalled_for >= self.stall_threshold_ms).then_some(stalled_for)
    }

    pub fn mark_reset_issued(&self) {
        self.reset_issued.store(true, Ordering::Release);
    }

    /// Consume the "already reset" flag, so recovery acts on it exactly once.
    pub fn take_reset_issued(&self) -> bool {
        self.reset_issued.swap(false, Ordering::AcqRel)
    }

    /// Suspend watching while recovery owns the device. Clears the in-flight
    /// write too: the write that stalled has returned by now, and recovery is
    /// about to drop the handle it belonged to.
    pub fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
        self.end_write();
    }

    pub fn arm(&self) {
        self.armed.store(true, Ordering::Release);
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

/// Run the watchdog for the lifetime of `scope`. Scoped rather than detached so
/// it cannot outlive the session whose writes it watches.
pub fn spawn<'scope, 'env>(
    scope: &'scope Scope<'scope, 'env>,
    watchdog: &'env WriteWatchdog,
    reset: ResetAction,
    poll_interval: Duration,
) -> ScopedJoinHandle<'scope, ()> {
    scope.spawn(move || {
        while !watchdog.stopped() {
            std::thread::sleep(poll_interval);
            let Some(stalled_for) = watchdog.stall_due(watchdog.now_ms()) else {
                continue;
            };
            eprintln!(
                "HID write stalled {stalled_for} ms; resetting the device rather than \
                 waiting out the kernel write timeout"
            );
            // Marked before the outcome is known: a reset that failed must not
            // be retried every poll interval, and recovery still needs to know
            // the device was disturbed.
            watchdog.mark_reset_issued();
            if let Err(err) = reset() {
                eprintln!("watchdog USB reset failed: {err}");
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watchdog() -> WriteWatchdog {
        WriteWatchdog::new(Duration::from_millis(500))
    }

    #[test]
    fn no_reset_is_due_while_no_write_is_in_flight() {
        let w = watchdog();
        assert_eq!(w.stall_due(10_000), None);
    }

    #[test]
    fn no_reset_is_due_before_the_threshold_elapses() {
        let w = watchdog();
        w.begin_write(1_000);

        assert_eq!(w.stall_due(1_499), None);
        assert_eq!(w.stall_due(1_500), Some(500));
    }

    #[test]
    fn a_completed_write_clears_the_pending_stall() {
        let w = watchdog();
        w.begin_write(1_000);
        w.end_write();

        assert_eq!(w.stall_due(9_000), None);
    }

    #[test]
    fn only_one_reset_is_issued_per_stall() {
        let w = watchdog();
        w.begin_write(1_000);
        assert!(w.stall_due(2_000).is_some());

        w.mark_reset_issued();

        assert_eq!(
            w.stall_due(9_000),
            None,
            "a reset already issued for this stall must not be repeated every poll"
        );
    }

    #[test]
    fn recovery_consumes_the_reset_flag_exactly_once() {
        let w = watchdog();
        w.mark_reset_issued();

        assert!(w.take_reset_issued());
        assert!(!w.take_reset_issued());
    }

    #[test]
    fn a_disarmed_watchdog_never_resets_underneath_recovery() {
        let w = watchdog();
        w.begin_write(1_000);
        w.disarm();

        assert_eq!(w.stall_due(9_000), None);

        w.arm();
        assert_eq!(
            w.stall_due(9_000),
            None,
            "rearming must not resurrect the write recovery already dealt with"
        );
    }
}
