use serde::{Deserialize, Serialize};
use settings::{PartialSettings, Settings};

/// Messages sent from the GUI client to the driver.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuiToDriver {
    /// Request a full `Settings` snapshot.
    GetSettings,
    /// Apply a sparse settings delta. `seq` correlates the matching `Ack`.
    Apply {
        seq: u64,
        delta: Box<PartialSettings>,
    },
    /// Opt in to the `ControlActuated` / `MidiActivity` event stream.
    SubscribeEvents,
}

/// Messages sent from the driver to the GUI client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriverToGui {
    /// Full snapshot: sent on connect and after each successful apply.
    Settings(Box<Settings>),
    /// Result for the `Apply` carrying the matching `seq`.
    Ack {
        seq: u64,
        result: Result<(), String>,
    },
    /// A control was actuated on the hardware (drives Touch-Select).
    ControlActuated { control: ControlRef },
    /// MIDI traffic occurred in the given direction (drives the In/Out indicator).
    MidiActivity { dir: MidiDir },
}

/// Identifies a single control on the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRef {
    /// Internal pad index, `0..=15`.
    Pad(u8),
    /// Button index, `0..=40` — see `maschine_library::controls::BUTTON_NAMES`.
    Button(u8),
    Encoder,
    Slider,
}

/// MIDI traffic direction for the activity indicator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MidiDir {
    In,
    Out,
}

#[cfg(test)]
mod tests {
    use super::*;
    use settings::PadPressureAction;
    use settings::partial::PartialPadConfig;

    fn cbor_round_trip<T>(value: &T) -> T
    where
        T: Serialize + serde::de::DeserializeOwned,
    {
        let mut bytes = Vec::new();
        ciborium::into_writer(value, &mut bytes).expect("serialize");
        ciborium::from_reader(&bytes[..]).expect("deserialize")
    }

    #[test]
    fn apply_with_multi_pad_delta_round_trips() {
        let mut pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        pads[2] = Some(PartialPadConfig {
            hit: None,
            pressure: Some(PadPressureAction::Poly {
                channel: None,
                note: Some(60),
            }),
        });
        pads[7] = Some(PartialPadConfig {
            hit: None,
            pressure: Some(PadPressureAction::Disabled),
        });
        let delta = PartialSettings {
            pads: Some(pads),
            ..Default::default()
        };
        let msg = GuiToDriver::Apply {
            seq: 42,
            delta: Box::new(delta),
        };
        assert_eq!(cbor_round_trip(&msg), msg);
    }

    #[test]
    fn settings_snapshot_round_trips() {
        let msg = DriverToGui::Settings(Box::default());
        assert_eq!(cbor_round_trip(&msg), msg);
    }

    #[test]
    fn ack_ok_and_err_round_trip() {
        let ok = DriverToGui::Ack {
            seq: 1,
            result: Ok(()),
        };
        let err = DriverToGui::Ack {
            seq: 2,
            result: Err("bad value".to_string()),
        };
        assert_eq!(cbor_round_trip(&ok), ok);
        assert_eq!(cbor_round_trip(&err), err);
    }

    #[test]
    fn control_actuated_variants_round_trip() {
        for control in [
            ControlRef::Pad(15),
            ControlRef::Button(40),
            ControlRef::Encoder,
            ControlRef::Slider,
        ] {
            let msg = DriverToGui::ControlActuated { control };
            assert_eq!(cbor_round_trip(&msg), msg);
        }
    }

    #[test]
    fn midi_activity_round_trips() {
        for dir in [MidiDir::In, MidiDir::Out] {
            let msg = DriverToGui::MidiActivity { dir };
            assert_eq!(cbor_round_trip(&msg), msg);
        }
    }
}
