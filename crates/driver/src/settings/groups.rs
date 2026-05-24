use serde::{Deserialize, Serialize};

use crate::settings::{BacklightBrightness, MidiChannel};
use crate::velocity::PadVelocityCurve;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalSettings {
    pub midi_channel: MidiChannel,
    pub client_name: String,
    pub port_name: String,
    pub port_name_in: String,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            midi_channel: MidiChannel::default(),
            client_name: "Maschine Mikro MK3".to_string(),
            port_name: "Maschine Mikro MK3 MIDI Out".to_string(),
            port_name_in: "Maschine Mikro MK3 MIDI In".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareSettings {
    pub pad_sensitivity: u8,
    pub display_contrast: u8,
    pub pad_velocity_curve: PadVelocityCurve,
    pub backlight_buttons: bool,
    pub backlight_brightness: BacklightBrightness,
}

impl Default for HardwareSettings {
    fn default() -> Self {
        Self {
            pad_sensitivity: 50,
            display_contrast: 50,
            pad_velocity_curve: PadVelocityCurve::Linear,
            backlight_buttons: true,
            backlight_brightness: BacklightBrightness::Dim,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSettings {
    pub midi_bridge_virmidi: bool,
    pub autoconnect_virmidi: bool,
    pub virmidi_client_name: String,
    pub virmidi_port: usize,
}

impl Default for BridgeSettings {
    fn default() -> Self {
        Self {
            midi_bridge_virmidi: false,
            autoconnect_virmidi: true,
            virmidi_client_name: String::new(),
            virmidi_port: 0,
        }
    }
}
