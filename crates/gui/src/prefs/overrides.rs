//! Pure builders for sparse `PartialSettings` deltas from GUI field edits.

use settings::PartialSettings;
use settings::partial::PartialHardwareSettings;

/// Build a delta touching only the `[hardware]` section, via `f`.
pub fn hardware_delta(f: impl FnOnce(&mut PartialHardwareSettings)) -> PartialSettings {
    let mut hardware = PartialHardwareSettings::default();
    f(&mut hardware);
    PartialSettings {
        hardware: Some(hardware),
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
}
