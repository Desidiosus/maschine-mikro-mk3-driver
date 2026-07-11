//! Pure builders for sparse `PartialSettings` deltas from GUI field edits.

use settings::PartialSettings;
use settings::partial::{PartialDriverSettings, PartialHardwareSettings};

/// Build a delta touching only the `[hardware]` section, via `f`.
pub fn hardware_delta(f: impl FnOnce(&mut PartialHardwareSettings)) -> PartialSettings {
    let mut hardware = PartialHardwareSettings::default();
    f(&mut hardware);
    PartialSettings {
        hardware: Some(hardware),
        ..Default::default()
    }
}

/// Build a delta touching only the `[driver]` section, via `f`.
pub fn driver_delta(f: impl FnOnce(&mut PartialDriverSettings)) -> PartialSettings {
    let mut driver = PartialDriverSettings::default();
    f(&mut driver);
    PartialSettings {
        driver: Some(driver),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::Settings;

    #[test]
    fn hardware_delta_sets_only_the_edited_field() {
        let delta = hardware_delta(|h| h.pad_sensitivity = Some(73));
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(merged.hardware.pad_sensitivity, 73);
        // Other hardware fields stay at defaults.
        assert_eq!(
            merged.hardware.display_contrast,
            Settings::default().hardware.display_contrast
        );
    }

    #[test]
    fn driver_delta_sets_only_the_edited_field() {
        let delta = driver_delta(|d| d.soft_off_enabled = Some(false));
        let merged = Settings::default().merge_overrides(delta);
        assert!(!merged.driver.soft_off_enabled);
        // Untouched flag stays at its default.
        assert!(merged.driver.self_test_on_launch);
    }
}
