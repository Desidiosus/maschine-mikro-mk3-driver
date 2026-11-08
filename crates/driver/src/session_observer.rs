use std::time::Duration;

use hidapi::HidResult;

/// Hooks into the device session for optional diagnostics. Every method
/// defaults to a no-op, so the session loop carries no diagnostic logic itself.
pub trait SessionObserver {
    /// A fault is about to send the session into recovery.
    fn fault(&self, _reason: &str) {}

    /// A main-loop flush is starting; `will_write` is false when nothing is dirty.
    fn flush_started(&self, _will_write: bool) {}

    fn flush_finished(&self, _result: &HidResult<()>) {}

    fn recovery_finished(&self, _took: Duration, _recovered: bool) {}
}

pub struct NoopObserver;

impl SessionObserver for NoopObserver {}
