use hidapi::HidResult;
use maschine_library::hid::HidIo;
use maschine_library::lights::Lights;
use maschine_library::screen::Screen;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct DeviceOutputs {
    lights: Arc<Mutex<Lights>>,
    screen: Arc<Mutex<Screen>>,
    lights_dirty: Arc<AtomicBool>,
    screen_dirty: Arc<AtomicBool>,
}

impl DeviceOutputs {
    pub fn new() -> Self {
        Self {
            lights: Arc::new(Mutex::new(Lights::new())),
            screen: Arc::new(Mutex::new(Screen::new())),
            lights_dirty: Arc::new(AtomicBool::new(false)),
            screen_dirty: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_screen_mut<R>(&self, f: impl FnOnce(&mut Screen) -> R) -> R {
        let result = {
            let mut screen = self.screen.lock().unwrap();
            f(&mut screen)
        };
        self.screen_dirty.store(true, Ordering::SeqCst);
        result
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&Screen) -> R) -> R {
        let screen = self.screen.lock().unwrap();
        f(&screen)
    }

    pub fn screen_dirty(&self) -> bool {
        self.screen_dirty.load(Ordering::SeqCst)
    }

    pub fn take_screen_dirty(&self) -> bool {
        self.screen_dirty.swap(false, Ordering::SeqCst)
    }

    pub fn with_lights_mut<R>(&self, f: impl FnOnce(&mut Lights) -> R) -> R {
        let result = {
            let mut lights = self.lights.lock().unwrap();
            f(&mut lights)
        };
        self.lights_dirty.store(true, Ordering::SeqCst);
        result
    }

    pub fn with_lights<R>(&self, f: impl FnOnce(&Lights) -> R) -> R {
        let lights = self.lights.lock().unwrap();
        f(&lights)
    }

    pub fn lights_dirty(&self) -> bool {
        self.lights_dirty.load(Ordering::SeqCst)
    }

    pub fn take_lights_dirty(&self) -> bool {
        self.lights_dirty.swap(false, Ordering::SeqCst)
    }

    pub fn mark_all_dirty(&self) {
        self.lights_dirty.store(true, Ordering::SeqCst);
        self.screen_dirty.store(true, Ordering::SeqCst);
    }

    pub fn flush(&self, device: &impl HidIo) -> HidResult<()> {
        // Clear each flag before writing so an update during the write is not lost,
        // and restore it on failure so the unsent frame is retried next flush.
        if self.take_lights_dirty()
            && let Err(err) = self.with_lights(|lights| lights.write(device))
        {
            self.lights_dirty.store(true, Ordering::SeqCst);
            return Err(err);
        }

        if self.take_screen_dirty()
            && let Err(err) = self.with_screen(|screen| screen.write(device))
        {
            self.screen_dirty.store(true, Ordering::SeqCst);
            return Err(err);
        }

        Ok(())
    }
}

impl Default for DeviceOutputs {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct CapturingHid {
        writes: RefCell<Vec<Vec<u8>>>,
    }

    impl CapturingHid {
        fn new() -> Self {
            Self {
                writes: RefCell::new(Vec::new()),
            }
        }
    }

    impl HidIo for CapturingHid {
        fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            Ok(0)
        }

        fn write(&self, data: &[u8]) -> HidResult<usize> {
            self.writes.borrow_mut().push(data.to_vec());
            Ok(data.len())
        }

        fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
            Ok(())
        }
    }

    #[test]
    fn flush_writes_lights_report_only_when_dirty() {
        let outputs = DeviceOutputs::new();
        let hid = CapturingHid::new();

        outputs.flush(&hid).unwrap();
        assert!(hid.writes.borrow().is_empty());

        outputs.with_lights_mut(|_| {});
        outputs.flush(&hid).unwrap();

        let writes = hid.writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].len(), 81);
        assert_eq!(writes[0][0], 0x80);
    }

    #[test]
    fn flush_writes_screen_in_two_chunks_when_dirty() {
        let outputs = DeviceOutputs::new();
        let hid = CapturingHid::new();

        outputs.with_screen_mut(|_| {});
        outputs.flush(&hid).unwrap();

        let writes = hid.writes.borrow();
        assert_eq!(writes.len(), 2);
        assert_eq!(writes[0].len(), 265);
        assert_eq!(writes[1].len(), 265);
    }

    #[test]
    fn mark_all_dirty_sets_both_dirty_flags() {
        let outputs = DeviceOutputs::new();
        assert!(!outputs.lights_dirty());
        assert!(!outputs.screen_dirty());

        outputs.mark_all_dirty();

        assert!(outputs.lights_dirty());
        assert!(outputs.screen_dirty());
    }

    struct FailingHid;

    impl HidIo for FailingHid {
        fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            Ok(0)
        }

        fn write(&self, _data: &[u8]) -> HidResult<usize> {
            Err(hidapi::HidError::HidApiError {
                message: "write failed".to_string(),
            })
        }

        fn send_feature_report(&self, _data: &[u8]) -> HidResult<()> {
            Ok(())
        }
    }

    #[test]
    fn flush_restores_dirty_flag_when_write_fails() {
        let outputs = DeviceOutputs::new();
        outputs.with_lights_mut(|_| {});
        assert!(outputs.lights_dirty());

        assert!(outputs.flush(&FailingHid).is_err());

        assert!(
            outputs.lights_dirty(),
            "a failed write must leave the frame dirty so it is retried, not dropped"
        );
    }

    #[test]
    fn flush_clears_dirty_flag_after_successful_write() {
        let outputs = DeviceOutputs::new();
        let hid = CapturingHid::new();
        outputs.with_lights_mut(|_| {});

        outputs.flush(&hid).unwrap();

        assert!(!outputs.lights_dirty());
    }
}
