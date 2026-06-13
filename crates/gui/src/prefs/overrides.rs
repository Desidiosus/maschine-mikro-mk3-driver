//! Pure builders for sparse `PartialSettings` deltas from GUI field edits.

use settings::PartialSettings;
use settings::partial::{PartialGlobalSettings, PartialHardwareSettings};

/// Build a delta touching only the `[hardware]` section, via `f`.
pub fn hardware_delta(f: impl FnOnce(&mut PartialHardwareSettings)) -> PartialSettings {
    let mut hardware = PartialHardwareSettings::default();
    f(&mut hardware);
    PartialSettings {
        hardware: Some(hardware),
        ..Default::default()
    }
}

/// Build a delta touching only the `[global]` section, via `f`.
pub fn global_delta(f: impl FnOnce(&mut PartialGlobalSettings)) -> PartialSettings {
    let mut global = PartialGlobalSettings::default();
    f(&mut global);
    PartialSettings {
        global: Some(global),
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
    fn global_delta_sets_only_the_edited_field() {
        let delta = global_delta(|g| g.client_name = Some("Custom".to_string()));
        let merged = Settings::default().merge_overrides(delta);
        assert_eq!(merged.global.client_name, "Custom");
        // Other global fields stay at defaults.
        assert_eq!(
            merged.global.port_name,
            Settings::default().global.port_name
        );
    }
}
