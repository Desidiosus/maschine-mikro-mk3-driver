use std::collections::BTreeMap;

use maschine_library::controls::{BUTTON_NAMES, button_index_from_name};
use serde::de::Error as DeError;
use serde::{Deserialize, Serialize};

use crate::actions::{
    ButtonPressAction, EncoderTurnAction, PadHitAction, PadLedColorMode, PadLedSource,
    PadPressureAction, SliderLedMode, SliderPositionAction, SliderTouchAction,
};
use crate::pad_paging::{PadPage, PageId, default_page};
use crate::velocity_curve::PadVelocityCurve;
use crate::{MidiChannel, Settings};
use maschine_library::lights::PadColors;

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialSettings {
    pub global: Option<PartialGlobalSettings>,
    pub hardware: Option<PartialHardwareSettings>,
    pub bridge: Option<PartialBridgeSettings>,
    pub driver: Option<PartialDriverSettings>,
    // Sparse per-pad overrides applied to the ACTIVE page on merge. Retained so a
    // pre-paging `[pads]` config still loads (migrating onto page 0) and so the
    // GUI's per-pad edits stay small. Never emitted by `diff_from` — persistence
    // writes the whole `pad_paging` block instead.
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
    pub pad_paging: Option<PartialPadPaging>,
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
pub struct PartialDriverSettings {
    pub soft_off_enabled: Option<bool>,
    pub self_test_on_launch: Option<bool>,
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

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialPadPaging {
    pub enabled: Option<bool>,
    pub active: Option<usize>,
    pub default_page_color: Option<PadColors>,
    /// Sparse per-page patches. Applied positionally onto the existing page list
    /// when the length matches (preserves seed show-through); otherwise a
    /// structural change (add/duplicate/delete/reorder), so each patch is applied
    /// onto a fresh default page instead.
    pub pages: Option<Vec<PartialPadPage>>,
}

#[derive(Default, Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PartialPadPage {
    /// The page's identity. Carried through the diff so a structural rewrite
    /// (add/delete/reorder) moves ids with their pages instead of leaving them
    /// pinned to slots. Absent in overrides written before ids existed; the
    /// merge assigns those.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<PageId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<PadColors>,
    /// Set the page name back to `None` (render as "Page N"). Distinguishes a
    /// deliberate reset-to-default from "field absent / no change".
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub clear_name: bool,
    /// Set the page color back to `None` (inherit `default_page_color`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub clear_color: bool,
    #[serde(
        deserialize_with = "deserialize_partial_pads",
        serialize_with = "serialize_partial_pads",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub pads: Option<[Option<PartialPadConfig>; 16]>,
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

impl PartialPadPage {
    fn apply_onto(self, page: &mut PadPage) {
        if let Some(id) = self.id {
            page.id = id;
        }
        if self.clear_name {
            page.name = None;
        } else if let Some(name) = self.name {
            page.name = Some(name);
        }
        if self.clear_color {
            page.color = None;
        } else if let Some(color) = self.color {
            page.color = Some(color);
        }
        if let Some(pads) = self.pads {
            for (idx, cfg) in pads.into_iter().enumerate() {
                let Some(cfg) = cfg else { continue };
                apply_overrides!(page.pads[idx], cfg; hit, pressure);
                if let Some(led) = cfg.led {
                    apply_overrides!(page.pads[idx].led, led; source, midi_in, midi_out);
                }
            }
        }
    }

    fn diff(cur: &PadPage, base: &PadPage) -> PartialPadPage {
        let mut pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        let mut any_pad = false;
        for (idx, pad) in cur.pads.iter().enumerate() {
            let mut p = PartialPadConfig::default();
            diff_section!(pad, base.pads[idx], p; copy: {}; clone: { hit, pressure };);
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
        let (name, clear_name) = match (&cur.name, &base.name) {
            (a, b) if a == b => (None, false),
            (None, Some(_)) => (None, true),
            (Some(n), _) => (Some(n.clone()), false),
            (None, None) => (None, false),
        };
        let (color, clear_color) = match (cur.color, base.color) {
            (a, b) if a == b => (None, false),
            (None, Some(_)) => (None, true),
            (Some(c), _) => (Some(c), false),
            (None, None) => (None, false),
        };
        PartialPadPage {
            id: (cur.id != base.id).then_some(cur.id),
            name,
            clear_name,
            color,
            clear_color,
            pads: any_pad.then_some(pads),
        }
    }
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
        if let Some(d) = partial.driver {
            apply_overrides!(self.driver, d; soft_off_enabled, self_test_on_launch);
        }
        if let Some(pp) = partial.pad_paging {
            apply_overrides!(self.pad_paging, pp; enabled, active, default_page_color);
            if let Some(pages) = pp.pages {
                if pages.len() == self.pad_paging.pages.len() {
                    for (i, patch) in pages.into_iter().enumerate() {
                        patch.apply_onto(&mut self.pad_paging.pages[i]);
                    }
                } else {
                    self.pad_paging.pages = pages
                        .into_iter()
                        .map(|patch| {
                            let mut page = default_page();
                            patch.apply_onto(&mut page);
                            page
                        })
                        .collect();
                }
            }
        }
        // Renumber pages that arrived sharing an id — a config written before ids
        // existed loads every page as unassigned — so no two pages downstream can
        // claim to be the same page.
        self.pad_paging.ensure_unique_page_ids();
        // Self-heal a persisted/overridden `active` that outlives a shrunk page vec
        // (e.g. a `-c` seed dropped pages). Clamp instead of bricking startup;
        // `validate()` would otherwise reject and the driver would refuse to boot.
        // Runs for every merge, not only one carrying `pad_paging`: an earlier layer
        // can leave `active` stale, and the legacy absorber below would then have no
        // page to write to and would drop the whole block.
        if self.pad_paging.active >= self.pad_paging.pages.len() {
            self.pad_paging.active = self.pad_paging.pages.len().saturating_sub(1);
        }
        if let Some(pads) = partial.pads {
            let page = self.pad_paging.active_page_mut();
            for (idx, cfg) in pads.into_iter().enumerate() {
                let Some(cfg) = cfg else { continue };
                apply_overrides!(page.pads[idx], cfg; hit, pressure);
                if let Some(led) = cfg.led {
                    apply_overrides!(page.pads[idx].led, led; source, midi_in, midi_out);
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

        let mut d = PartialDriverSettings::default();
        diff_section!(
            self.driver, base.driver, d;
            copy: { soft_off_enabled, self_test_on_launch };
            clone: {};
        );
        if d != PartialDriverSettings::default() {
            out.driver = Some(d);
        }

        if self.pad_paging != base.pad_paging {
            let pages = if self.pad_paging.pages != base.pad_paging.pages {
                Some(
                    if self.pad_paging.pages.len() == base.pad_paging.pages.len() {
                        self.pad_paging
                            .pages
                            .iter()
                            .zip(base.pad_paging.pages.iter())
                            .map(|(cur, b)| PartialPadPage::diff(cur, b))
                            .collect()
                    } else {
                        let def = default_page();
                        self.pad_paging
                            .pages
                            .iter()
                            .map(|cur| PartialPadPage::diff(cur, &def))
                            .collect()
                    },
                )
            } else {
                None
            };
            out.pad_paging = Some(PartialPadPaging {
                enabled: (self.pad_paging.enabled != base.pad_paging.enabled)
                    .then_some(self.pad_paging.enabled),
                active: (self.pad_paging.active != base.pad_paging.active)
                    .then_some(self.pad_paging.active),
                default_page_color: (self.pad_paging.default_page_color
                    != base.pad_paging.default_page_color)
                    .then_some(self.pad_paging.default_page_color),
                pages,
            });
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
            merged.active_pads()[8].pressure,
            PadPressureAction::Poly {
                channel: None,
                note: None
            }
        );
        assert_eq!(
            merged.active_pads()[5].pressure,
            PadPressureAction::Disabled
        );
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
        s.active_pads_mut()[2].pressure = PadPressureAction::Poly {
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
        live.active_pads_mut()[2].hit = PadHitAction::Note {
            channel: None,
            note: 61,
        };

        let diff = live.diff_from(&base);
        assert!(diff.global.is_none(), "unchanged-vs-base global is omitted");
        assert!(diff.pad_paging.is_some(), "changed pad is captured");
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
        assert_eq!(merged.active_pads()[12].led.source, PadLedSource::MidiIn);
        // Other LED fields keep their defaults.
        assert_eq!(
            merged.active_pads()[12].led.midi_out,
            Settings::default().active_pads()[12].led.midi_out
        );
        // Other pads untouched.
        assert_eq!(
            merged.active_pads()[0].led,
            Settings::default().active_pads()[0].led
        );
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
        assert_eq!(
            merged.active_pads()[12].led.midi_in.mode,
            crate::PadLedMode::Single
        );
        assert_eq!(merged.active_pads()[12].led.midi_in.single, PadColors::Red);
        assert_eq!(merged.active_pads()[12].led.source, PadLedSource::MidiOut);
    }

    #[test]
    fn diff_from_defaults_round_trips_pad_led() {
        let mut s = Settings::default();
        s.active_pads_mut()[3].led.source = PadLedSource::MidiIn;
        s.active_pads_mut()[3].led.midi_in =
            PadLedColorMode::dual(PadColors::Green, PadColors::Turquoise);
        let partial = s.diff_from_defaults();
        assert!(partial.pad_paging.is_some(), "changed pad LED is captured");
        let round_tripped = Settings::default().merge_overrides(partial);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn driver_defaults_produce_no_diff() {
        let s = Settings::default();
        assert!(s.diff_from_defaults().driver.is_none());
    }

    #[test]
    fn driver_flag_round_trips_through_merge_and_diff() {
        let mut s = Settings::default();
        s.driver.soft_off_enabled = false;

        let diff = s.diff_from_defaults();
        let d = diff.driver.clone().expect("driver section present");
        assert_eq!(d.soft_off_enabled, Some(false));
        assert_eq!(d.self_test_on_launch, None);

        let merged = Settings::default().merge_overrides(diff);
        assert!(!merged.driver.soft_off_enabled);
        assert!(merged.driver.self_test_on_launch);
    }

    #[test]
    fn legacy_pads_table_migrates_onto_active_page() {
        let toml_str = r#"
[pads.5.pressure]
type = "poly"
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);
        // TOML key 5 → internal pad 8; applied to page 0's pads.
        assert_eq!(
            merged.pad_paging.pages[0].pads[8].pressure,
            PadPressureAction::Poly {
                channel: None,
                note: None
            }
        );
    }

    #[test]
    fn pad_paging_enable_and_active_round_trip() {
        let mut s = Settings::default();
        s.pad_paging.enabled = true;
        let page = PadPage {
            id: s.pad_paging.next_page_id(),
            ..s.pad_paging.pages[0].clone()
        };
        s.pad_paging.pages.push(page);
        s.pad_paging.active = 1;

        let diff = s.diff_from_defaults();
        assert!(diff.pad_paging.is_some(), "changed paging is captured");
        assert!(
            diff.pads.is_none(),
            "diff never emits the legacy pads field"
        );

        let round_tripped = Settings::default().merge_overrides(diff);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn pad_paging_default_color_round_trips() {
        let mut s = Settings::default();
        s.pad_paging.default_page_color = PadColors::Magenta;
        let diff = s.diff_from_defaults();
        let round_tripped = Settings::default().merge_overrides(diff);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn per_page_pad_edit_round_trips_through_pad_paging() {
        let mut s = Settings::default();
        let mut page2 = PadPage {
            id: s.pad_paging.next_page_id(),
            ..s.pad_paging.pages[0].clone()
        };
        page2.pads[3].hit = PadHitAction::Note {
            channel: None,
            note: 61,
        };
        s.pad_paging.pages.push(page2);
        s.pad_paging.active = 0;

        let diff = s.diff_from_defaults();
        let round_tripped = Settings::default().merge_overrides(diff);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn pages_replace_then_active_pad_absorber_layers_on_top() {
        // A partial that both replaces the page list (2 pages, structural) AND
        // carries a sparse `pads` active-page edit must apply the replace first,
        // then the edit on top — swapping the order would clobber the edit.
        let mut page_b_pads: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        page_b_pads[7] = Some(PartialPadConfig {
            hit: Some(PadHitAction::Note {
                channel: None,
                note: 70,
            }),
            pressure: None,
            led: None,
        });

        let mut sparse: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
        sparse[0] = Some(PartialPadConfig {
            hit: Some(PadHitAction::Note {
                channel: None,
                note: 99,
            }),
            pressure: None,
            led: None,
        });

        let partial = PartialSettings {
            pad_paging: Some(PartialPadPaging {
                enabled: None,
                active: None,
                default_page_color: None,
                pages: Some(vec![
                    PartialPadPage::default(),
                    PartialPadPage {
                        pads: Some(page_b_pads),
                        ..Default::default()
                    },
                ]),
            }),
            pads: Some(sparse),
            ..Default::default()
        };

        let merged = Settings::default().merge_overrides(partial);
        assert_eq!(merged.pad_paging.pages.len(), 2, "page list was replaced");
        // active defaults to 0 → sparse edit landed on page 0, pad 0.
        assert_eq!(
            merged.pad_paging.pages[0].pads[0].hit,
            PadHitAction::Note {
                channel: None,
                note: 99
            }
        );
        // page 1 carries its own seeded pad (proves the replace ran).
        assert_eq!(
            merged.pad_paging.pages[1].pads[7].hit,
            PadHitAction::Note {
                channel: None,
                note: 70
            }
        );
    }

    #[test]
    fn seed_pad_shows_through_after_editing_a_different_pad() {
        // base stands in for a `-c` seed that customized pad 5.
        let mut base = Settings::default();
        base.active_pads_mut()[5].hit = PadHitAction::Note {
            channel: None,
            note: 60,
        };

        // live = base with a DIFFERENT pad (3) edited via the GUI.
        let mut live = base.clone();
        live.active_pads_mut()[3].hit = PadHitAction::Note {
            channel: None,
            note: 40,
        };

        let diff = live.diff_from(&base);

        // The seed later changes pad 5 to 70. Applying the persisted diff onto the
        // updated seed must let pad 5 show through (NOT freeze at 60), while pad 3's
        // edit still applies.
        let mut later_seed = base.clone();
        later_seed.active_pads_mut()[5].hit = PadHitAction::Note {
            channel: None,
            note: 70,
        };
        let merged = later_seed.merge_overrides(diff);

        assert_eq!(
            merged.active_pads()[3].hit,
            PadHitAction::Note {
                channel: None,
                note: 40
            }
        );
        assert_eq!(
            merged.active_pads()[5].hit,
            PadHitAction::Note {
                channel: None,
                note: 70
            },
            "unedited pad 5 must show through the updated seed, not freeze"
        );
    }

    #[test]
    fn single_pad_paging_pad_override_parses_without_all_16() {
        // A single pad line under [[pad_paging.pages]] must parse (sparse), not
        // demand all 16 pads.
        let toml_str = r#"
[[pad_paging.pages]]

[pad_paging.pages.pads.1.hit]
type = "note"
note = 55
"#;
        let partial: PartialSettings = toml::from_str(toml_str).unwrap();
        let merged = Settings::default().merge_overrides(partial);
        // TOML key 1 → internal pad 12.
        assert_eq!(
            merged.active_pads()[12].hit,
            PadHitAction::Note {
                channel: None,
                note: 55
            }
        );
        assert_eq!(
            merged.active_pads()[0].hit,
            Settings::default().active_pads()[0].hit
        );
    }

    #[test]
    fn structural_page_add_round_trips_through_pad_paging() {
        // Adding a second page changes the page count → structural full spell-out.
        let mut s = Settings::default();
        let mut page2 = s.pad_paging.new_page();
        page2.pads[7].hit = PadHitAction::Note {
            channel: None,
            note: 70,
        };
        s.pad_paging.pages.push(page2);
        s.pad_paging.active = 1;

        let diff = s.diff_from_defaults();
        assert!(diff.pad_paging.is_some());
        assert!(
            diff.pads.is_none(),
            "diff never emits the legacy pads field"
        );
        let round_tripped = Settings::default().merge_overrides(diff);
        assert_eq!(round_tripped, s);
    }

    #[test]
    fn pads_with_out_of_range_active_self_heals_instead_of_failing_validate() {
        // Superseded by the merge-time clamp: an out-of-range `active` now
        // self-heals instead of surviving into an invalid `Settings` (previously
        // this asserted `validate().is_err()`; now the merge itself repairs it).
        let partial = PartialSettings {
            pad_paging: Some(PartialPadPaging {
                enabled: None,
                active: Some(5),
                default_page_color: None,
                pages: None,
            }),
            pads: Some({
                let mut s: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
                s[0] = Some(PartialPadConfig {
                    hit: None,
                    pressure: Some(PadPressureAction::Poly {
                        channel: None,
                        note: None,
                    }),
                    led: None,
                });
                s
            }),
            ..Default::default()
        };
        let merged = Settings::default().merge_overrides(partial); // must NOT panic
        assert_eq!(
            merged.pad_paging.active, 0,
            "stale active self-heals into range"
        );
        merged.validate().expect("clamped settings validate");
        // The legacy pads absorber still applies, landing on the clamped active page.
        assert_eq!(
            merged.active_pads()[0].pressure,
            PadPressureAction::Poly {
                channel: None,
                note: None
            }
        );
    }

    #[test]
    fn a_reorder_moves_page_ids_with_their_pages() {
        // The diff rewrites pages slot by slot, so without the id travelling in
        // the patch the identities would stay pinned to the slots and every
        // consumer would believe the pages never moved.
        let mut s = Settings::default();
        s.pad_paging.pages.push(s.pad_paging.new_page());
        let (first, second) = (s.pad_paging.pages[0].id, s.pad_paging.pages[1].id);

        let mut reordered = s.clone();
        reordered.pad_paging.pages.swap(0, 1);

        let merged = s.clone().merge_overrides(reordered.diff_from(&s));

        assert_eq!(
            (merged.pad_paging.pages[0].id, merged.pad_paging.pages[1].id),
            (second, first)
        );
    }

    #[test]
    fn a_config_written_before_ids_existed_gets_distinct_ones_on_merge() {
        // Overrides that predate ids deserialize every page as unassigned. The
        // merge has to pull them apart, or two pages would both read as "no id"
        // and compare equal as identities.
        let partial: PartialSettings = toml::from_str(
            "[pad_paging]\n\
             [[pad_paging.pages]]\n\
             name = \"Drums\"\n\
             [[pad_paging.pages]]\n\
             name = \"Keys\"\n",
        )
        .unwrap();

        let merged = Settings::default().merge_overrides(partial);

        let ids: Vec<_> = merged.pad_paging.pages.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1], "each page gets its own identity: {ids:?}");
    }

    #[test]
    fn legacy_pads_apply_even_when_the_merge_carries_no_pad_paging() {
        // A partial with only the legacy `[pads]` block, merged onto settings
        // whose `active` an earlier layer left out of range. The absorber has to
        // land on the same page `active_pads()` reads, not silently discard the
        // whole block because a raw index missed.
        let mut base = Settings::default();
        base.pad_paging.active = 3;

        let partial = PartialSettings {
            pads: Some({
                let mut s: [Option<PartialPadConfig>; 16] = std::array::from_fn(|_| None);
                s[0] = Some(PartialPadConfig {
                    hit: Some(PadHitAction::Note {
                        channel: None,
                        note: 70,
                    }),
                    pressure: None,
                    led: None,
                });
                s
            }),
            ..Default::default()
        };

        let merged = base.merge_overrides(partial);
        assert_eq!(merged.pad_paging.active, 0, "stale active self-heals");
        assert_eq!(
            merged.active_pads()[0].hit,
            PadHitAction::Note {
                channel: None,
                note: 70
            },
            "a legacy pads block must not be dropped by an out-of-range active"
        );
    }

    #[test]
    fn merge_clamps_active_into_range_when_pages_shrink() {
        let mut base = Settings::default();
        base.pad_paging
            .pages
            .push(crate::pad_paging::default_page());
        base.pad_paging.active = 1;

        // The override shrinks pages back to one but leaves active=1. A different
        // page count than base makes the merge rebuild a single page, which must
        // self-heal `active` into range instead of leaving it dangling at 1.
        let delta = PartialSettings {
            pad_paging: Some(PartialPadPaging {
                active: Some(1),
                pages: Some(vec![PartialPadPage::default()]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let merged = base.merge_overrides(delta);
        assert_eq!(merged.pad_paging.pages.len(), 1);
        assert_eq!(merged.pad_paging.active, 0, "active self-heals into range");
        merged.validate().expect("clamped settings validate");
    }

    #[test]
    fn per_page_color_clear_to_inherit_round_trips() {
        // Base: a page with an explicit color. Edit: clear it to inherit (None).
        let mut base = Settings::default();
        base.pad_paging.pages[0].color = Some(PadColors::Red);

        let mut edited = base.clone();
        edited.pad_paging.pages[0].color = None;

        let delta = edited.diff_from(&base);
        let merged = base.merge_overrides(delta);
        assert_eq!(
            merged.pad_paging.pages[0].color, None,
            "clearing a page color to inherit must survive diff+merge"
        );
    }

    #[test]
    fn per_page_name_clear_round_trips() {
        let mut base = Settings::default();
        base.pad_paging.pages[0].name = Some("Kick".to_string());

        let mut edited = base.clone();
        edited.pad_paging.pages[0].name = None;

        let merged = base.clone().merge_overrides(edited.diff_from(&base));
        assert_eq!(merged.pad_paging.pages[0].name, None);
    }

    #[test]
    fn reorder_preserves_inherit_and_explicit_colors() {
        // Two pages: A explicit Red, B inherit (None). Reorder to [B, A].
        let mut base = Settings::default();
        base.pad_paging
            .pages
            .push(crate::pad_paging::default_page());
        base.pad_paging.pages[0].color = Some(PadColors::Red);
        base.pad_paging.pages[1].color = None;
        base.pad_paging.pages[0].name = Some("A".to_string());
        base.pad_paging.pages[1].name = None;

        let mut edited = base.clone();
        edited.pad_paging.pages.swap(0, 1);

        let merged = base.clone().merge_overrides(edited.diff_from(&base));
        assert_eq!(
            merged.pad_paging.pages[0].color, None,
            "moved inherit page stays inherit"
        );
        assert_eq!(merged.pad_paging.pages[0].name, None);
        assert_eq!(
            merged.pad_paging.pages[1].color,
            Some(PadColors::Red),
            "moved explicit page keeps its color"
        );
        assert_eq!(merged.pad_paging.pages[1].name.as_deref(), Some("A"));
    }

    #[test]
    fn clear_flags_round_trip_through_toml() {
        // A page colour reset must survive being written to and read back from the
        // config file, not just an in-process merge.
        let mut base = Settings::default();
        base.pad_paging.pages[0].color = Some(PadColors::Red);
        base.pad_paging.pages[0].name = Some("Kick".to_string());

        let mut edited = base.clone();
        edited.pad_paging.pages[0].color = None;
        edited.pad_paging.pages[0].name = None;

        let delta = edited.diff_from(&base);
        let text = toml::to_string(&delta).unwrap();
        let reparsed: PartialSettings = toml::from_str(&text).unwrap();
        assert_eq!(reparsed, delta, "clear flags survive a TOML round-trip");

        let merged = base.merge_overrides(reparsed);
        assert_eq!(merged.pad_paging.pages[0].color, None);
        assert_eq!(merged.pad_paging.pages[0].name, None);
    }
}
