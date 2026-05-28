use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use hidapi::{HidError, HidResult};
use maschine_library::hid::HidIo;

#[derive(Default)]
struct CapturingHid {
    writes: Mutex<Vec<Vec<u8>>>,
    read_behavior: ReadBehavior,
}

#[derive(Default)]
enum ReadBehavior {
    #[default]
    Empty,
    EintrOnce(Mutex<bool>),
}

impl HidIo for CapturingHid {
    fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
        match &self.read_behavior {
            ReadBehavior::Empty => Ok(0),
            ReadBehavior::EintrOnce(returned) => {
                let mut returned = returned.lock().unwrap();
                if *returned {
                    Ok(0)
                } else {
                    *returned = true;
                    Err(HidError::HidApiError {
                        message: "Interrupted system call".to_string(),
                    })
                }
            }
        }
    }

    fn write(&self, data: &[u8]) -> HidResult<usize> {
        self.writes.lock().unwrap().push(data.to_vec());
        Ok(data.len())
    }
}

fn test_settings() -> driver::settings::Settings {
    let mut s = driver::settings::Settings::default();
    s.global.client_name = "Client".into();
    s.global.port_name = "Port".into();
    s.global.port_name_in = "Input".into();
    s.bridge.autoconnect_virmidi = false;
    s
}

fn assert_last_lights_report_blank(writes: &[Vec<u8>]) {
    let last_lights = writes
        .iter()
        .rev()
        .find(|w| w.len() == 81 && w[0] == 0x80)
        .expect("expected a lights report write");
    assert!(
        last_lights[1..].iter().all(|&b| b == 0),
        "last lights report should be all zeros, got {:?}",
        &last_lights[1..]
    );
}

#[test]
fn run_with_device_blanks_lights_on_shutdown() {
    if !Path::new("/dev/snd/seq").exists() {
        return;
    }

    let hid = CapturingHid::default();
    let shutdown = AtomicBool::new(true);

    driver::app::run_with_device(test_settings(), &hid, &shutdown).unwrap();

    assert_last_lights_report_blank(&hid.writes.lock().unwrap());
}

#[test]
fn run_with_device_treats_read_error_as_graceful_exit_when_shutdown_requested() {
    if !Path::new("/dev/snd/seq").exists() {
        return;
    }

    let hid = CapturingHid {
        writes: Mutex::new(Vec::new()),
        read_behavior: ReadBehavior::EintrOnce(Mutex::new(false)),
    };
    let shutdown = AtomicBool::new(false);

    let result = std::thread::scope(|s| {
        let shutdown_ref = &shutdown;
        let hid_ref = &hid;
        let handle =
            s.spawn(move || driver::app::run_with_device(test_settings(), hid_ref, shutdown_ref));
        shutdown_ref.store(true, Ordering::Relaxed);
        handle.join().unwrap()
    });

    result.expect("run_with_device should exit cleanly when EINTR coincides with shutdown");
    assert_last_lights_report_blank(&hid.writes.lock().unwrap());
}
