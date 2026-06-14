use crate::controls::Buttons;
use crate::hid::HidIo;
use hidapi::HidResult;
use num_derive::FromPrimitive;
use serde::{Deserialize, Serialize};

#[derive(FromPrimitive, Debug, Clone, Copy, PartialEq)]
pub enum Brightness {
    Off = 0x00,
    Dim = 0x7c,
    Normal = 0x7e,
    Bright = 0x7f,
}

/// Per-LED brightness written to a backlight-capable button when it is lit. The
/// global `0xf3` preference scales the actual emitted intensity on top of this.
pub const BUTTON_BACKLIGHT_LEVEL: Brightness = Brightness::Normal;

#[derive(FromPrimitive, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PadColors {
    Off = 0,
    Red = 1,
    Orange = 2,
    LightOrange = 3,
    WarmYellow = 4,
    Yellow = 5,
    Lime = 6,
    Green = 7,
    Mint = 8,
    Cyan = 9,
    Turquoise = 10,
    Blue = 11,
    Plum = 12,
    Violet = 13,
    Purple = 14,
    Magenta = 15,
    Fuchsia = 16,
    White = 31,
}

impl PadColors {
    pub const ALL: [PadColors; 18] = [
        PadColors::Off,
        PadColors::Red,
        PadColors::Orange,
        PadColors::LightOrange,
        PadColors::WarmYellow,
        PadColors::Yellow,
        PadColors::Lime,
        PadColors::Green,
        PadColors::Mint,
        PadColors::Cyan,
        PadColors::Turquoise,
        PadColors::Blue,
        PadColors::Plum,
        PadColors::Violet,
        PadColors::Purple,
        PadColors::Magenta,
        PadColors::Fuchsia,
        PadColors::White,
    ];
}

impl std::fmt::Display for PadColors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PadColors::Off => "Off",
            PadColors::Red => "Red",
            PadColors::Orange => "Orange",
            PadColors::LightOrange => "Light Orange",
            PadColors::WarmYellow => "Warm Yellow",
            PadColors::Yellow => "Yellow",
            PadColors::Lime => "Lime",
            PadColors::Green => "Green",
            PadColors::Mint => "Mint",
            PadColors::Cyan => "Cyan",
            PadColors::Turquoise => "Turquoise",
            PadColors::Blue => "Blue",
            PadColors::Plum => "Plum",
            PadColors::Violet => "Violet",
            PadColors::Purple => "Purple",
            PadColors::Magenta => "Magenta",
            PadColors::Fuchsia => "Fuchsia",
            PadColors::White => "White",
        };
        f.write_str(s)
    }
}

#[derive(Clone)]
pub struct Lights {
    status: [u8; 80],
}

const SLIDER_CENTER_LED: usize = 12;

impl Lights {
    #[allow(clippy::new_without_default, reason = "intentional")]
    pub fn new() -> Self {
        Self { status: [0; 80] }
    }

    pub fn reset(&mut self) {
        self.status.fill(0);
    }

    pub fn get_button(&self, id: Buttons) -> Brightness {
        num::FromPrimitive::from_u8(self.status[id as usize]).unwrap()
    }

    pub fn button_has_light(&self, id: Buttons) -> bool {
        !matches!(id, Buttons::EncoderTouch | Buttons::EncoderPress)
    }

    pub fn set_button(&mut self, id: Buttons, b: Brightness) {
        self.status[id as usize] = b as u8;
    }

    pub fn set_slider(&mut self, id: usize, c: PadColors, b: Brightness) {
        let val = match b {
            Brightness::Off => 0,
            _ => ((c as u8) << 2) | ((b as u8) & 0b11),
        };
        self.status[55 + id] = val;
    }

    /// Test/debug accessor for the raw slider LED byte at `idx`. Not part of
    /// the stable API.
    #[doc(hidden)]
    pub fn slider_byte(&self, idx: usize) -> u8 {
        self.status[55 + idx]
    }

    /// Apply the standard slider brightness ladder over the inclusive range
    /// `[lo, hi]` with `head` as the head LED. When `stylized`, the head is
    /// Normal and the rest of the range is Dim; otherwise every lit LED in
    /// the range is Normal. LEDs outside `[lo, hi]` are written Off.
    fn render_slider_range(
        &mut self,
        lo: usize,
        hi: usize,
        head: usize,
        color: PadColors,
        stylized: bool,
    ) {
        for idx in 0..25 {
            let b = if idx < lo || idx > hi {
                Brightness::Off
            } else if idx == head {
                Brightness::Normal
            } else if stylized {
                Brightness::Dim
            } else {
                Brightness::Normal
            };
            self.set_slider(idx, color, b);
        }
    }

    fn blank_slider_leds(&mut self) {
        self.status[55..80].fill(0);
    }

    /// Map a raw touch reading (0..=200) to the head LED index (0..=24).
    /// Any non-zero raw value <= 5 pins to 0 by the `.max(0)` clamp, so a touch
    /// at the very bottom of the strip lights at least LED[0].
    fn head_from_raw(raw: u8) -> usize {
        (((raw as i32 + 4) * 25 / 200 - 1).clamp(0, 24)) as usize
    }

    /// Render the 25-LED slider bar in "bar" mode from a raw touch reading
    /// (0..=200 from the HID input report). When `raw == 0` (no touch), all
    /// LEDs blank. Otherwise LEDs 0..=head are lit; if `stylized` is true,
    /// the trail renders Dim and the head Normal; otherwise every lit LED is
    /// Normal.
    pub fn render_slider_bar(&mut self, raw: u8, color: PadColors, stylized: bool) {
        if raw == 0 {
            self.blank_slider_leds();
            return;
        }
        let head = Self::head_from_raw(raw);
        self.render_slider_range(0, head, head, color, stylized);
    }

    /// Render the slider bar in "pan" mode: LEDs from the fixed center (LED 12)
    /// outward to the current position lit. When `raw == 0`, all 25 LEDs blank.
    /// Any non-zero touch lights at least LED[12]. When `stylized`, the head
    /// (farthest from center) renders Normal and the remaining lit LEDs Dim;
    /// if head == 12, the single lit LED is the head.
    pub fn render_slider_pan(&mut self, raw: u8, color: PadColors, stylized: bool) {
        if raw == 0 {
            self.blank_slider_leds();
            return;
        }
        let head = Self::head_from_raw(raw);
        let (lo, hi) = if head <= SLIDER_CENTER_LED {
            (head, SLIDER_CENTER_LED)
        } else {
            (SLIDER_CENTER_LED, head)
        };
        self.render_slider_range(lo, hi, head, color, stylized);
    }

    /// Render the slider bar in "dot" mode: a single LED lit at the current
    /// position. All other LEDs blank. `raw == 0` blanks all.
    pub fn render_slider_dot(&mut self, raw: u8, color: PadColors) {
        if raw == 0 {
            self.blank_slider_leds();
            return;
        }
        let head = Self::head_from_raw(raw);
        // A "dot" is a one-LED range with the single LED as its head.
        // stylized has no effect when lo == hi, so pass false.
        self.render_slider_range(head, head, head, color, false);
    }

    pub fn set_pad(&mut self, id: usize, c: PadColors, b: Brightness) {
        let val = match b {
            Brightness::Off => 0,
            _ => {
                let c = c as u8;
                let b = b as u8;
                (c << 2) + (b & 0b11)
            }
        };
        self.status[39 + id] = val;
    }

    pub fn get_pad(&self, id: usize) -> (PadColors, Brightness) {
        let val = self.status[39 + id];
        let color: PadColors = num::FromPrimitive::from_u8(val >> 2).unwrap();
        let b = match val {
            0..=3 => Brightness::Off,
            _ => match val % 4 {
                0 => Brightness::Dim,
                1 => Brightness::Dim,
                2 => Brightness::Normal,
                3 => Brightness::Bright,
                _ => Brightness::Off,
            },
        };
        (color, b)
    }

    pub fn write(&self, h: &impl HidIo) -> HidResult<()> {
        let mut buf = [0u8; 81];
        buf[0] = 0x80;
        buf[1..].copy_from_slice(&self.status);
        h.write(&buf)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Brightness, Lights, PadColors};

    #[test]
    fn pad_colors_serialize_as_snake_case_string() {
        let json = serde_json::to_string(&PadColors::LightOrange).unwrap();
        assert_eq!(json, "\"light_orange\"");
    }

    #[test]
    fn pad_colors_deserialize_from_snake_case_string() {
        let parsed: PadColors = serde_json::from_str("\"warm_yellow\"").unwrap();
        assert_eq!(parsed, PadColors::WarmYellow);
    }

    #[test]
    fn set_slider_packs_color_and_brightness_white_normal_is_0x7e() {
        let mut lights = Lights::new();
        lights.set_slider(0, PadColors::White, Brightness::Normal);
        // Wire-level check: this test pins the 0x80 report-id offset and the
        // 55-byte slider region offset via the actual write path.
        let buf = lights_capture_byte(&lights, 55);
        assert_eq!(buf, 0x7e);
    }

    #[test]
    fn set_slider_writes_off_byte_when_brightness_off() {
        let mut lights = Lights::new();
        lights.set_slider(3, PadColors::Red, Brightness::Off);
        assert_eq!(lights.slider_byte(3), 0);
    }

    #[test]
    fn set_slider_packs_orange_normal_as_0x0a() {
        let mut lights = Lights::new();
        lights.set_slider(7, PadColors::Orange, Brightness::Normal);
        assert_eq!(lights.slider_byte(7), 0x0a);
    }

    #[test]
    fn render_slider_bar_zero_raw_blanks_all_leds() {
        let mut lights = Lights::new();
        lights.set_slider(5, PadColors::Red, Brightness::Normal);
        lights.render_slider_bar(0, PadColors::White, false);
        for i in 0..25 {
            assert_eq!(lights.slider_byte(i), 0, "led {i}");
        }
    }

    #[test]
    fn render_slider_bar_raw_1_lights_only_first_led_normal() {
        let mut lights = Lights::new();
        lights.render_slider_bar(1, PadColors::White, false);
        let head = ((PadColors::White as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        assert_eq!(lights.slider_byte(0), head);
        for i in 1..25 {
            assert_eq!(lights.slider_byte(i), 0, "led {i}");
        }
    }

    #[test]
    fn render_slider_bar_low_raw_values_light_only_led_0() {
        // Design contract: any non-zero touch lights at least LED[0] (Normal,
        // default color). For raw in 1..=5 only LED[0] should be lit.
        let head = ((PadColors::White as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for raw in 1u8..=5 {
            let mut lights = Lights::new();
            lights.render_slider_bar(raw, PadColors::White, false);
            assert_eq!(lights.slider_byte(0), head, "raw={raw} led 0");
            for i in 1..25 {
                assert_eq!(lights.slider_byte(i), 0, "raw={raw} led {i}");
            }
        }
    }

    #[test]
    fn render_slider_bar_raw_200_lights_all_leds_normal() {
        let mut lights = Lights::new();
        lights.render_slider_bar(200, PadColors::Red, false);
        let lit = ((PadColors::Red as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for i in 0..25 {
            assert_eq!(lights.slider_byte(i), lit, "led {i}");
        }
    }

    #[test]
    fn render_slider_bar_stylized_has_dim_trail_and_normal_head() {
        let mut lights = Lights::new();
        let raw = 100u8;
        lights.render_slider_bar(raw, PadColors::Blue, true);

        let lit_count = (raw as i32 + 4) * 25 / 200 - 1;
        let head_idx = lit_count as usize;

        let trail = ((PadColors::Blue as u8) << 2) | (Brightness::Dim as u8 & 0b11);
        let head = ((PadColors::Blue as u8) << 2) | (Brightness::Normal as u8 & 0b11);

        for i in 0..25 {
            let byte = lights.slider_byte(i);
            if i < head_idx {
                assert_eq!(byte, trail, "trail led {i}");
            } else if i == head_idx {
                assert_eq!(byte, head, "head led {i}");
            } else {
                assert_eq!(byte, 0, "off led {i}");
            }
        }
    }

    #[test]
    fn render_slider_pan_zero_raw_blanks_all() {
        let mut lights = Lights::new();
        lights.set_slider(0, PadColors::Red, Brightness::Normal);
        lights.render_slider_pan(0, PadColors::White, false);
        for i in 0..25 {
            assert_eq!(lights.slider_byte(i), 0);
        }
    }

    #[test]
    fn render_slider_pan_at_center_lights_only_led_12() {
        let mut lights = Lights::new();
        // raw value yielding lit_count == 12 with the formula ((raw+4)*25/200-1).max(0):
        // (raw+4)*25/200 == 13 → raw+4 in [104, 112) → raw in [100, 108).
        lights.render_slider_pan(104, PadColors::Green, false);
        let lit = ((PadColors::Green as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for i in 0..25 {
            let byte = lights.slider_byte(i);
            if i == 12 {
                assert_eq!(byte, lit, "center led");
            } else {
                assert_eq!(byte, 0, "led {i} should be off");
            }
        }
    }

    #[test]
    fn render_slider_pan_above_center_lights_12_to_lit_count() {
        let mut lights = Lights::new();
        let raw = 150u8;
        lights.render_slider_pan(raw, PadColors::Magenta, false);

        let lit_count = (((raw as i32 + 4) * 25 / 200 - 1).max(0)) as usize;
        assert!(lit_count > 12, "test precondition: pick a raw above center");

        let lit = ((PadColors::Magenta as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for i in 0..25 {
            let byte = lights.slider_byte(i);
            if i >= 12 && i <= lit_count {
                assert_eq!(byte, lit, "led {i} should be lit");
            } else {
                assert_eq!(byte, 0, "led {i} should be off");
            }
        }
    }

    #[test]
    fn render_slider_pan_below_center_lights_lit_count_to_12() {
        let mut lights = Lights::new();
        let raw = 50u8;
        lights.render_slider_pan(raw, PadColors::Cyan, false);

        let lit_count = (((raw as i32 + 4) * 25 / 200 - 1).max(0)) as usize;
        assert!(lit_count < 12, "test precondition: pick a raw below center");

        let lit = ((PadColors::Cyan as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for i in 0..25 {
            let byte = lights.slider_byte(i);
            if i >= lit_count && i <= 12 {
                assert_eq!(byte, lit, "led {i} should be lit");
            } else {
                assert_eq!(byte, 0, "led {i} should be off");
            }
        }
    }

    #[test]
    fn render_slider_pan_stylized_endpoint_is_head() {
        let mut lights = Lights::new();
        let raw = 150u8;
        lights.render_slider_pan(raw, PadColors::Plum, true);
        let lit_count = (((raw as i32 + 4) * 25 / 200 - 1).max(0)) as usize;

        let head = ((PadColors::Plum as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        let trail = ((PadColors::Plum as u8) << 2) | (Brightness::Dim as u8 & 0b11);

        for i in 0..25 {
            let byte = lights.slider_byte(i);
            if i == lit_count {
                assert_eq!(byte, head, "head led {i}");
            } else if i >= 12 && i < lit_count {
                assert_eq!(byte, trail, "trail led {i}");
            } else {
                assert_eq!(byte, 0, "off led {i}");
            }
        }
    }

    #[test]
    fn render_slider_dot_zero_raw_blanks_all() {
        let mut lights = Lights::new();
        lights.set_slider(7, PadColors::Red, Brightness::Normal);
        lights.render_slider_dot(0, PadColors::White);
        for i in 0..25 {
            assert_eq!(lights.slider_byte(i), 0);
        }
    }

    #[test]
    fn render_slider_dot_lights_single_led_at_lit_count() {
        let mut lights = Lights::new();
        let raw = 100u8;
        lights.render_slider_dot(raw, PadColors::Yellow);
        let lit_count = (((raw as i32 + 4) * 25 / 200 - 1).max(0)) as usize;

        let lit = ((PadColors::Yellow as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for i in 0..25 {
            let byte = lights.slider_byte(i);
            if i == lit_count {
                assert_eq!(byte, lit, "dot led {i}");
            } else {
                assert_eq!(byte, 0, "off led {i}");
            }
        }
    }

    // Helper for wire-level byte inspection (kept for the one test that pins
    // the 0x80 report-id offset and the 55-byte slider region offset).
    fn lights_capture_byte(lights: &Lights, idx: usize) -> u8 {
        use std::cell::RefCell;
        struct Capture(RefCell<Vec<u8>>);
        impl crate::hid::HidIo for Capture {
            fn write(&self, buf: &[u8]) -> hidapi::HidResult<usize> {
                self.0.borrow_mut().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn read_timeout(&self, _buf: &mut [u8], _ms: i32) -> hidapi::HidResult<usize> {
                Ok(0)
            }
            fn send_feature_report(&self, _data: &[u8]) -> hidapi::HidResult<()> {
                Ok(())
            }
        }
        let cap = Capture(RefCell::new(Vec::new()));
        lights.write(&cap).unwrap();
        cap.0.borrow()[idx + 1]
    }
}

#[cfg(test)]
mod pad_colors_tests {
    use super::PadColors;

    #[test]
    fn pad_colors_labels_are_unique_and_nonempty() {
        let labels: Vec<String> = PadColors::ALL.iter().map(|c| c.to_string()).collect();
        assert!(labels.iter().all(|l| !l.is_empty()), "{labels:?}");
        let unique: std::collections::HashSet<_> = labels.iter().collect();
        assert_eq!(unique.len(), labels.len(), "duplicate label: {labels:?}");
    }
}
