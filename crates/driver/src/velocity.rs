pub use ::settings::PadVelocityCurve;

pub(crate) fn raw_pad_velocity(value: u16) -> u8 {
    if value == 0 {
        return 0;
    }

    (value >> 5).clamp(1, 127) as u8
}

pub(crate) fn pad_velocity(value: u16, curve: PadVelocityCurve) -> u8 {
    apply_pad_velocity_curve(raw_pad_velocity(value), curve)
}

pub(crate) fn apply_pad_velocity_curve(velocity: u8, curve: PadVelocityCurve) -> u8 {
    if velocity == 0 || velocity == 127 {
        return velocity;
    }

    let gamma = match curve {
        PadVelocityCurve::Soft3 => 0.45,
        PadVelocityCurve::Soft2 => 0.60,
        PadVelocityCurve::Soft1 => 0.75,
        PadVelocityCurve::Linear => 1.00,
        PadVelocityCurve::Hard1 => 1.35,
        PadVelocityCurve::Hard2 => 1.75,
        PadVelocityCurve::Hard3 => 2.30,
    };

    ((f64::from(velocity) / 127.0).powf(gamma) * 127.0)
        .round()
        .clamp(1.0, 127.0) as u8
}

#[cfg(test)]
mod tests {
    use super::{PadVelocityCurve, apply_pad_velocity_curve, pad_velocity, raw_pad_velocity};

    #[test]
    fn raw_velocity_clamps_to_existing_hid_mapping() {
        assert_eq!(raw_pad_velocity(0), 0);
        assert_eq!(raw_pad_velocity(1), 1);
        assert_eq!(raw_pad_velocity(31), 1);
        assert_eq!(raw_pad_velocity(32), 1);
        assert_eq!(raw_pad_velocity(3200), 100);
        assert_eq!(raw_pad_velocity(4095), 127);
        assert_eq!(raw_pad_velocity(4096), 127);
    }

    #[test]
    fn linear_curve_preserves_current_raw_velocity_mapping() {
        assert_eq!(pad_velocity(0, PadVelocityCurve::Linear), 0);
        assert_eq!(pad_velocity(1, PadVelocityCurve::Linear), 1);
        assert_eq!(pad_velocity(31, PadVelocityCurve::Linear), 1);
        assert_eq!(pad_velocity(32, PadVelocityCurve::Linear), 1);
        assert_eq!(pad_velocity(3200, PadVelocityCurve::Linear), 100);
        assert_eq!(pad_velocity(4095, PadVelocityCurve::Linear), 127);
        assert_eq!(pad_velocity(4096, PadVelocityCurve::Linear), 127);
    }

    #[test]
    fn soft_curves_raise_mid_velocities_in_order() {
        let linear = apply_pad_velocity_curve(64, PadVelocityCurve::Linear);
        let soft1 = apply_pad_velocity_curve(64, PadVelocityCurve::Soft1);
        let soft2 = apply_pad_velocity_curve(64, PadVelocityCurve::Soft2);
        let soft3 = apply_pad_velocity_curve(64, PadVelocityCurve::Soft3);

        assert!(soft1 > linear);
        assert!(soft2 > soft1);
        assert!(soft3 > soft2);
    }

    #[test]
    fn hard_curves_lower_mid_velocities_in_order() {
        let linear = apply_pad_velocity_curve(64, PadVelocityCurve::Linear);
        let hard1 = apply_pad_velocity_curve(64, PadVelocityCurve::Hard1);
        let hard2 = apply_pad_velocity_curve(64, PadVelocityCurve::Hard2);
        let hard3 = apply_pad_velocity_curve(64, PadVelocityCurve::Hard3);

        assert!(hard1 < linear);
        assert!(hard2 < hard1);
        assert!(hard3 < hard2);
    }

    #[test]
    fn curves_keep_zero_and_max_anchors() {
        for curve in [
            PadVelocityCurve::Soft3,
            PadVelocityCurve::Soft2,
            PadVelocityCurve::Soft1,
            PadVelocityCurve::Linear,
            PadVelocityCurve::Hard1,
            PadVelocityCurve::Hard2,
            PadVelocityCurve::Hard3,
        ] {
            assert_eq!(apply_pad_velocity_curve(0, curve), 0);
            assert_eq!(apply_pad_velocity_curve(127, curve), 127);
            assert!(apply_pad_velocity_curve(1, curve) >= 1);
        }
    }

    #[test]
    fn pad_velocity_curve_serializes_as_lowercase_string() {
        #[derive(serde::Serialize, serde::Deserialize)]
        struct Wrapper {
            curve: PadVelocityCurve,
        }

        let toml_str = toml::to_string(&Wrapper {
            curve: PadVelocityCurve::Hard2,
        })
        .unwrap();
        assert!(toml_str.contains("hard2"));

        let round_trip: Wrapper = toml::from_str("curve = \"hard2\"").unwrap();
        assert_eq!(round_trip.curve, PadVelocityCurve::Hard2);
    }
}
