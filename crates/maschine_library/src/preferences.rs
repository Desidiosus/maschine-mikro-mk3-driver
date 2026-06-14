use hidapi::HidResult;

use crate::hid::HidIo;

const MAX_HARDWARE_PREFERENCE_VALUE: u8 = 100;
const MAX_BUTTON_BRIGHTNESS: u8 = 10;

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

fn button_brightness_report(value: u8) -> [u8; 2] {
    [0xf3, value.min(MAX_BUTTON_BRIGHTNESS)]
}

pub fn set_pad_sensitivity<D: HidIo>(device: &D, value: u8) -> HidResult<()> {
    device.write(&pad_sensitivity_report(value))?;
    Ok(())
}

pub fn set_display_contrast<D: HidIo>(device: &D, value: u8) -> HidResult<()> {
    device.send_feature_report(&display_contrast_report(value))?;
    Ok(())
}

pub fn set_button_brightness<D: HidIo>(device: &D, value: u8) -> HidResult<()> {
    device.write(&button_brightness_report(value))?;
    Ok(())
}

fn clamp_hardware_preference_value(value: u8) -> u8 {
    value.min(MAX_HARDWARE_PREFERENCE_VALUE)
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_BUTTON_BRIGHTNESS, button_brightness_report, display_contrast_report,
        pad_sensitivity_report,
    };

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

    #[test]
    fn button_brightness_report_matches_known_bytes() {
        assert_eq!(button_brightness_report(10), [0xf3, 0x0a]);
        assert_eq!(button_brightness_report(0), [0xf3, 0x00]);
    }

    #[test]
    fn button_brightness_report_clamps_values_above_10() {
        assert_eq!(button_brightness_report(50)[1], MAX_BUTTON_BRIGHTNESS);
    }
}
