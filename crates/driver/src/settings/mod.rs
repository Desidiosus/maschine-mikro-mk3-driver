use serde::{Deserialize, Serialize};

pub mod actions;
pub mod buttons_by_name;
pub mod defaults;
pub mod groups;
pub mod pads_by_index;
pub mod partial;
pub use partial::PartialSettings;

pub use actions::{
    ButtonConfig, ButtonPressAction, CcValueMode, EncoderConfig, EncoderTurnAction, PadConfig,
    PadHitAction, PadPressureAction, SliderConfig, SliderLedMode, SliderLedSettings,
    SliderPositionAction, SliderTouchAction,
};
pub use buttons_by_name::ButtonsByName;
pub use groups::{BridgeSettings, GlobalSettings, HardwareSettings};
pub use pads_by_index::PadsByIndex;

#[derive(Default, Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum BacklightBrightness {
    #[default]
    Dim,
    Normal,
    Bright,
}

impl BacklightBrightness {
    pub fn as_light_brightness(self) -> maschine_library::lights::Brightness {
        match self {
            Self::Dim => maschine_library::lights::Brightness::Dim,
            Self::Normal => maschine_library::lights::Brightness::Normal,
            Self::Bright => maschine_library::lights::Brightness::Bright,
        }
    }
}

#[derive(Default, Deserialize, Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(try_from = "u8", into = "u8")]
pub struct MidiChannel(u8);

impl MidiChannel {
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for MidiChannel {
    type Error = String;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        if value <= 15 {
            Ok(Self(value))
        } else {
            Err(format!(
                "invalid midi_channel={value} (expected integer in range 0..=15)"
            ))
        }
    }
}

impl From<MidiChannel> for u8 {
    fn from(value: MidiChannel) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub global: GlobalSettings,
    pub hardware: HardwareSettings,
    pub bridge: BridgeSettings,
    pub pads: PadsByIndex,
    pub buttons: ButtonsByName,
    pub encoder: EncoderConfig,
    pub slider: SliderConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            global: GlobalSettings::default(),
            hardware: HardwareSettings::default(),
            bridge: BridgeSettings::default(),
            pads: defaults::default_pads(),
            buttons: defaults::default_buttons(),
            encoder: defaults::default_encoder(),
            slider: defaults::default_slider(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if self.hardware.pad_sensitivity > 100 {
            return Err("pad_sensitivity must be in range 0..=100".to_string());
        }
        if self.hardware.display_contrast > 100 {
            return Err("display_contrast must be in range 0..=100".to_string());
        }
        if self.global.client_name.is_empty() {
            return Err("client_name must not be empty".to_string());
        }
        if self.global.port_name.is_empty() {
            return Err("port_name must not be empty".to_string());
        }
        if self.global.port_name_in.is_empty() {
            return Err("port_name_in must not be empty".to_string());
        }
        Ok(())
    }
}
