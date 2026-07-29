use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use driver::shared_settings::new_shared;
use hidapi::HidResult;
use maschine_library::controls::Buttons;
use maschine_library::hid::HidIo;

/// Drives `run_device_session` through a fixed number of iterations: the
/// first `read_timeout` call reports a `Group` button press (opens the page
/// picker), every call after that reports no input, and once `iterations`
/// reads have happened it flips the shared `shutdown` flag so the loop exits
/// on its own next check. Counting reads (not wall-clock time) keeps the
/// iteration count exact and the test independent of how fast the loop runs.
struct CapturingHid<'a> {
    writes: Mutex<Vec<Vec<u8>>>,
    reads: AtomicUsize,
    iterations: usize,
    shutdown: &'a AtomicBool,
}

impl HidIo for CapturingHid<'_> {
    fn read_timeout(&self, buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
        let n = self.reads.fetch_add(1, Ordering::Relaxed);
        if n >= self.iterations {
            self.shutdown.store(true, Ordering::Relaxed);
            return Ok(0);
        }
        if n == 0 {
            // Button-report packet: report id 0x01, `Group` (index 14) is bit
            // 6 of buf[2] (idx = i*8+j with i=1, j=6 -> buf[i+1] = buf[2]).
            buf[0] = 0x01;
            buf[2] = 1 << (Buttons::Group as usize % 8);
            Ok(3)
        } else {
            Ok(0)
        }
    }

    fn write(&self, data: &[u8]) -> HidResult<usize> {
        self.writes.lock().unwrap().push(data.to_vec());
        Ok(data.len())
    }

    fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
        Ok(())
    }
}

fn test_settings() -> settings::Settings {
    let mut s = settings::Settings::default();
    s.global.client_name = "Client".into();
    s.global.port_name = "Port".into();
    s.global.port_name_in = "Input".into();
    s.bridge.autoconnect_virmidi = false;
    // The self-test splash sleeps ~1s and hammers the device with writes
    // unrelated to picker rendering; disable it so the write count below
    // reflects only the loop's own output decisions.
    s.driver.self_test_on_launch = false;
    s.pad_paging.enabled = true;
    s.pad_paging
        .pages
        .push(settings::pad_paging::default_page());
    s
}

/// A `Group` hold with no further input must not re-render the picker every
/// ~1 ms loop iteration: that pushes one identical 81-byte lights report per
/// iteration. The picker is rendered only on open/select (or when something
/// else repaints the pads), plus a 50 ms self-healing backstop.
#[test]
fn holding_group_with_no_further_input_does_not_rerender_every_iteration() {
    if !Path::new("/dev/snd/seq").exists() {
        eprintln!(
            "skipping holding_group_with_no_further_input_does_not_rerender_every_iteration: \
             /dev/snd/seq not present (no ALSA sequencer support on this host)"
        );
        return;
    }

    const ITERATIONS: usize = 800;
    let shutdown = AtomicBool::new(false);
    let hid = CapturingHid {
        writes: Mutex::new(Vec::new()),
        reads: AtomicUsize::new(0),
        iterations: ITERATIONS,
        shutdown: &shutdown,
    };

    let (_effects_tx, effects_rx) = std::sync::mpsc::channel();
    let subscriber = driver::ipc::new_subscriber();
    driver::app::run_with_device(
        new_shared(test_settings()),
        &hid,
        &shutdown,
        effects_rx,
        subscriber,
    )
    .unwrap();

    let writes = hid.writes.lock().unwrap();
    let lights_writes = writes
        .iter()
        .filter(|w| w.len() == 81 && w[0] == 0x80)
        .count();

    // This must stay a "far fewer than iterations" inequality, not an exact
    // count: the 50 ms backstop repaint (which corrects pad writes from other
    // sources, e.g. the MIDI-in feedback thread) fires on wall-clock time, and
    // this fake HID drives iterations far faster than 50 ms, so how many
    // backstop repaints happen to land is not deterministic. The property under
    // test -- writes grossly sublinear in iterations, instead of one per
    // iteration -- holds regardless. Do not tighten this into an exact count.
    assert!(
        lights_writes < ITERATIONS / 10,
        "expected picker rendering to be gated rather than re-rendered every \
         iteration: {lights_writes} lights writes across {ITERATIONS} loop iterations"
    );
}
