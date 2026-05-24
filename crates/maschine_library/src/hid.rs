use hidapi::{HidDevice, HidResult};

/// Minimal HID read/write surface for device reports; faked in tests.
pub trait HidIo {
    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> HidResult<usize>;
    fn write(&self, data: &[u8]) -> HidResult<usize>;
}

impl HidIo for HidDevice {
    fn read_timeout(&self, buf: &mut [u8], timeout_ms: i32) -> HidResult<usize> {
        HidDevice::read_timeout(self, buf, timeout_ms)
    }

    fn write(&self, data: &[u8]) -> HidResult<usize> {
        HidDevice::write(self, data)
    }
}
