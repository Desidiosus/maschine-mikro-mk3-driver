use std::collections::BTreeMap;

use maschine_library::controls::{BUTTON_NAMES, button_index_from_name};
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ButtonPressAction, EncoderTurnAction, PadHitAction, PadLedColorMode, PadLedSource,
    PadPressureAction, SliderLedMode, SliderPositionAction, SliderTouchAction,
};
use crate::velocity_curve::PadVelocityCurve;
use crate::{MidiChannel, Settings};
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
    /// Deprecated and ignored. Private (not part of the builder surface): it
    /// exists only so a legacy `[global] midi_channel` key still parses under
    /// `deny_unknown_fields` rather than failing the load. Per-control channels
    /// are authoritative; an unset channel resolves to channel 1. Never read or
    /// merged, so it is dropped on the next persisted diff.
    #[serde(skip_serializing)]
    midi_channel: Option<MidiChannel>,
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
    pub led_brightness: Option<u8>,
    /// Deprecated and ignored. Private absorber fields for the old on/off flag and
    /// string-typed brightness preset that `led_brightness` replaced: they exist
    /// only so a legacy `[hardware]` config still parses under `deny_unknown_fields`
    /// rather than failing the load. Never read or merged, so they are dropped on
    /// the next persisted diff.
    #[serde(skip_serializing)]
    backlight_buttons: Option<bool>,
    #[serde(skip_serializing)]
    backlight_brightness: Option<String>,
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
    pub led: Option<PartialPadLedConfig>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialPadLedConfig {
    pub source: Option<PadLedSource>,
    pub midi_in: Option<PadLedColorMode>,
    pub midi_out: Option<PadLedColorMode>,
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
        let config_key: usize = key
            .parse()
            .map_err(|_| DeError::custom(format!("pad index must be an integer, got: {key}")))?;
        if !(1..=16).contains(&config_key) {
            return Err(DeError::custom(format!(
                "pad index {config_key} out of range 1..=16"
            )));
        }
        let internal = crate::pads_by_index::config_key_to_internal(config_key);
        // Distinct string keys can normalize to the same index (e.g. "1" and
        // "01"); reject the collision instead of silently last-write-wins, matching
        // the full-config `PadsByIndex` decoder.
        if out[internal].is_some() {
            return Err(DeError::custom(format!("duplicate pad key {config_key}")));
        }
        out[internal] = Some(cfg);
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
    for (internal, pad) in pads.iter().enumerate() {
        if let Some(pad) = pad {
            let config_key = crate::pads_by_index::internal_to_config_key(internal);
            map.serialize_entry(&config_key.to_string(), pad)?;
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

/// For each named field, assign `partial.field` into `target.field` when set.
macro_rules! apply_overrides {
    ($target:expr, $partial:expr; $($field:ident),+ $(,)?) => {
        $(
            if let Some(v) = $partial.$field {
                $target.$field = v;
            }
        )+
    };
}

/// Compare each named field of `cur` against `def`; for any that differ, store
/// `Some(cur.field.clone())` (or `Some(cur.field)` for Copy types) into `out`.
/// Two arms split Copy vs Clone fields to avoid `clippy::clone_on_copy` noise.
macro_rules! diff_section {
    (
        $cur:expr, $def:expr, $out:expr;
        copy: { $($cf:ident),* $(,)? } $(;)?
        clone: { $($lf:ident),* $(,)? } $(;)?
    ) => {
        $(
            if $cur.$cf != $def.$cf {
                $out.$cf = Some($cur.$cf);
            }
        )*
        $(
            if $cur.$lf != $def.$lf {
                $out.$lf = Some($cur.$lf.clone());
            }
        )*
    };
}

impl Settings {
    pub fn merge_overrides(mut self, partial: PartialSettings) -> Self {
        if let Some(g) = partial.global {
            apply_overrides!(self.global, g; client_name, port_name, port_name_in);
        }
        if let Some(h) = partial.hardware {
            apply_overrides!(
                self.hardware, h;
                pad_sensitivity,
                display_contrast,
                pad_velocity_curve,
                led_brightness,
            );
        }
        if let Some(b) = partial.bridge {
            apply_overrides!(
                self.bridge, b;
                midi_bridge_virmidi,
                autoconnect_virmidi,
                virmidi_client_name,
                virmidi_port,
            );
        }
        if let Some(pads) = partial.pads {
            for (idx, cfg) in pads.into_iter().enumerate() {
                let Some(cfg) = cfg else { continue };
                apply_overrides!(self.pads[idx], cfg; hit, pressure);
                if let Some(led) = cfg.led {
                    apply_overrides!(self.pads[idx].led, led; source, midi_in, midi_out);
                }
            }
        }
        if let Some(buttons) = partial.buttons {
            for (idx, cfg) in buttons.into_iter().enumerate() {
                let Some(cfg) = cfg else { continue };
                apply_overrides!(self.buttons[idx], cfg; press);
            }
        }
        if let Some(e) = partial.encoder {
            apply_overrides!(self.encoder, e; turn);
        }
        if let Some(s) = partial.slider {
            apply_overrides!(self.slider, s; position, touch);
            if let Some(led) = s.led {
                apply_overrides!(self.slider.led, led; mode, color, stylized, auto_off_ms);
            }
        }
        self
    }

    /// Sparse overrides of `self` relative to `Settings::default()`.
    pub fn diff_from_defaults(&self) -> PartialSettings {
        self.diff_from(&Settings::default())
    }

    /// Sparse overrides of `self` relative to an arbitrary `base`. Used to
    /// persist only the GUI-made changes layered on top of a read-only `-c`
    /// seed (`base = defaults ∘ -c`), so the seed shows through for untouched
    /// keys. `diff_from(&Settings::default())` is `diff_from_defaults`.
    pub fn diff_from(&self, base: &Settings) -> PartialSettings {
        let mut out = PartialSettings::default();

        let mut g = PartialGlobalSettings::default();
        diff_section!(
            self.global, base.global, g;
            copy: {};
            clone: { client_name, port_name, port_name_in };
        );
        if g != PartialGlobalSettings::default() {
            out.global = Some(g);
        }

        let mut h = PartialHardwareSettings::default();
        diff_section!(
            self.hardware, base.hardware, h;
            copy: {
                pad_sensitivity,
                display_contrast,
                pad_velocity_curve,
                led_brightness,
            };
            clone: {};
        );
        if h != PartialHardwareSettings::default() {
            out.hardware = Some(h);
        }

        let mut b = PartialBridgeSettings::default();
        diff_section!(
            self.bridge, base.bridge, b;
            copy: { midi_bridge_virmidi, autoconnect_virmidi, virmidi_port };
            clone: { virmidi_client_name };
        );
        if b != PartialBridgeSettings::default() {
            out.bridge = Some(b);
        }

        let mut pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        let mut any_pad = false;
        for (idx, pad) in self.pads.iter().enumerate() {
            let mut p = PartialPadConfig::default();
            diff_section!(
                pad, base.pads[idx], p;
                copy: {};
                clone: { hit, pressure };
            );
            let mut led = PartialPadLedConfig::default();
            diff_section!(
                pad.led, base.pads[idx].led, led;
                copy: { source, midi_in, midi_out };
                clone: {};
            );
            if led != PartialPadLedConfig::default() {
                p.led = Some(led);
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
            if self.buttons[idx].press != base.buttons[idx].press {
                *slot = Some(PartialButtonConfig {
                    press: Some(self.buttons[idx].press.clone()),
                });
                any_button = true;
            }
        }
        if any_button {
            out.buttons = Some(buttons);
        }

        if self.encoder.turn != base.encoder.turn {
            out.encoder = Some(PartialEncoderConfig {
                turn: Some(self.encoder.turn.clone()),
            });
        }

        let mut s = PartialSliderConfig::default();
        diff_section!(
            self.slider, base.slider, s;
            copy: {};
            clone: { position, touch };
        );
        let mut led = PartialSliderLedSettings::default();
        diff_section!(
            self.slider.led, base.slider.led, led;
            copy: { mode, color, stylized, auto_off_ms };
            clone: {};
        );
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
    use crate::Settings;
    use crate::actions::{ButtonPressAction, PadPressureAction, SliderTouchAction};

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
            ButtonPressAction::Off => panic!("expected cc"),
        }
        match &merged.buttons[Buttons::Stop as usize].press {
            ButtonPressAction::Cc { cc, .. } => assert_eq!(*cc, 44),
            ButtonPressAction::Off => panic!("expected cc"),
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

        // TOML key 5 (physical pad 5) maps to internal logical pad 8 (row-flip).
        assert_eq!(
            merged.pads[8].pressure,
            PadPressureAction::Poly {
                channel: None,
                note: None
            }
        );
        assert_eq!(merged.pads[5].pressure, PadPressureAction::Disabled);
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
    fn unknown_hardware_key_is_rejected() {
        // A typo'd key must fail the load rather than being silently dropped, so a
        // mistyped override surfaces as an error instead of quietly doing nothing.
        let err = toml::from_str::<PartialSettings>("[hardware]\npad_sensitvity = 90\n")
            .expect_err("typo'd key must be rejected");
        assert!(err.to_string().contains("pad_sensitvity"), "got: {err}");
    }

    #[test]
    fn legacy_backlight_keys_are_accepted_and_ignored() {
        // The removed on/off flag and renamed brightness preset still parse (absorbed
        // by deprecated fields) so an older config loads, but never affect settings.
        let partial: PartialSettings = toml::from_str(
            "[hardware]\nbacklight_buttons = false\nbacklight_brightness = \"dim\"\nled_brightness = 7\n",
        )
        .expect("legacy keys still parse");
        let merged = Settings::default().merge_overrides(partial);
        assert_eq!(merged.hardware.led_brightness, 7);
    }

    #[test]
    fn legacy_global_midi_channel_is_accepted_and_ignored() {
        let toml_str = r#"
[global]
midi_channel = 5
client_name = "Custom"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).expect("legacy key still parses");
        let merged = Settings::default().merge_overrides(partial);
        // The deprecated channel is ignored; other global fields still apply.
        assert_eq!(merged.global.client_name, "Custom");
    }

    #[test]
    fn diff_from_defaults_is_inverse_of_merge_overrides() {
        let mut s = Settings::default();
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
    fn diff_from_base_captures_only_changes_relative_to_base() {
        // base = defaults with a custom global field (stands in for a `-c` seed).
        let mut base = Settings::default();
        base.global.client_name = "Seeded".to_string();

        // live = base plus one pad-note change. The base's client_name must NOT
        // appear in the diff (it's part of the seed, not a GUI edit).
        let mut live = base.clone();
        live.pads[2].hit = PadHitAction::Note {
            channel: None,
            note: 61,
        };

        let diff = live.diff_from(&base);
        assert!(diff.global.is_none(), "unchanged-vs-base global is omitted");
        assert!(diff.pads.is_some(), "changed pad is captured");
        // Applying the diff onto the base reconstructs the live settings.
        assert_eq!(base.merge_overrides(diff), live);
    }

    #[test]
    fn partial_overrides_slider_led_mode_only() {
        let toml_str = r#"
[slider.led]
mode = "pan"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);

        assert_eq!(merged.slider.led.mode, SliderLedMode::Pan);
        assert_eq!(merged.slider.led.color, PadColors::White);
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

        assert_eq!(merged.slider.led.color, PadColors::Cyan);
        assert!(merged.slider.led.stylized);
        assert_eq!(merged.slider.led.mode, SliderLedMode::Bar);
    }

    #[test]
    fn diff_from_defaults_emits_slider_led_overrides() {
        use crate::actions::SliderLedMode;
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
        assert_eq!(merged.slider.led.mode, SliderLedMode::Bar);
    }

    #[test]
    fn partial_pads_reject_duplicate_normalized_keys() {
        // "1" and "01" parse to the same index; the partial decoder must reject
        // the collision rather than silently last-write-wins.
        let toml_str = r#"
[pads.1.hit]
type = "note"
note = 60

[pads.01.hit]
type = "note"
note = 62
"#;
        let err = toml::from_str::<PartialSettings>(toml_str).unwrap_err();
        assert!(err.to_string().contains("duplicate pad key"), "got: {err}");
    }

    #[test]
    fn partial_pads_round_trip_through_self_describing_codec() {
        use crate::PadPressureAction;

        let mut pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        pads[2] = Some(PartialPadConfig {
            hit: None,
            pressure: Some(PadPressureAction::Poly {
                channel: None,
                note: Some(60),
            }),
            led: None,
        });
        let original = PartialSettings {
            pads: Some(pads),
            ..Default::default()
        };

        let mut bytes = Vec::new();
        ciborium::into_writer(&original, &mut bytes).expect("serialize");
        let back: PartialSettings = ciborium::from_reader(&bytes[..]).expect("deserialize");
        assert_eq!(back, original);
    }

    #[test]
    fn partial_sets_pad_led_source_only() {
        let toml_str = r#"
[pads.1.led]
source = "midi_in"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);
        // TOML key 1 → internal pad 12 (row flip).
        assert_eq!(merged.pads[12].led.source, PadLedSource::MidiIn);
        // Other LED fields keep their defaults.
        assert_eq!(
            merged.pads[12].led.midi_out,
            Settings::default().pads[12].led.midi_out
        );
        // Other pads untouched.
        assert_eq!(merged.pads[0].led, Settings::default().pads[0].led);
    }

    #[test]
    fn partial_sets_pad_led_in_mode_only() {
        let toml_str = r#"
[pads.1.led.midi_in]
mode = "single"
single = "red"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);
        assert_eq!(merged.pads[12].led.midi_in.mode, crate::PadLedMode::Single);
        assert_eq!(merged.pads[12].led.midi_in.single, PadColors::Red);
        assert_eq!(merged.pads[12].led.source, PadLedSource::MidiOut);
    }

    #[test]
    fn diff_from_defaults_round_trips_pad_led() {
        let mut s = Settings::default();
        s.pads[3].led.source = PadLedSource::MidiIn;
        s.pads[3].led.midi_in = PadLedColorMode::dual(PadColors::Green, PadColors::Turquoise);
        let partial = s.diff_from_defaults();
        assert!(partial.pads.is_some(), "changed pad LED is captured");
        let round_tripped = Settings::default().merge_overrides(partial);
        assert_eq!(round_tripped, s);
    }
}
