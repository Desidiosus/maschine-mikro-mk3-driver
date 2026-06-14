use std::path::Path;
use std::sync::Arc;

use maschine_library::hid::HidIo;
use maschine_library::preferences::{set_display_contrast, set_pad_sensitivity};

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
    /// Button backlight settings changed → re-apply the backlight level to all
    /// backlight-capable buttons (or turn them off when disabled).
    pub reinit_backlight: bool,
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
        reinit_backlight: old.backlight_buttons != new.backlight_buttons
            || old.backlight_brightness != new.backlight_brightness,
    };

    Ok(effects)
}

/// Apply hardware `effects` to the device and outputs. Runs on the loop thread.
pub fn apply_side_effects<D: HidIo>(
    effects: &SideEffects,
    settings: &crate::settings::Settings,
    device: &D,
    outputs: &DeviceOutputs,
) -> DriverResult<()> {
    if let Some(value) = effects.pad_sensitivity {
        set_pad_sensitivity(device, value)?;
    }
    if let Some(value) = effects.display_contrast {
        set_display_contrast(device, value)?;
    }
    if effects.reinit_backlight {
        crate::app::refresh_button_backlight(outputs, settings);
    }
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
            reinit_backlight: false,
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
}
