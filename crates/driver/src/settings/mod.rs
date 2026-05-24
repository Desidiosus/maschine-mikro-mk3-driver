pub mod actions;

use serde::{Deserialize, Deserializer, Serialize};

use crate::velocity::PadVelocityCurve;

const PAD_COUNT: usize = 16;
const BUTTON_COUNT: usize = 41;

fn deserialize_button_ccs<'de, D>(de: D) -> Result<[u8; BUTTON_COUNT], D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<u8>::deserialize(de)?;
    let count = values.len();
    values.try_into().map_err(|_: Vec<u8>| {
        serde::de::Error::custom(format!(
            "There should be {BUTTON_COUNT} button CC mappings exactly (found {count})"
        ))
    })
}

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

impl From<MidiChannel> for u8 {
    fn from(value: MidiChannel) -> Self {
        value.0
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

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct MidiMapping {
    #[serde(default, rename = "midi_channel", alias = "channel")]
    pub channel: MidiChannel,
    #[serde(default = "default_pad_notes", alias = "notemaps")]
    pub pad_notes: [u8; PAD_COUNT],
    #[serde(
        default = "default_button_ccs",
        deserialize_with = "deserialize_button_ccs"
    )]
    pub button_ccs: [u8; BUTTON_COUNT],
    #[serde(default = "default_encoder_cc")]
    pub encoder_cc: u8,
    #[serde(default = "default_slider_cc")]
    pub slider_cc: u8,
}

impl Default for MidiMapping {
    fn default() -> Self {
        Self {
            channel: MidiChannel::default(),
            pad_notes: default_pad_notes(),
            button_ccs: default_button_ccs(),
            encoder_cc: default_encoder_cc(),
            slider_cc: default_slider_cc(),
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct Settings {
    pub midi_bridge_virmidi: bool,
    #[serde(flatten)]
    pub midi: MidiMapping,
    pub client_name: String,
    pub port_name: String,
    pub port_name_in: String,
    /// If true, treat "LED Off" for buttons as a low backlight instead.
    /// Useful as a "night mode" so you can see buttons in the dark.
    pub backlight_buttons: bool,
    /// Backlight level for buttons when `backlight_buttons = true`.
    pub backlight_brightness: BacklightBrightness,
    /// If true, try to connect the driver's ALSA sequencer ports to a kernel rawmidi
    /// device exposed via snd-virmidi (what Bitwig enumerates as "Virtual Raw MIDI ...").
    pub autoconnect_virmidi: bool,
    /// ALSA sequencer client name for the rawmidi bridge, e.g. "Virtual Raw MIDI 1-0".
    /// If empty, the first client starting with "Virtual Raw MIDI" will be used.
    pub virmidi_client_name: String,
    /// Port number on the virmidi client (usually 0).
    pub virmidi_port: usize,
    pub pad_sensitivity: u8,
    pub display_contrast: u8,
    pub pad_velocity_curve: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            midi_bridge_virmidi: false,
            midi: MidiMapping::default(),
            client_name: "Maschine Mikro MK3".to_string(),
            port_name: "Maschine Mikro MK3 MIDI Out".to_string(),
            port_name_in: "Maschine Mikro MK3 MIDI In".to_string(),
            backlight_buttons: true,
            backlight_brightness: BacklightBrightness::Dim,
            autoconnect_virmidi: true,
            virmidi_client_name: "".to_string(),
            virmidi_port: 0,
            pad_sensitivity: 50,
            display_contrast: 50,
            pad_velocity_curve: "linear".to_string(),
        }
    }
}

impl Settings {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.midi.pad_notes.iter().any(|x| *x >= 128) {
            return Err("MIDI notes should be 0 to 127".to_string());
        }

        if self.midi.button_ccs.iter().any(|x| *x >= 128) {
            return Err("Button CC values should be 0 to 127".to_string());
        }

        if self.midi.encoder_cc >= 128 {
            return Err("Encoder CC value should be 0 to 127".to_string());
        }

        if self.midi.slider_cc >= 128 {
            return Err("Slider CC value should be 0 to 127".to_string());
        }

        if self.client_name.is_empty() {
            return Err("Client name must not be empty".to_string());
        }

        if self.port_name.is_empty() {
            return Err("Port name must not be empty".to_string());
        }

        if self.port_name_in.is_empty() {
            return Err("Input port name must not be empty".to_string());
        }

        if self.pad_sensitivity > 100 {
            return Err("pad_sensitivity must be in range 0..=100".to_string());
        }

        if self.display_contrast > 100 {
            return Err("display_contrast must be in range 0..=100".to_string());
        }

        self.pad_velocity_curve()?;

        Ok(())
    }

    pub(crate) fn pad_velocity_curve(&self) -> Result<PadVelocityCurve, String> {
        crate::velocity::parse_pad_velocity_curve_setting(&self.pad_velocity_curve)
    }
}

const fn default_pad_notes() -> [u8; PAD_COUNT] {
    [
        48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39,
    ]
}

const fn default_button_ccs() -> [u8; BUTTON_COUNT] {
    [
        20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42,
        43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 58, 59, 60,
    ]
}

const fn default_encoder_cc() -> u8 {
    1
}

const fn default_slider_cc() -> u8 {
    9
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use crate::velocity::PadVelocityCurve;
    use config::{Config, File, FileFormat};

    fn load_settings(src: &str) -> crate::settings::Settings {
        Config::builder()
            .add_source(File::from_str(src, FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn settings_still_accept_legacy_notemaps_field() {
        let settings = load_settings(
            r#"
notemaps = [36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51]
"#,
        );

        assert_eq!(settings.midi.pad_notes[0], 36);
        assert_eq!(settings.midi.pad_notes[15], 51);
    }

    #[test]
    fn rejects_button_ccs_with_wrong_length() {
        let cfg = Config::builder()
            .add_source(File::from_str("button_ccs = [1, 2, 3]", FileFormat::Toml))
            .build()
            .unwrap();
        let err = cfg
            .try_deserialize::<crate::settings::Settings>()
            .unwrap_err();
        assert!(err.to_string().contains("button CC mappings"));
    }

    #[test]
    fn rejects_pad_sensitivity_above_100() {
        let settings = Settings {
            pad_sensitivity: 101,
            ..Settings::default()
        };

        let err = settings
            .validate()
            .expect_err("pad_sensitivity should be rejected");

        assert!(err.contains("pad_sensitivity"));
    }

    #[test]
    fn rejects_display_contrast_above_100() {
        let settings = Settings {
            display_contrast: 101,
            ..Settings::default()
        };

        let err = settings
            .validate()
            .expect_err("display_contrast should be rejected");

        assert!(err.contains("display_contrast"));
    }

    #[test]
    fn rejects_invalid_pad_velocity_curve() {
        let settings = Settings {
            pad_velocity_curve: "flat".to_string(),
            ..Settings::default()
        };

        assert_eq!(
            settings
                .validate()
                .expect_err("pad_velocity_curve should be rejected"),
            "invalid pad_velocity_curve=\"flat\" (expected one of: soft3, soft2, soft1, linear, hard1, hard2, hard3)"
        );
    }

    #[test]
    fn pad_velocity_curve_helper_uses_shared_parser() {
        let settings = Settings {
            pad_velocity_curve: "Hard 2".to_string(),
            ..Settings::default()
        };

        assert_eq!(
            settings.pad_velocity_curve().unwrap(),
            PadVelocityCurve::Hard2
        );
    }

    #[test]
    fn midi_channel_round_trips_via_toml_as_integer() {
        use crate::settings::MidiChannel;

        let channel = MidiChannel::try_from(7).unwrap();
        let toml_value = toml::Value::try_from(channel).unwrap();
        assert_eq!(toml_value, toml::Value::Integer(7));

        let parsed: MidiChannel = toml::Value::Integer(7).try_into().unwrap();
        assert_eq!(parsed.as_u8(), 7);
    }
}
