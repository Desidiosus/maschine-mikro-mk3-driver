use std::collections::BTreeMap;

use maschine_library::controls::{BUTTON_NAMES, button_index_from_name};
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

use crate::settings::actions::{
    ButtonPressAction, EncoderTurnAction, PadHitAction, PadPressureAction, SliderLedMode,
    SliderPositionAction, SliderTouchAction,
};
use crate::settings::{BacklightBrightness, MidiChannel, Settings};
use crate::velocity::PadVelocityCurve;
use maschine_library::lights::PadColors;

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialSettings {
    pub global: Option<PartialGlobalSettings>,
    pub hardware: Option<PartialHardwareSettings>,
    pub bridge: Option<PartialBridgeSettings>,
    #[serde(
        deserialize_with = "deserialize_partial_pads",
        serialize_with = "serialize_partial_pads",
        default
    )]
    pub pads: Option<[Option<PartialPadConfig>; 16]>,
    #[serde(
        deserialize_with = "deserialize_partial_buttons",
        serialize_with = "serialize_partial_buttons",
        default
    )]
    pub buttons: Option<[Option<PartialButtonConfig>; 41]>,
    pub encoder: Option<PartialEncoderConfig>,
    pub slider: Option<PartialSliderConfig>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialGlobalSettings {
    pub midi_channel: Option<MidiChannel>,
    pub client_name: Option<String>,
    pub port_name: Option<String>,
    pub port_name_in: Option<String>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialHardwareSettings {
    pub pad_sensitivity: Option<u8>,
    pub display_contrast: Option<u8>,
    pub pad_velocity_curve: Option<PadVelocityCurve>,
    pub backlight_buttons: Option<bool>,
    pub backlight_brightness: Option<BacklightBrightness>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialBridgeSettings {
    pub midi_bridge_virmidi: Option<bool>,
    pub autoconnect_virmidi: Option<bool>,
    pub virmidi_client_name: Option<String>,
    pub virmidi_port: Option<usize>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialPadConfig {
    pub hit: Option<PadHitAction>,
    pub pressure: Option<PadPressureAction>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialButtonConfig {
    pub press: Option<ButtonPressAction>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialEncoderConfig {
    pub turn: Option<EncoderTurnAction>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialSliderConfig {
    pub position: Option<SliderPositionAction>,
    pub touch: Option<SliderTouchAction>,
    pub led: Option<PartialSliderLedSettings>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialSliderLedSettings {
    pub mode: Option<SliderLedMode>,
    pub color: Option<PadColors>,
    pub stylized: Option<bool>,
    pub auto_off_ms: Option<u64>,
}

fn deserialize_partial_pads<'de, D>(
    de: D,
) -> Result<Option<[Option<PartialPadConfig>; 16]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map: Option<BTreeMap<String, PartialPadConfig>> = Option::deserialize(de)?;
    let Some(map) = map else {
        return Ok(None);
    };
    let mut out: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
    for (key, cfg) in map {
        let idx: usize = key
            .parse()
            .map_err(|_| DeError::custom(format!("pad index must be an integer, got: {key}")))?;
        if idx >= 16 {
            return Err(DeError::custom(format!(
                "pad index {idx} out of range 0..=15"
            )));
        }
        out[idx] = Some(cfg);
    }
    Ok(Some(out))
}

fn serialize_partial_pads<S>(
    pads: &Option<[Option<PartialPadConfig>; 16]>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let Some(pads) = pads else {
        return serializer.serialize_none();
    };
    let count = pads.iter().filter(|p| p.is_some()).count();
    let mut map = serializer.serialize_map(Some(count))?;
    for (idx, pad) in pads.iter().enumerate() {
        if let Some(pad) = pad {
            map.serialize_entry(&idx, pad)?;
        }
    }
    map.end()
}

fn serialize_partial_buttons<S>(
    buttons: &Option<[Option<PartialButtonConfig>; 41]>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;
    let Some(buttons) = buttons else {
        return serializer.serialize_none();
    };
    let count = buttons.iter().filter(|b| b.is_some()).count();
    let mut map = serializer.serialize_map(Some(count))?;
    for (idx, btn) in buttons.iter().enumerate() {
        if let Some(btn) = btn {
            map.serialize_entry(BUTTON_NAMES[idx], btn)?;
        }
    }
    map.end()
}

fn deserialize_partial_buttons<'de, D>(
    de: D,
) -> Result<Option<[Option<PartialButtonConfig>; 41]>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let map: Option<BTreeMap<String, PartialButtonConfig>> = Option::deserialize(de)?;
    let Some(map) = map else {
        return Ok(None);
    };
    let mut out: [Option<PartialButtonConfig>; 41] = std::array::from_fn(|_| None);
    for (name, cfg) in map {
        let idx = button_index_from_name(&name)
            .ok_or_else(|| DeError::custom(format!("unknown button name: {name}")))?;
        out[idx] = Some(cfg);
    }
    Ok(Some(out))
}

impl Settings {
    pub fn merge_overrides(mut self, partial: PartialSettings) -> Self {
        if let Some(g) = partial.global {
            if let Some(v) = g.midi_channel {
                self.global.midi_channel = v;
            }
            if let Some(v) = g.client_name {
                self.global.client_name = v;
            }
            if let Some(v) = g.port_name {
                self.global.port_name = v;
            }
            if let Some(v) = g.port_name_in {
                self.global.port_name_in = v;
            }
        }
        if let Some(h) = partial.hardware {
            if let Some(v) = h.pad_sensitivity {
                self.hardware.pad_sensitivity = v;
            }
            if let Some(v) = h.display_contrast {
                self.hardware.display_contrast = v;
            }
            if let Some(v) = h.pad_velocity_curve {
                self.hardware.pad_velocity_curve = v;
            }
            if let Some(v) = h.backlight_buttons {
                self.hardware.backlight_buttons = v;
            }
            if let Some(v) = h.backlight_brightness {
                self.hardware.backlight_brightness = v;
            }
        }
        if let Some(b) = partial.bridge {
            if let Some(v) = b.midi_bridge_virmidi {
                self.bridge.midi_bridge_virmidi = v;
            }
            if let Some(v) = b.autoconnect_virmidi {
                self.bridge.autoconnect_virmidi = v;
            }
            if let Some(v) = b.virmidi_client_name {
                self.bridge.virmidi_client_name = v;
            }
            if let Some(v) = b.virmidi_port {
                self.bridge.virmidi_port = v;
            }
        }
        if let Some(pads) = partial.pads {
            for (idx, cfg) in pads.into_iter().enumerate() {
                let Some(cfg) = cfg else { continue };
                if let Some(hit) = cfg.hit {
                    self.pads[idx].hit = hit;
                }
                if let Some(pressure) = cfg.pressure {
                    self.pads[idx].pressure = pressure;
                }
            }
        }
        if let Some(buttons) = partial.buttons {
            for (idx, cfg) in buttons.into_iter().enumerate() {
                let Some(cfg) = cfg else { continue };
                if let Some(press) = cfg.press {
                    self.buttons[idx].press = press;
                }
            }
        }
        if let Some(e) = partial.encoder
            && let Some(v) = e.turn
        {
            self.encoder.turn = v;
        }
        if let Some(s) = partial.slider {
            if let Some(v) = s.position {
                self.slider.position = v;
            }
            if let Some(v) = s.touch {
                self.slider.touch = v;
            }
            if let Some(led) = s.led {
                if let Some(v) = led.mode {
                    self.slider.led.mode = v;
                }
                if let Some(v) = led.color {
                    self.slider.led.color = v;
                }
                if let Some(v) = led.stylized {
                    self.slider.led.stylized = v;
                }
                if let Some(v) = led.auto_off_ms {
                    self.slider.led.auto_off_ms = v;
                }
            }
        }
        self
    }

    pub fn diff_from_defaults(&self) -> PartialSettings {
        let defaults = Settings::default();
        let mut out = PartialSettings::default();

        let mut g = PartialGlobalSettings::default();
        if self.global.midi_channel != defaults.global.midi_channel {
            g.midi_channel = Some(self.global.midi_channel);
        }
        if self.global.client_name != defaults.global.client_name {
            g.client_name = Some(self.global.client_name.clone());
        }
        if self.global.port_name != defaults.global.port_name {
            g.port_name = Some(self.global.port_name.clone());
        }
        if self.global.port_name_in != defaults.global.port_name_in {
            g.port_name_in = Some(self.global.port_name_in.clone());
        }
        if g != PartialGlobalSettings::default() {
            out.global = Some(g);
        }

        let mut h = PartialHardwareSettings::default();
        if self.hardware.pad_sensitivity != defaults.hardware.pad_sensitivity {
            h.pad_sensitivity = Some(self.hardware.pad_sensitivity);
        }
        if self.hardware.display_contrast != defaults.hardware.display_contrast {
            h.display_contrast = Some(self.hardware.display_contrast);
        }
        if self.hardware.pad_velocity_curve != defaults.hardware.pad_velocity_curve {
            h.pad_velocity_curve = Some(self.hardware.pad_velocity_curve);
        }
        if self.hardware.backlight_buttons != defaults.hardware.backlight_buttons {
            h.backlight_buttons = Some(self.hardware.backlight_buttons);
        }
        if self.hardware.backlight_brightness != defaults.hardware.backlight_brightness {
            h.backlight_brightness = Some(self.hardware.backlight_brightness);
        }
        if h != PartialHardwareSettings::default() {
            out.hardware = Some(h);
        }

        let mut b = PartialBridgeSettings::default();
        if self.bridge.midi_bridge_virmidi != defaults.bridge.midi_bridge_virmidi {
            b.midi_bridge_virmidi = Some(self.bridge.midi_bridge_virmidi);
        }
        if self.bridge.autoconnect_virmidi != defaults.bridge.autoconnect_virmidi {
            b.autoconnect_virmidi = Some(self.bridge.autoconnect_virmidi);
        }
        if self.bridge.virmidi_client_name != defaults.bridge.virmidi_client_name {
            b.virmidi_client_name = Some(self.bridge.virmidi_client_name.clone());
        }
        if self.bridge.virmidi_port != defaults.bridge.virmidi_port {
            b.virmidi_port = Some(self.bridge.virmidi_port);
        }
        if b != PartialBridgeSettings::default() {
            out.bridge = Some(b);
        }

        let mut pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        let mut any_pad = false;
        for (idx, pad) in self.pads.iter().enumerate() {
            let mut p = PartialPadConfig::default();
            if pad.hit != defaults.pads[idx].hit {
                p.hit = Some(pad.hit.clone());
            }
            if pad.pressure != defaults.pads[idx].pressure {
                p.pressure = Some(pad.pressure.clone());
            }
            if p != PartialPadConfig::default() {
                pads[idx] = Some(p);
                any_pad = true;
            }
        }
        if any_pad {
            out.pads = Some(pads);
        }

        let mut buttons: [Option<PartialButtonConfig>; 41] = std::array::from_fn(|_| None);
        let mut any_button = false;
        for (idx, slot) in buttons.iter_mut().enumerate() {
            if self.buttons[idx].press != defaults.buttons[idx].press {
                *slot = Some(PartialButtonConfig {
                    press: Some(self.buttons[idx].press.clone()),
                });
                any_button = true;
            }
        }
        if any_button {
            out.buttons = Some(buttons);
        }

        if self.encoder.turn != defaults.encoder.turn {
            out.encoder = Some(PartialEncoderConfig {
                turn: Some(self.encoder.turn.clone()),
            });
        }

        let mut s = PartialSliderConfig::default();
        if self.slider.position != defaults.slider.position {
            s.position = Some(self.slider.position.clone());
        }
        if self.slider.touch != defaults.slider.touch {
            s.touch = Some(self.slider.touch.clone());
        }
        let mut led = PartialSliderLedSettings::default();
        if self.slider.led.mode != defaults.slider.led.mode {
            led.mode = Some(self.slider.led.mode);
        }
        if self.slider.led.color != defaults.slider.led.color {
            led.color = Some(self.slider.led.color);
        }
        if self.slider.led.stylized != defaults.slider.led.stylized {
            led.stylized = Some(self.slider.led.stylized);
        }
        if self.slider.led.auto_off_ms != defaults.slider.led.auto_off_ms {
            led.auto_off_ms = Some(self.slider.led.auto_off_ms);
        }
        if led != PartialSliderLedSettings::default() {
            s.led = Some(led);
        }
        if s != PartialSliderConfig::default() {
            out.slider = Some(s);
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::actions::{ButtonPressAction, PadPressureAction, SliderTouchAction};
    use crate::settings::{MidiChannel, Settings};

    #[test]
    fn empty_partial_merges_to_default() {
        let merged = Settings::default().merge_overrides(PartialSettings::default());
        assert_eq!(merged, Settings::default());
    }

    #[test]
    fn partial_overrides_single_button_cc() {
        use maschine_library::controls::Buttons;

        let toml_str = r#"
[buttons.play.press]
type = "cc"
cc = 99
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        match &merged.buttons[Buttons::Play as usize].press {
            ButtonPressAction::Cc { cc, .. } => assert_eq!(*cc, 99),
        }
        match &merged.buttons[Buttons::Stop as usize].press {
            ButtonPressAction::Cc { cc, .. } => assert_eq!(*cc, 44),
        }
    }

    #[test]
    fn partial_enables_pad_pressure_on_one_pad_only() {
        let toml_str = r#"
[pads.5.pressure]
type = "poly"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(
            merged.pads[5].pressure,
            PadPressureAction::Poly {
                channel: None,
                note: None
            }
        );
        assert_eq!(merged.pads[0].pressure, PadPressureAction::Disabled);
    }

    #[test]
    fn partial_enables_slider_touch_as_cc() {
        let toml_str = r#"
[slider.touch]
type = "cc"
cc = 70
on_value = 127
off_value = 0
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(
            merged.slider.touch,
            SliderTouchAction::Cc {
                channel: None,
                cc: 70,
                on_value: 127,
                off_value: 0
            }
        );
    }

    #[test]
    fn partial_overrides_global_midi_channel_only() {
        let toml_str = r#"
[global]
midi_channel = 5
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(
            merged.global.midi_channel,
            MidiChannel::try_from(5).unwrap()
        );
        assert_eq!(merged.global.client_name, "Maschine Mikro MK3");
    }

    #[test]
    fn diff_from_defaults_is_inverse_of_merge_overrides() {
        let mut s = Settings::default();
        s.global.midi_channel = MidiChannel::try_from(3).unwrap();
        s.pads[2].pressure = PadPressureAction::Poly {
            channel: None,
            note: Some(60),
        };
        s.slider.touch = SliderTouchAction::Cc {
            channel: None,
            cc: 70,
            on_value: 100,
            off_value: 10,
        };

        let partial = s.diff_from_defaults();
        let round_tripped = Settings::default().merge_overrides(partial);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn partial_overrides_slider_led_mode_only() {
        let toml_str = r#"
[slider.led]
mode = "pan"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(
            merged.slider.led.mode,
            crate::settings::actions::SliderLedMode::Pan
        );
        assert_eq!(
            merged.slider.led.color,
            maschine_library::lights::PadColors::White
        );
        assert!(!merged.slider.led.stylized);
    }

    #[test]
    fn partial_overrides_slider_led_color_and_stylized() {
        let toml_str = r#"
[slider.led]
color = "cyan"
stylized = true
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(
            merged.slider.led.color,
            maschine_library::lights::PadColors::Cyan
        );
        assert!(merged.slider.led.stylized);
        assert_eq!(
            merged.slider.led.mode,
            crate::settings::actions::SliderLedMode::Bar
        );
    }

    #[test]
    fn diff_from_defaults_emits_slider_led_overrides() {
        use crate::settings::actions::SliderLedMode;
        let mut s = Settings::default();
        s.slider.led.mode = SliderLedMode::Dot;
        s.slider.led.stylized = true;

        let partial = s.diff_from_defaults();
        let round_tripped = Settings::default().merge_overrides(partial);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn partial_overrides_slider_led_auto_off_ms() {
        let toml_str = r#"
[slider.led]
auto_off_ms = 0
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(merged.slider.led.auto_off_ms, 0);
        assert_eq!(
            merged.slider.led.mode,
            crate::settings::actions::SliderLedMode::Bar
        );
    }
}
