use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use hidapi::{HidError, HidResult};
use maschine_library::hid::HidIo;

use driver::device::{DeviceController, RecoveryTuning};
use driver::error::DriverResult;

fn test_settings() -> driver::settings::Settings {
    let mut s = driver::settings::Settings::default();
    s.global.client_name = "Client".into();
    s.global.port_name = "Port".into();
    s.global.port_name_in = "Input".into();
    s.bridge.autoconnect_virmidi = false;
    s
}

fn fast_tuning() -> RecoveryTuning {
    RecoveryTuning {
        enabled: true,
        quick_retries: 3,
        quick_retry_delay: Duration::from_millis(0),
        reset_attempts: 2,
        reset_settle: Duration::from_millis(0),
        backoff_initial: Duration::from_millis(0),
        backoff_cap: Duration::from_millis(0),
        max_reopen_attempts: 5,
        max_consecutive_recoveries: 4,
        write_stall_timeout: Duration::from_millis(500),
        write_stall_poll: Duration::from_millis(100),
    }
}

/// The MIDI backend opens ALSA sequencer ports, so these tests need `/dev/snd/seq`.
/// The skip is logged so a missing sequencer does not look like a pass.
fn alsa_seq_available() -> bool {
    let available = Path::new("/dev/snd/seq").exists();
    if !available {
        eprintln!(
            "SKIP: /dev/snd/seq missing; ALSA sequencer unavailable, recovery integration test skipped"
        );
    }
    available
}

fn timed_out() -> HidError {
    HidError::HidApiError {
        message: "Connection timed out".to_string(),
    }
}

/// One device handed out by an `open()` call. Reads are scripted; once the
/// script is exhausted the device is idle and requests shutdown, which ends
/// the session as soon as a recovered device is back in the main loop.
struct FakeDevice {
    reads: Mutex<VecDeque<HidResult<usize>>>,
    shutdown: Arc<AtomicBool>,
}

impl HidIo for FakeDevice {
    fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
        self.reads.lock().unwrap().pop_front().unwrap_or_else(|| {
            self.shutdown.store(true, Ordering::Relaxed);
            Ok(0)
        })
    }

    fn write(&self, data: &[u8]) -> HidResult<usize> {
        Ok(data.len())
    }

    fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
        Ok(())
    }
}

enum OpenOutcome {
    Fail,
    Device(Vec<HidResult<usize>>),
}

struct FakeController {
    outcomes: Mutex<VecDeque<OpenOutcome>>,
    open_calls: Arc<AtomicUsize>,
    reset_calls: Arc<AtomicUsize>,
    /// When the scripted outcomes are exhausted: if true, fail every further
    /// `open` (device never returns); if false, hand out an idle device.
    default_open_fails: bool,
    shutdown: Arc<AtomicBool>,
    shutdown_after_resets: Option<usize>,
}

impl FakeController {
    fn new(outcomes: Vec<OpenOutcome>, default_open_fails: bool) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into()),
            open_calls: Arc::new(AtomicUsize::new(0)),
            reset_calls: Arc::new(AtomicUsize::new(0)),
            default_open_fails,
            shutdown: Arc::new(AtomicBool::new(false)),
            shutdown_after_resets: None,
        }
    }

    fn device(&self, reads: Vec<HidResult<usize>>) -> FakeDevice {
        FakeDevice {
            reads: Mutex::new(reads.into()),
            shutdown: self.shutdown.clone(),
        }
    }

    fn run(&self, tuning: &RecoveryTuning) -> DriverResult<()> {
        driver::app::run_with_controller(test_settings(), self, &self.shutdown, tuning)
    }
}

impl DeviceController for FakeController {
    type Device = FakeDevice;

    fn open(&self) -> DriverResult<Self::Device> {
        self.open_calls.fetch_add(1, Ordering::SeqCst);
        let outcome = self.outcomes.lock().unwrap().pop_front();
        match outcome {
            Some(OpenOutcome::Fail) => Err(driver::error::DriverError::Settings(
                "fake open failure".into(),
            )),
            Some(OpenOutcome::Device(reads)) => Ok(self.device(reads)),
            None if self.default_open_fails => Err(driver::error::DriverError::Settings(
                "fake open failure".into(),
            )),
            None => Ok(self.device(vec![])),
        }
    }

    fn reset(&self) -> DriverResult<()> {
        let resets = self.reset_calls.fetch_add(1, Ordering::SeqCst) + 1;
        if self.shutdown_after_resets == Some(resets) {
            self.shutdown.store(true, Ordering::Relaxed);
        }
        Ok(())
    }
}

#[test]
fn wedged_device_recovers_via_reset_and_reopen() {
    if !alsa_seq_available() {
        return;
    }

    // Four timeouts exceed quick_retries (3) and trigger recovery; the reopened
    // device stays idle until shutdown.
    let controller = FakeController::new(
        vec![
            OpenOutcome::Device(vec![
                Err(timed_out()),
                Err(timed_out()),
                Err(timed_out()),
                Err(timed_out()),
            ]),
            OpenOutcome::Device(vec![]),
        ],
        false,
    );
    let reset_calls = controller.reset_calls.clone();
    let open_calls = controller.open_calls.clone();

    controller
        .run(&fast_tuning())
        .expect("loop should exit cleanly");

    assert_eq!(
        reset_calls.load(Ordering::SeqCst),
        1,
        "the reopen after the first reset succeeds, so the second is never issued"
    );
    assert_eq!(
        open_calls.load(Ordering::SeqCst),
        2,
        "one initial open + one reopen"
    );
}

#[test]
fn reopen_retries_until_device_returns() {
    if !alsa_seq_available() {
        return;
    }

    // Device #1 wedges; first two reopen attempts fail, third succeeds.
    let controller = FakeController::new(
        vec![
            OpenOutcome::Device(vec![
                Err(timed_out()),
                Err(timed_out()),
                Err(timed_out()),
                Err(timed_out()),
            ]),
            OpenOutcome::Fail,
            OpenOutcome::Fail,
            OpenOutcome::Device(vec![]),
        ],
        false,
    );
    let reset_calls = controller.reset_calls.clone();
    let open_calls = controller.open_calls.clone();

    controller
        .run(&fast_tuning())
        .expect("loop should exit cleanly");

    // Initial open + 3 reopens (2 fail, 1 succeeds).
    assert_eq!(open_calls.load(Ordering::SeqCst), 4);
    // Cycle 1 spends both resets on the failing reopens; cycle 2 succeeds on its first.
    assert_eq!(reset_calls.load(Ordering::SeqCst), 3);
}

#[test]
fn shutdown_during_escalation_exits_cleanly() {
    if !alsa_seq_available() {
        return;
    }

    // Every reopen fails and the budgets are unbounded, so only a shutdown request
    // can end escalation, and it must do so promptly.
    let escalation_tuning = RecoveryTuning {
        enabled: true,
        quick_retries: 3,
        quick_retry_delay: Duration::from_millis(0),
        reset_attempts: 2,
        reset_settle: Duration::from_millis(1),
        backoff_initial: Duration::from_millis(1),
        backoff_cap: Duration::from_millis(5),
        max_reopen_attempts: u32::MAX,
        max_consecutive_recoveries: u32::MAX,
        write_stall_timeout: Duration::from_millis(500),
        write_stall_poll: Duration::from_millis(100),
    };
    let mut controller = FakeController::new(
        vec![OpenOutcome::Device(vec![
            Err(timed_out()),
            Err(timed_out()),
            Err(timed_out()),
            Err(timed_out()),
        ])],
        true,
    );
    // Several cycles in, so the request lands mid-escalation.
    controller.shutdown_after_resets = Some(5);
    let reset_calls = controller.reset_calls.clone();

    controller
        .run(&escalation_tuning)
        .expect("loop should exit cleanly");

    assert_eq!(
        reset_calls.load(Ordering::SeqCst),
        5,
        "no reset may be issued once shutdown is requested"
    );
}

#[test]
fn escalation_gives_up_and_reports_lost_when_device_never_returns() {
    if !alsa_seq_available() {
        return;
    }

    // Every reopen fails, so the finite reopen budget makes the session give up
    // and return without a shutdown request.
    let controller = FakeController::new(
        vec![OpenOutcome::Device(vec![
            Err(timed_out()),
            Err(timed_out()),
            Err(timed_out()),
            Err(timed_out()),
        ])],
        true,
    );
    let reset_calls = controller.reset_calls.clone();
    let open_calls = controller.open_calls.clone();

    controller
        .run(&fast_tuning())
        .expect("loop should exit cleanly");

    // Initial open plus one failed reopen per reset:
    // `max_reopen_attempts` (5) cycles x `reset_attempts` (2).
    assert_eq!(open_calls.load(Ordering::SeqCst), 1 + 5 * 2);
    assert_eq!(reset_calls.load(Ordering::SeqCst), 5 * 2);
}

#[test]
fn disabled_recovery_ends_the_session_without_a_reset() {
    if !alsa_seq_available() {
        return;
    }

    let controller = FakeController::new(
        vec![OpenOutcome::Device(vec![
            Err(timed_out()),
            Err(timed_out()),
            Err(timed_out()),
            Err(timed_out()),
        ])],
        false,
    );
    let tuning = RecoveryTuning {
        enabled: false,
        ..fast_tuning()
    };

    controller.run(&tuning).expect("loop should exit cleanly");

    assert_eq!(controller.reset_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        controller.open_calls.load(Ordering::SeqCst),
        1,
        "the session hands the device back to the re-acquire loop instead of reopening it"
    );
}

/// Reads emitted by a wedged device while writes (LED/screen flushes) fail.
#[derive(Clone, Copy)]
enum ReadMode {
    /// Every read returns a pad-on packet (mimics sustained input/DAW feedback).
    AlwaysInput,
    /// One pad-on packet, then idle `Ok(0)` forever, so only a retried flush can fail.
    InputThenIdle,
}

fn pad_on_packet(buf: &mut [u8]) {
    buf[0] = 0x02;
    buf[1] = 0;
    buf[2] = 0x10;
    buf[3] = 64;
}

/// Fails every write once the runtime loop starts (detected by the first
/// `read_timeout`) while reads keep succeeding, matching the firmware wedge
/// where only the OUT endpoint times out.
struct WedgeDevice {
    started: Arc<AtomicBool>,
    read_count: Arc<AtomicUsize>,
    mode: ReadMode,
}

impl HidIo for WedgeDevice {
    fn read_timeout(&self, buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
        self.started.store(true, Ordering::SeqCst);
        let n = self.read_count.fetch_add(1, Ordering::SeqCst);
        let emit = match self.mode {
            ReadMode::AlwaysInput => true,
            ReadMode::InputThenIdle => n == 0,
        };
        if emit {
            pad_on_packet(buf);
            Ok(64)
        } else {
            Ok(0)
        }
    }

    fn write(&self, data: &[u8]) -> HidResult<usize> {
        if self.started.load(Ordering::SeqCst) {
            Err(timed_out())
        } else {
            Ok(data.len())
        }
    }

    fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
        Ok(())
    }
}

struct WedgeController {
    started: Arc<AtomicBool>,
    read_count: Arc<AtomicUsize>,
    mode: ReadMode,
    open_calls: Arc<AtomicUsize>,
    reset_calls: Arc<AtomicUsize>,
}

impl WedgeController {
    fn new(mode: ReadMode) -> Self {
        Self {
            started: Arc::new(AtomicBool::new(false)),
            read_count: Arc::new(AtomicUsize::new(0)),
            mode,
            open_calls: Arc::new(AtomicUsize::new(0)),
            reset_calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl DeviceController for WedgeController {
    type Device = WedgeDevice;

    fn open(&self) -> DriverResult<Self::Device> {
        self.open_calls.fetch_add(1, Ordering::SeqCst);
        Ok(WedgeDevice {
            started: self.started.clone(),
            read_count: self.read_count.clone(),
            mode: self.mode,
        })
    }

    fn reset(&self) -> DriverResult<()> {
        self.reset_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

/// Runs until the recovery budget is spent; the session gives up on its own.
fn run_wedge_until_give_up(mode: ReadMode) -> usize {
    let controller = WedgeController::new(mode);
    let shutdown = AtomicBool::new(false);

    driver::app::run_with_controller(test_settings(), &controller, &shutdown, &fast_tuning())
        .expect("loop should exit cleanly");

    controller.reset_calls.load(Ordering::SeqCst)
}

#[test]
fn failing_writes_trigger_recovery_despite_successful_reads() {
    if !alsa_seq_available() {
        return;
    }

    // A failure counter that resets on every successful read would never escalate
    // here. The reopened device stays wedged, so recovery runs
    // `max_consecutive_recoveries` (4) times, one reset each since every reopen succeeds.
    let resets = run_wedge_until_give_up(ReadMode::AlwaysInput);
    let budgeted = fast_tuning().max_consecutive_recoveries as usize;
    assert_eq!(
        resets, budgeted,
        "a device that reopens but stays wedged should recover the budgeted number of times then give up, got {resets}"
    );
}

#[test]
fn single_dirty_then_idle_still_triggers_recovery() {
    if !alsa_seq_available() {
        return;
    }

    // One input marks the LEDs dirty, then reads go idle, so recovery only fires if
    // the failed flush is retried. It runs `max_consecutive_recoveries` (4) times,
    // one reset each.
    let resets = run_wedge_until_give_up(ReadMode::InputThenIdle);
    let budgeted = fast_tuning().max_consecutive_recoveries as usize;
    assert_eq!(
        resets, budgeted,
        "a retried dirty flush against a still-wedged device should recover the budgeted number of times then give up, got {resets}"
    );
}
