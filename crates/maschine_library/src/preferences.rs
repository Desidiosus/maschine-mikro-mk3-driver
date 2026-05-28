use hidapi::{HidDevice, HidResult};

const MAX_HARDWARE_PREFERENCE_VALUE: u8 = 100;

fn pad_sensitivity_report(value: u8) -> [u8; 33] {
    let mut report = [0u8; 33];
    report[..6].copy_from_slice(&[
        0xf4,
        0x22,
        0xff,
        0x05,
        0x01,
        clamp_hardware_preference_value(value),
    ]);
    report
}

fn display_contrast_report(value: u8) -> [u8; 11] {
    [
        0xf8,
        0x80,
        0x00,
        0x20,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        clamp_hardware_preference_value(value),
        0x00,
    ]
}

pub fn set_pad_sensitivity(device: &HidDevice, value: u8) -> HidResult<()> {
    device.write(&pad_sensitivity_report(value))?;
    Ok(())
}

pub fn set_display_contrast(device: &HidDevice, value: u8) -> HidResult<()> {
    device.send_feature_report(&display_contrast_report(value))?;
    Ok(())
}

fn clamp_hardware_preference_value(value: u8) -> u8 {
    value.min(MAX_HARDWARE_PREFERENCE_VALUE)
}

#[cfg(test)]
mod tests {
    use super::{display_contrast_report, pad_sensitivity_report};

    #[test]
    fn pad_sensitivity_report_matches_known_bytes() {
        assert_eq!(
            pad_sensitivity_report(50),
            [
                0xf4, 0x22, 0xff, 0x05, 0x01, 50, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn display_contrast_report_matches_known_bytes() {
        assert_eq!(
            display_contrast_report(50),
            [
                0xf8, 0x80, 0x00, 0x20, 0x00, 0x01, 0x00, 0x00, 0x00, 50, 0x00
            ]
        );
    }

    #[test]
    fn pad_sensitivity_report_clamps_values_above_100() {
        assert_eq!(pad_sensitivity_report(127)[5], 100);
    }

    #[test]
    fn display_contrast_report_clamps_values_above_100() {
        assert_eq!(display_contrast_report(127)[9], 100);
    }
}
