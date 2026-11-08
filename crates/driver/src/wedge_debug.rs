//! `driver --debug-wedge`: diagnostics for the replug-only device wedge.
//!
//! The leading explanation is a read-modify-write race on `USB_EP1R`, the
//! register shared by endpoint `0x81` IN and endpoint `0x01` OUT, which
//! predicts that IN keeps working while OUT is dead. `InputMonitor` measures IN
//! on its own handle and thread, because a wedged write blocks the writing
//! thread for exactly the window that matters.

use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use hidapi::{HidApi, HidDevice, HidResult};
use maschine_library::{USB_PID, USB_VID};

use crate::app;
use crate::error::DriverResult;
use crate::events::ControlEvent;
use crate::hid::{ControlState, decode_packet};
use crate::session_observer::SessionObserver;
use crate::settings::LoadedConfig;

/// Write latency above which a frame is counted as stalled. Well clear of the
/// ~20 ms floor, so ordinary endpoint backpressure is not counted as a stall.
const STALL_THRESHOLD: Duration = Duration::from_millis(100);

const SUMMARY_INTERVAL: Duration = Duration::from_secs(60);

/// Pause before reopening the monitor's handle, so an absent device or one
/// mid re-enumeration is polled rather than spun on.
const REOPEN_DELAY: Duration = Duration::from_millis(250);

/// Latencies past this land in the last bucket. The kernel's 5 s HID write
/// timeout bounds real latencies anyway.
const MAX_BUCKET_MS: usize = 5_000;

const LOG_PREFIX: &str = "[wedge-debug]";

/// Write latencies bucketed per millisecond, so a driver left running for
/// hours keeps a fixed footprint.
struct WriteStats {
    buckets: Vec<u64>,
    writes: u64,
    stalls: u64,
    max: Duration,
}

impl WriteStats {
    fn new() -> Self {
        Self {
            buckets: vec![0; MAX_BUCKET_MS + 1],
            writes: 0,
            stalls: 0,
            max: Duration::ZERO,
        }
    }

    fn record(&mut self, latency: Duration) {
        let bucket = (latency.as_millis() as usize).min(MAX_BUCKET_MS);
        self.buckets[bucket] += 1;
        self.writes += 1;
        if latency >= STALL_THRESHOLD {
            self.stalls += 1;
        }
        self.max = self.max.max(latency);
    }

    fn writes(&self) -> u64 {
        self.writes
    }

    fn stalls(&self) -> u64 {
        self.stalls
    }

    /// Latency at `fraction` of recorded writes, at millisecond resolution.
    fn percentile(&self, fraction: f64) -> Duration {
        if self.writes == 0 {
            return Duration::ZERO;
        }
        let rank = ((self.writes as f64 * fraction).ceil() as u64).max(1);
        let mut seen = 0;
        for (ms, &count) in self.buckets.iter().enumerate() {
            seen += count;
            if seen >= rank {
                return Duration::from_millis(ms as u64);
            }
        }
        self.max
    }

    fn latency_line(&self) -> String {
        format!(
            "p50 {:?}  p99 {:?}  max {:?}",
            self.percentile(0.50),
            self.percentile(0.99),
            self.max
        )
    }
}

impl Default for WriteStats {
    fn default() -> Self {
        Self::new()
    }
}

/// Reports new kernel log lines around a wedge. Every candidate explanation
/// predicts no USB event there, so a spontaneous disconnect would rule them all out.
struct DmesgWatch {
    /// `None` when dmesg is unreadable (kernel.dmesg_restrict). Otherwise the
    /// last line already reported, keyed by content rather than count because
    /// the kernel ring buffer wraps during a long driver run.
    last_seen: Mutex<Option<Option<String>>>,
}

impl DmesgWatch {
    fn start() -> Self {
        let last_seen = match Self::read() {
            Some(output) => Some(output.lines().last().map(str::to_string)),
            None => {
                eprintln!(
                    "{LOG_PREFIX} cannot read dmesg (kernel.dmesg_restrict?); \
                     the kernel-log discriminator will be unavailable"
                );
                None
            }
        };
        Self {
            last_seen: Mutex::new(last_seen),
        }
    }

    fn read() -> Option<String> {
        let output = Command::new("dmesg").output().ok()?;
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Print kernel log lines that appeared since the previous call.
    fn print_new_lines(&self, label: &str) {
        let mut last_seen = self.last_seen.lock().unwrap();
        let Some(last_line) = last_seen.as_mut() else {
            println!("{LOG_PREFIX} kernel log: unavailable");
            return;
        };
        let Some(current) = Self::read() else {
            println!("{LOG_PREFIX} kernel log: became unreadable mid-run");
            return;
        };
        let new = lines_after(&current, last_line.as_deref());
        if new.is_empty() {
            println!("{LOG_PREFIX} kernel log {label}: no new lines — the device never detached");
        } else {
            println!(
                "{LOG_PREFIX} kernel log {label}: {} new line(s):",
                new.len()
            );
            for line in &new {
                println!("{LOG_PREFIX}   {line}");
            }
        }
        if let Some(last) = new.last() {
            *last_line = Some((*last).to_string());
        }
    }
}

/// Lines of `log` after the last occurrence of `last_seen`. If that line has
/// rotated out of the ring buffer, everything still in it is new.
fn lines_after<'a>(log: &'a str, last_seen: Option<&str>) -> Vec<&'a str> {
    let lines: Vec<&str> = log.lines().collect();
    let start = last_seen
        .and_then(|seen| lines.iter().rposition(|line| *line == seen))
        .map_or(0, |index| index + 1);
    lines[start..].to_vec()
}

/// Counts input reports on a hidraw handle of its own, from a thread of its own.
///
/// hidraw delivers every input report to all open handles, so this observes
/// the IN direction independently of whatever the writing thread is blocked on.
struct InputMonitor {
    origin: Instant,
    reports: AtomicU64,
    /// Millis since `origin` of the most recent report; 0 = none yet.
    last_report_ms: AtomicU64,
    pad_strikes: AtomicU64,
    stop: AtomicBool,
}

/// The IN direction as a write goes out. If that write wedges, the difference
/// against these values shows whether IN outlived OUT.
#[derive(Clone, Copy)]
struct WriteSnapshot {
    reports_before: u64,
    submitted_ms: u64,
}

impl InputMonitor {
    fn new() -> Self {
        Self {
            origin: Instant::now(),
            reports: AtomicU64::new(0),
            last_report_ms: AtomicU64::new(0),
            pad_strikes: AtomicU64::new(0),
            stop: AtomicBool::new(false),
        }
    }

    fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }

    fn reports(&self) -> u64 {
        self.reports.load(Ordering::Acquire)
    }

    fn pad_strikes(&self) -> u64 {
        self.pad_strikes.load(Ordering::Acquire)
    }

    fn snapshot(&self) -> WriteSnapshot {
        WriteSnapshot {
            reports_before: self.reports(),
            submitted_ms: self.now_ms(),
        }
    }

    fn survival_since(&self, snapshot: WriteSnapshot) -> InSurvival {
        InSurvival::measure(
            snapshot,
            self.reports(),
            self.last_report_ms.load(Ordering::Acquire),
            self.pad_strikes(),
        )
    }

    fn record(&self, pad_strikes: u64) {
        self.last_report_ms.store(self.now_ms(), Ordering::Release);
        self.reports.fetch_add(1, Ordering::AcqRel);
        self.pad_strikes.fetch_add(pad_strikes, Ordering::AcqRel);
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    /// Read and decode input reports until stopped. The handle is reopened
    /// whenever it fails, because a USB reset re-enumerates the device and
    /// kills the old hidraw node.
    fn run(&self) {
        let mut api = match HidApi::new() {
            Ok(api) => api,
            Err(err) => {
                eprintln!("{LOG_PREFIX} input monitor disabled: {err}");
                return;
            }
        };
        let mut buf = [0u8; 64];

        while !self.stopped() {
            let Ok(device) = open_reader(&mut api) else {
                thread::sleep(REOPEN_DELAY);
                continue;
            };
            let mut state = ControlState::new();

            while !self.stopped() {
                buf.fill(0);
                match device.read_timeout(&mut buf, 1) {
                    Ok(0) => {}
                    Ok(_) => {
                        let events = decode_packet(&mut state, &buf);
                        let strikes = events
                            .iter()
                            .filter(|event| matches!(event, ControlEvent::PadNoteOn { .. }))
                            .count() as u64;
                        self.record(strikes);
                    }
                    Err(_) => {
                        thread::sleep(REOPEN_DELAY);
                        break;
                    }
                }
            }
        }
    }
}

impl Default for InputMonitor {
    fn default() -> Self {
        Self::new()
    }
}

fn open_reader(api: &mut HidApi) -> HidResult<HidDevice> {
    api.refresh_devices()?;
    let device = api.open(USB_VID, USB_PID)?;
    device.set_blocking_mode(false)?;
    Ok(device)
}

/// How the IN direction behaved across a wedge, measured while it happened
/// rather than probed afterwards.
#[derive(Debug, PartialEq, Eq)]
struct InSurvival {
    reports_after_stall: u64,
    outlived_out_by: Option<Duration>,
    pad_strikes_during_run: u64,
}

impl InSurvival {
    fn measure(
        snapshot: WriteSnapshot,
        reports_now: u64,
        last_report_ms: u64,
        pad_strikes: u64,
    ) -> Self {
        Self {
            reports_after_stall: reports_now - snapshot.reports_before,
            outlived_out_by: (last_report_ms > snapshot.submitted_ms)
                .then(|| Duration::from_millis(last_report_ms - snapshot.submitted_ms)),
            pad_strikes_during_run: pad_strikes,
        }
    }

    fn print(&self) {
        println!(
            "{LOG_PREFIX} input reports after the failing write was submitted: {}",
            self.reports_after_stall
        );
        match self.outlived_out_by {
            Some(lag) => println!("{LOG_PREFIX} IN outlived OUT by {} ms", lag.as_millis()),
            None => println!("{LOG_PREFIX} IN delivered nothing once the write stalled"),
        }
        if !self.is_informative() {
            println!(
                "{LOG_PREFIX} NOTE: no pads were struck this run, so the IN measurement \
                 says nothing about the mechanism."
            );
        }
    }

    /// Only meaningful if pads were actually being struck — the IN endpoint is
    /// silent on an untouched device whether or not it is wedged.
    fn is_informative(&self) -> bool {
        self.pad_strikes_during_run > 0
    }

    /// Which explanation the IN measurement alone supports. It cannot tell USB
    /// suspend from an MCU reset; that needs someone to see whether the startup
    /// LED animation replayed at the freeze.
    fn reading(&self) -> &'static str {
        if !self.is_informative() {
            "inconclusive — no pad strikes yet, so the IN direction was never \
             exercised. Drum continuously while reproducing."
        } else if self.reports_after_stall > 0 {
            "EP1R race: IN kept working while OUT was dead, which neither a USB \
             suspend nor an MCU watchdog reset can produce."
        } else {
            "both directions dead at the stall: USB suspend, MCU reset (did the \
             startup LED animation replay?), or the EP1R race with IN dying before \
             the write was submitted."
        }
    }
}

/// Runs the driver with wedge diagnostics attached, printing a summary on exit.
pub fn run(loaded: LoadedConfig) -> DriverResult<()> {
    let diagnostics = WedgeDiagnostics::start();
    let result = app::run(loaded, &diagnostics);
    diagnostics.finish();
    result
}

struct WedgeDiagnostics {
    started: Instant,
    monitor: Arc<InputMonitor>,
    reader: JoinHandle<()>,
    dmesg: DmesgWatch,
    stats: Mutex<WriteStats>,
    last_summary: Mutex<Instant>,
    wedges: AtomicU64,
    in_flight: Mutex<Option<FlushProbe>>,
}

struct FlushProbe {
    started: Instant,
    snapshot: WriteSnapshot,
}

impl WedgeDiagnostics {
    fn start() -> Self {
        let monitor = Arc::new(InputMonitor::new());
        let reader = {
            let monitor = monitor.clone();
            thread::spawn(move || monitor.run())
        };
        println!(
            "{LOG_PREFIX} enabled: monitoring IN on a second handle, timing every \
             device write, diffing the kernel log at each wedge. Strike pads \
             continuously while reproducing; the EP1R race needs simultaneous IN \
             and OUT traffic."
        );
        let now = Instant::now();
        Self {
            started: now,
            monitor,
            reader,
            dmesg: DmesgWatch::start(),
            stats: Mutex::new(WriteStats::new()),
            last_summary: Mutex::new(now),
            wedges: AtomicU64::new(0),
            in_flight: Mutex::new(None),
        }
    }

    fn finish(self) {
        self.monitor.stop();
        println!("\n{LOG_PREFIX} --- summary ---");
        self.print_summary();
        let _ = self.reader.join();
    }

    fn maybe_print_summary(&self) {
        let mut last = self.last_summary.lock().unwrap();
        if last.elapsed() >= SUMMARY_INTERVAL {
            *last = Instant::now();
            self.print_summary();
        }
    }

    fn print_summary(&self) {
        let stats = self.stats.lock().unwrap();
        println!(
            "{LOG_PREFIX} {:.0} s: {} writes, {} input reports, {} pad strikes, \
             {} stall(s) >= {STALL_THRESHOLD:?}, {} wedge(s); latency {}",
            self.started.elapsed().as_secs_f64(),
            stats.writes(),
            self.monitor.reports(),
            self.monitor.pad_strikes(),
            stats.stalls(),
            self.wedges.load(Ordering::Acquire),
            stats.latency_line(),
        );
    }
}

impl SessionObserver for WedgeDiagnostics {
    /// Flushes with nothing to write are not timed, so idle loop iterations do
    /// not dilute the latency statistics.
    fn flush_started(&self, will_write: bool) {
        *self.in_flight.lock().unwrap() = will_write.then(|| FlushProbe {
            started: Instant::now(),
            snapshot: self.monitor.snapshot(),
        });
    }

    fn flush_finished(&self, result: &HidResult<()>) {
        let Some(probe) = self.in_flight.lock().unwrap().take() else {
            return;
        };
        let latency = probe.started.elapsed();
        match result {
            Ok(()) => {
                self.stats.lock().unwrap().record(latency);
                if latency >= STALL_THRESHOLD {
                    let survival = self.monitor.survival_since(probe.snapshot);
                    println!(
                        "{LOG_PREFIX} slow write: {latency:?}, {} input report(s) during it",
                        survival.reports_after_stall
                    );
                }
            }
            Err(err) => {
                let survival = self.monitor.survival_since(probe.snapshot);
                let wedge = self.wedges.fetch_add(1, Ordering::AcqRel) + 1;
                println!(
                    "\n{LOG_PREFIX} --- wedge #{wedge} at {:.1} s ---",
                    self.started.elapsed().as_secs_f64()
                );
                println!("{LOG_PREFIX} failing write blocked for {latency:?} then returned: {err}");
                survival.print();
                println!("{LOG_PREFIX} reading: {}", survival.reading());
                self.dmesg.print_new_lines("since the last check");
            }
        }
        self.maybe_print_summary();
    }

    fn fault(&self, reason: &str) {
        let wedge = self.wedges.fetch_add(1, Ordering::AcqRel) + 1;
        println!(
            "\n{LOG_PREFIX} --- fault #{wedge} at {:.1} s: {reason} ---",
            self.started.elapsed().as_secs_f64()
        );
        let since_last = self
            .monitor
            .now_ms()
            .saturating_sub(self.monitor.last_report_ms.load(Ordering::Acquire));
        println!(
            "{LOG_PREFIX} last input report {since_last} ms ago, {} in total",
            self.monitor.reports()
        );
        self.dmesg.print_new_lines("since the last check");
    }

    fn recovery_finished(&self, took: Duration, recovered: bool) {
        if recovered {
            println!("{LOG_PREFIX} recovered in {took:?}");
        } else {
            println!("{LOG_PREFIX} recovery ended without a device after {took:?}");
        }
        self.dmesg.print_new_lines("during recovery");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(reports_before: u64, submitted_ms: u64) -> WriteSnapshot {
        WriteSnapshot {
            reports_before,
            submitted_ms,
        }
    }

    #[test]
    fn percentiles_come_from_the_recorded_latency_distribution() {
        let mut stats = WriteStats::new();
        for ms in 1..=100 {
            stats.record(Duration::from_millis(ms));
        }

        assert_eq!(stats.percentile(0.50), Duration::from_millis(50));
        assert_eq!(stats.percentile(0.99), Duration::from_millis(99));
        assert_eq!(
            stats.stalls(),
            1,
            "only the 100 ms write reaches the stall threshold"
        );
    }

    #[test]
    fn a_latency_past_the_last_bucket_still_counts_and_keeps_its_exact_max() {
        let mut stats = WriteStats::new();
        stats.record(Duration::from_secs(9));

        assert_eq!(
            stats.percentile(0.99),
            Duration::from_millis(MAX_BUCKET_MS as u64)
        );
        assert!(stats.latency_line().ends_with("max 9s"));
    }

    #[test]
    fn empty_stats_report_zero_latency() {
        assert_eq!(WriteStats::new().percentile(0.99), Duration::ZERO);
    }

    #[test]
    fn kernel_lines_after_the_last_seen_one_are_new() {
        let log = "[1.0] a\n[2.0] b\n[3.0] c";

        assert_eq!(lines_after(log, Some("[2.0] b")), vec!["[3.0] c"]);
        assert_eq!(lines_after(log, Some("[3.0] c")), Vec::<&str>::new());
    }

    #[test]
    fn a_rotated_out_kernel_line_treats_the_whole_buffer_as_new() {
        let log = "[5.0] d\n[6.0] e";

        assert_eq!(
            lines_after(log, Some("[1.0] a")),
            vec!["[5.0] d", "[6.0] e"]
        );
        assert_eq!(lines_after(log, None), vec!["[5.0] d", "[6.0] e"]);
    }

    #[test]
    fn input_after_the_write_was_submitted_reads_as_the_ep1r_race() {
        let survival = InSurvival::measure(snapshot(10, 1_000), 14, 1_300, 7);

        assert_eq!(
            survival,
            InSurvival {
                reports_after_stall: 4,
                outlived_out_by: Some(Duration::from_millis(300)),
                pad_strikes_during_run: 7,
            }
        );
        assert!(survival.reading().starts_with("EP1R race"));
    }

    #[test]
    fn silent_input_after_the_write_reads_as_both_directions_dead() {
        let survival = InSurvival::measure(snapshot(10, 1_000), 10, 900, 7);

        assert_eq!(survival.outlived_out_by, None);
        assert!(survival.reading().starts_with("both directions dead"));
    }

    #[test]
    fn a_run_without_pad_strikes_is_inconclusive() {
        let survival = InSurvival::measure(snapshot(0, 1_000), 3, 1_200, 0);

        assert!(survival.reading().starts_with("inconclusive"));
    }
}
