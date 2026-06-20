use std::path::Path;
use std::sync::Arc;

use maschine_library::hid::HidIo;
use maschine_library::preferences::{
    set_button_brightness, set_display_contrast, set_pad_sensitivity,
};

use crate::error::DriverResult;
use crate::outputs::DeviceOutputs;
use crate::settings::PartialSettings;
use crate::settings::Settings;
use crate::settings::persist::save_overrides;
use crate::shared_settings::SharedSettings;

/// Hardware side effects that must be pushed to the device/outputs after a
/// successful `apply_delta`. The loop thread (which owns HID) applies these.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SideEffects {
    /// New pad sensitivity to re-push, if the delta changed it.
    pub pad_sensitivity: Option<u8>,
    /// New display contrast to re-push, if the delta changed it.
    pub display_contrast: Option<u8>,
    /// New button-backlight brightness (0..=10) to push to the device via the
    /// `0xf3` report, if the delta changed it. The global preference scales the
    /// emitted intensity; `0` turns the backlight off.
    pub button_brightness: Option<u8>,
    /// Re-apply the per-LED ambient state to all backlight-capable buttons. Only
    /// set when the backlight toggles between off and on, so adjusting brightness
    /// among non-zero values scales `0xf3` without overwriting DAW-driven LEDs.
    pub refresh_backlight: bool,
    /// Bitmask (bit `i` = pad `i`) of pads whose `led` config changed and whose
    /// idle LED must be re-seeded, so Single/Dual-idle pads update at rest.
    /// Only the changed pads are touched, so an edit never clobbers an unrelated
    /// pad that is currently lit by live feedback.
    pub refresh_pad_leds: u16,
}

/// Merge `delta` onto the live settings, validate, (when `persist`) persist the
/// sparse overrides relative to `persist_base` to `persist_path`, then atomically
/// swap the new settings in. Returns the side effects the caller must apply.
///
/// `persist=false` applies live (handle swap + side effects) without writing the
/// config file — used while a slider is being dragged. On validation OR
/// persistence failure the handle is left untouched and nothing goes live, so the
/// on-disk config and the in-memory settings never diverge: a failed persist
/// surfaces as an error and the prior settings stay authoritative.
pub fn apply_delta(
    handle: &SharedSettings,
    delta: PartialSettings,
    persist_base: &Settings,
    persist_path: &Path,
    persist: bool,
) -> Result<SideEffects, String> {
    let current = handle.load_full();
    let merged = (*current).clone().merge_overrides(delta);
    merged.validate()?;

    // Persist BEFORE swapping in the new settings: if the write fails the live
    // state stays at `current`, matching the on-disk file, and the error is
    // surfaced. Once persisted (or when not persisting), commit the swap.
    if persist {
        save_overrides(persist_path, &merged, persist_base)?;
    }
    handle.store(Arc::new(merged.clone()));

    // Derive hardware side effects from what actually changed between the live
    // settings and the merged result, so every path that alters a hardware field
    // re-pushes exactly that field — and an apply that leaves hardware untouched
    // (or re-sets the same value) queues no redundant device I/O.
    let (old, new) = (&current.hardware, &merged.hardware);
    let effects = SideEffects {
        pad_sensitivity: (old.pad_sensitivity != new.pad_sensitivity)
            .then_some(new.pad_sensitivity),
        display_contrast: (old.display_contrast != new.display_contrast)
            .then_some(new.display_contrast),
        button_brightness: (old.led_brightness != new.led_brightness).then_some(new.led_brightness),
        refresh_backlight: (old.led_brightness > 0) != (new.led_brightness > 0),
        refresh_pad_leds: current
            .pads
            .iter()
            .zip(merged.pads.iter())
            .enumerate()
            .filter(|(_, (a, b))| a.led != b.led)
            .fold(0u16, |mask, (i, _)| mask | (1 << i)),
    };

    Ok(effects)
}

/// Push the device-register `effects` (pad sensitivity, display contrast, button
/// brightness) over HID. These are independent of the blanked display state, so
/// the loop applies them immediately even while soft-off is active.
pub fn apply_device_registers<D: HidIo>(effects: &SideEffects, device: &D) -> DriverResult<()> {
    if let Some(value) = effects.pad_sensitivity {
        set_pad_sensitivity(device, value)?;
    }
    if let Some(value) = effects.display_contrast {
        set_display_contrast(device, value)?;
    }
    if let Some(value) = effects.button_brightness {
        set_button_brightness(device, value)?;
    }
    Ok(())
}

/// Re-render the output overlays (button backlight, pad idle LEDs) the `effects`
/// request. These write to `outputs`, so the loop defers them while soft-off
/// blanks the device and flushes them on wake.
pub fn apply_output_overlays(
    effects: &SideEffects,
    settings: &crate::settings::Settings,
    outputs: &DeviceOutputs,
) {
    if effects.refresh_backlight {
        crate::app::refresh_button_backlight(outputs, settings);
    }
    if effects.refresh_pad_leds != 0 {
        crate::app::reseed_pad_leds(outputs, settings, effects.refresh_pad_leds);
    }
}

/// Apply both halves of `effects`. Used off the soft-off path (tests, and any
/// caller that does not need to defer overlays).
pub fn apply_side_effects<D: HidIo>(
    effects: &SideEffects,
    settings: &crate::settings::Settings,
    device: &D,
    outputs: &DeviceOutputs,
) -> DriverResult<()> {
    apply_device_registers(effects, device)?;
    apply_output_overlays(effects, settings, outputs);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use crate::shared_settings::new_shared;
    use hidapi::HidResult;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeHid {
        writes: RefCell<Vec<Vec<u8>>>,
        features: RefCell<Vec<Vec<u8>>>,
    }

    impl HidIo for FakeHid {
        fn read_timeout(&self, _buf: &mut [u8], _timeout_ms: i32) -> HidResult<usize> {
            Ok(0)
        }
        fn write(&self, data: &[u8]) -> HidResult<usize> {
            self.writes.borrow_mut().push(data.to_vec());
            Ok(data.len())
        }
        fn send_feature_report(&self, data: &[u8]) -> HidResult<()> {
            self.features.borrow_mut().push(data.to_vec());
            Ok(())
        }
    }

    fn temp_config_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("mmk3-apply-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{}.toml", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn pad_sensitivity_delta(value: u8) -> PartialSettings {
        toml::from_str(&format!("[hardware]\npad_sensitivity = {value}\n")).unwrap()
    }

    #[test]
    fn apply_delta_updates_handle_and_persists_for_hardware_change() {
        let path = temp_config_path("hardware");
        let handle = new_shared(Settings::default());

        let effects = apply_delta(
            &handle,
            pad_sensitivity_delta(73),
            &Settings::default(),
            &path,
            true,
        )
        .unwrap();

        assert_eq!(effects.pad_sensitivity, Some(73));
        assert_eq!(handle.load().hardware.pad_sensitivity, 73);

        // Persisted overrides reload to the same live settings.
        let reloaded = crate::settings::persist::load_xdg(&path).unwrap();
        assert_eq!(reloaded.hardware.pad_sensitivity, 73);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_delta_without_persist_updates_handle_without_writing() {
        let path = temp_config_path("no-persist");
        let handle = new_shared(Settings::default());

        let effects = apply_delta(
            &handle,
            pad_sensitivity_delta(73),
            &Settings::default(),
            &path,
            false,
        )
        .unwrap();

        assert_eq!(effects.pad_sensitivity, Some(73));
        assert_eq!(handle.load().hardware.pad_sensitivity, 73);
        assert!(!path.exists(), "no file written when persist = false");
    }

    #[test]
    fn apply_delta_rejects_invalid_delta_without_side_effects() {
        let path = temp_config_path("invalid");
        let handle = new_shared(Settings::default());

        // pad_sensitivity > 100 fails validate().
        let result = apply_delta(
            &handle,
            pad_sensitivity_delta(200),
            &Settings::default(),
            &path,
            true,
        );

        assert!(result.is_err());
        assert_eq!(handle.load().hardware.pad_sensitivity, 50); // unchanged default
        assert!(!path.exists(), "no file written on validation failure");
    }

    #[test]
    fn apply_delta_leaves_live_settings_unchanged_when_persist_fails() {
        // A persist_path whose parent is a regular file makes create_dir_all (and
        // thus the write) fail, so the swap must not happen.
        let blocker = temp_config_path("persist-fail-blocker");
        std::fs::write(&blocker, "x").unwrap();
        let unwritable = blocker.join("nested").join("config.toml");

        let handle = new_shared(Settings::default());
        let result = apply_delta(
            &handle,
            pad_sensitivity_delta(73),
            &Settings::default(),
            &unwritable,
            true,
        );

        assert!(result.is_err(), "persist failure must surface as an error");
        assert_eq!(
            handle.load().hardware.pad_sensitivity,
            50,
            "live settings stay at the prior value when persist fails"
        );
        let _ = std::fs::remove_file(&blocker);
    }

    #[test]
    fn apply_delta_for_action_change_reports_no_hardware_side_effects() {
        let path = temp_config_path("action");
        let handle = new_shared(Settings::default());

        let delta: PartialSettings =
            toml::from_str("[buttons.play.press]\ntype = \"cc\"\ncc = 99\n").unwrap();
        let effects = apply_delta(&handle, delta, &Settings::default(), &path, true).unwrap();

        assert_eq!(effects, SideEffects::default());
        let reloaded = crate::settings::persist::load_xdg(&path).unwrap();
        assert_eq!(reloaded, *handle.load_full());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_side_effects_repushes_hardware_prefs_to_device() {
        let settings = Settings::default();
        let outputs = DeviceOutputs::new();
        let device = FakeHid::default();

        let effects = SideEffects {
            pad_sensitivity: Some(70),
            display_contrast: Some(30),
            button_brightness: None,
            refresh_backlight: false,
            refresh_pad_leds: 0,
        };
        apply_side_effects(&effects, &settings, &device, &outputs).unwrap();

        // pad sensitivity goes over write(); contrast over send_feature_report().
        let writes = device.writes.borrow();
        let features = device.features.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0][0], 0xf4); // pad-sensitivity report marker
        assert_eq!(features.len(), 1);
        assert_eq!(features[0][0], 0xf8); // display-contrast report marker
    }

    #[test]
    fn apply_side_effects_pushes_button_brightness() {
        let settings = Settings::default();
        let outputs = DeviceOutputs::new();
        let device = FakeHid::default();

        let effects = SideEffects {
            pad_sensitivity: None,
            display_contrast: None,
            button_brightness: Some(7),
            refresh_backlight: false,
            refresh_pad_leds: 0,
        };
        apply_side_effects(&effects, &settings, &device, &outputs).unwrap();

        let writes = device.writes.borrow();
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0], vec![0xf3, 7]);
    }

    fn pad_led_source_delta() -> PartialSettings {
        toml::from_str("[pads.1.led]\nsource = \"midi_in\"\n").unwrap()
    }

    #[test]
    fn apply_delta_for_pad_led_change_requests_a_reseed() {
        let path = temp_config_path("pad-led");
        let handle = new_shared(Settings::default());
        let effects = apply_delta(
            &handle,
            pad_led_source_delta(),
            &Settings::default(),
            &path,
            false,
        )
        .unwrap();
        // TOML key `pads.1` maps to internal pad 12; only that pad's bit is set.
        assert_eq!(
            effects.refresh_pad_leds,
            1 << 12,
            "a pad-LED change re-seeds only the changed pad"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn apply_delta_for_hardware_change_does_not_reseed_pads() {
        let path = temp_config_path("no-pad-led");
        let handle = new_shared(Settings::default());
        let effects = apply_delta(
            &handle,
            pad_sensitivity_delta(73),
            &Settings::default(),
            &path,
            false,
        )
        .unwrap();
        assert_eq!(effects.refresh_pad_leds, 0);
        let _ = std::fs::remove_file(&path);
    }

    fn backlight_delta(value: u8) -> PartialSettings {
        toml::from_str(&format!("[hardware]\nled_brightness = {value}\n")).unwrap()
    }

    #[test]
    fn apply_delta_refreshes_leds_only_when_backlight_toggles() {
        let path = temp_config_path("backlight-toggle");
        // Default brightness is non-zero (on).
        let handle = new_shared(Settings::default());

        // On → on: brightness changes but stays lit; push 0xf3 without an LED refresh.
        let effects = apply_delta(
            &handle,
            backlight_delta(8),
            &Settings::default(),
            &path,
            false,
        )
        .unwrap();
        assert_eq!(effects.button_brightness, Some(8));
        assert!(!effects.refresh_backlight);

        // On → off: refresh to drive the ambient LEDs dark.
        let effects = apply_delta(
            &handle,
            backlight_delta(0),
            &Settings::default(),
            &path,
            false,
        )
        .unwrap();
        assert_eq!(effects.button_brightness, Some(0));
        assert!(effects.refresh_backlight);

        // Off → on: refresh to light the ambient LEDs.
        let effects = apply_delta(
            &handle,
            backlight_delta(4),
            &Settings::default(),
            &path,
            false,
        )
        .unwrap();
        assert_eq!(effects.button_brightness, Some(4));
        assert!(effects.refresh_backlight);

        let _ = std::fs::remove_file(&path);
    }
}
