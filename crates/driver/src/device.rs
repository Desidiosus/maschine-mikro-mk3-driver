use std::sync::{Arc, Mutex};
use std::time::Duration;

use hidapi::{HidApi, HidDevice};
use maschine_library::hid::HidIo;
use maschine_library::preferences::{
    set_button_brightness, set_display_contrast, set_pad_sensitivity,
};
use maschine_library::{USB_PID, USB_VID};

use crate::error::{DriverError, DriverResult};
use crate::settings::Settings;
use crate::shared_settings::SharedSettings;

/// Opens and resets the HID device. Lets the runtime loop swap the device
/// handle on recovery while staying testable via a fake controller.
pub trait DeviceController {
    type Device: HidIo;

    /// (Re)enumerate, open the device, set non-blocking mode, and apply
    /// startup hardware preferences. Does NOT run the self-test.
    fn open(&self) -> DriverResult<Self::Device>;

    /// Best-effort USB-level reset (`USBDEVFS_RESET` via `rusb`).
    /// May invalidate existing handles by triggering re-enumeration.
    fn reset(&self) -> DriverResult<()>;

    /// A reset the write watchdog can call from its own thread while the runtime
    /// thread is blocked in a HID write. The default `None` leaves the watchdog
    /// unstarted, since test and borrowed controllers have no real device to reset.
    fn out_of_band_reset(&self) -> Option<ResetAction> {
        None
    }
}

/// A device reset callable from a thread that owns no device handle.
pub type ResetAction = Arc<dyn Fn() -> DriverResult<()> + Send + Sync>;

/// Issue `USBDEVFS_RESET` to every attached unit matching our VID/PID.
///
/// Free-standing so the write watchdog can call it without holding the
/// controller, which keeps `DeviceController` free of a `Sync` bound.
pub fn reset_usb_device() -> DriverResult<()> {
    let mut found = false;
    let mut last_err = None;
    for dev in rusb::devices()?.iter() {
        // An unrelated device with an unreadable descriptor must not abort the search.
        let Ok(desc) = dev.device_descriptor() else {
            continue;
        };
        if desc.vendor_id() != USB_VID || desc.product_id() != USB_PID {
            continue;
        }
        found = true;
        // During re-enumeration a stale and a fresh node can briefly coexist, and
        // the wedged one may not be enumerated first, so reset every match.
        match dev.open().and_then(|handle| handle.reset()) {
            // libusb reports `NotFound` when the reset forced a re-enumeration,
            // which means the reset took effect.
            Ok(()) | Err(rusb::Error::NotFound) => {}
            Err(err) => last_err = Some(err),
        }
    }
    match (found, last_err) {
        (false, _) => Err(DriverError::Usb(rusb::Error::NoDevice)),
        (true, Some(err)) => Err(DriverError::Usb(err)),
        (true, None) => Ok(()),
    }
}

/// Timing knobs for the recovery ladder. Tests use near-zero values to keep
/// escalation fast.
#[derive(Debug, Clone)]
pub struct RecoveryTuning {
    pub quick_retries: u32,
    pub quick_retry_delay: Duration,
    /// Upper bound on USB resets per recovery cycle. A reopen follows each reset,
    /// so a wedge that clears on the first reset costs only one.
    pub reset_attempts: u32,
    /// Wait after each reset for the device to re-enumerate and re-init.
    pub reset_settle: Duration,
    pub backoff_initial: Duration,
    pub backoff_cap: Duration,
    /// Reset-and-reopen cycles per recovery before the device is reported lost,
    /// so a real unplug falls back to the re-acquire loop instead of being reset
    /// forever.
    pub max_reopen_attempts: u32,
    /// Recoveries without a successful flush in between before giving up, so a
    /// device that reopens but stays wedged is reported lost.
    pub max_consecutive_recoveries: u32,
    /// How long an in-flight HID write may block before the watchdog resets the
    /// device. Without it, a hidraw write to a wedged OUT endpoint blocks in
    /// `usb_interrupt_msg` for `USB_CTRL_SET_TIMEOUT` (5 s) before recovery can
    /// start. Healthy 81-byte lights writes complete in 8-16 ms.
    pub write_stall_timeout: Duration,
    /// How often the watchdog samples the in-flight write.
    pub write_stall_poll: Duration,
}

impl Default for RecoveryTuning {
    fn default() -> Self {
        Self {
            quick_retries: 3,
            quick_retry_delay: Duration::from_millis(5),
            reset_attempts: 2,
            reset_settle: Duration::from_millis(500),
            backoff_initial: Duration::from_millis(100),
            backoff_cap: Duration::from_millis(2000),
            max_reopen_attempts: 6,
            max_consecutive_recoveries: 4,
            write_stall_timeout: Duration::from_millis(500),
            write_stall_poll: Duration::from_millis(100),
        }
    }
}

/// Real controller backed by hidapi (open) and rusb (reset).
pub struct HidController {
    api: Mutex<HidApi>,
    settings: SharedSettings,
}

impl HidController {
    /// Keeps the live settings handle rather than a snapshot, so recovery
    /// reopens and hotplug re-acquires apply the settings currently in effect.
    pub fn new(settings: SharedSettings) -> DriverResult<Self> {
        Ok(Self {
            api: Mutex::new(HidApi::new()?),
            settings,
        })
    }
}

fn apply_startup_preferences(device: &HidDevice, settings: &Settings) -> DriverResult<()> {
    set_pad_sensitivity(device, settings.hardware.pad_sensitivity)?;
    set_display_contrast(device, settings.hardware.display_contrast)?;
    set_button_brightness(device, settings.hardware.led_brightness)?;
    Ok(())
}

/// A device that just re-enumerated after a USB reset opens before it accepts
/// the preference reports. Without the retry it would count as a failed open,
/// costing an extra reset cycle in recovery or a 2 s re-acquire stall.
fn apply_startup_preferences_settling(device: &HidDevice, settings: &Settings) -> DriverResult<()> {
    const ATTEMPTS: u32 = 3;
    const RETRY_DELAY: Duration = Duration::from_millis(40);

    let mut result = apply_startup_preferences(device, settings);
    for _ in 1..ATTEMPTS {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(RETRY_DELAY);
        result = apply_startup_preferences(device, settings);
    }
    result
}

impl DeviceController for HidController {
    type Device = HidDevice;

    fn open(&self) -> DriverResult<HidDevice> {
        let mut api = self.api.lock().unwrap();
        api.refresh_devices()?;
        let device = api.open(USB_VID, USB_PID)?;
        device.set_blocking_mode(false)?;
        apply_startup_preferences_settling(&device, &self.settings.load())?;
        Ok(device)
    }

    fn reset(&self) -> DriverResult<()> {
        reset_usb_device()
    }

    fn out_of_band_reset(&self) -> Option<ResetAction> {
        Some(Arc::new(reset_usb_device))
    }
}

/// Wraps an already-open device for callers that never exercise recovery:
/// `open` always yields the same reference and `reset` is a no-op.
pub struct BorrowedController<'a, D: HidIo> {
    device: &'a D,
}

impl<'a, D: HidIo> BorrowedController<'a, D> {
    pub fn new(device: &'a D) -> Self {
        Self { device }
    }
}

impl<'a, D: HidIo> DeviceController for BorrowedController<'a, D> {
    type Device = &'a D;

    fn open(&self) -> DriverResult<Self::Device> {
        Ok(self.device)
    }

    fn reset(&self) -> DriverResult<()> {
        Ok(())
    }
}
