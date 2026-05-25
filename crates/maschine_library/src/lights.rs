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

#[derive(Clone)]
pub struct Lights {
    status: [u8; 80],
}

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

    /// Render the 25-LED slider bar in "bar" mode from a raw touch reading
    /// (0..=200 from the HID input report). When `raw == 0` (no touch), all
    /// LEDs blank. Otherwise LEDs 0..=lit_count are lit; if `stylized` is true,
    /// the trail (0..lit_count) renders Dim and the head (lit_count) Normal;
    /// otherwise every lit LED renders Normal.
    pub fn render_slider_bar(&mut self, raw: u8, color: PadColors, stylized: bool) {
        if raw == 0 {
            for i in 0..25 {
                self.set_slider(i, PadColors::Off, Brightness::Off);
            }
            return;
        }
        let lit_count: i32 = ((raw as i32 - 1 + 5) * 25 / 200 - 1).max(0);
        for idx in 0i32..25 {
            let b = if idx > lit_count {
                Brightness::Off
            } else if idx == lit_count {
                Brightness::Normal
            } else if stylized {
                Brightness::Dim
            } else {
                Brightness::Normal
            };
            let c = if matches!(b, Brightness::Off) {
                PadColors::Off
            } else {
                color
            };
            self.set_slider(idx as usize, c, b);
        }
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
        let buf = lights_status_byte(&lights, 55);
        assert_eq!(buf, 0x7e);
    }

    #[test]
    fn set_slider_writes_off_byte_when_brightness_off() {
        let mut lights = Lights::new();
        lights.set_slider(3, PadColors::Red, Brightness::Off);
        assert_eq!(lights_status_byte(&lights, 55 + 3), 0);
    }

    #[test]
    fn set_slider_packs_orange_normal_as_0x0a() {
        let mut lights = Lights::new();
        lights.set_slider(7, PadColors::Orange, Brightness::Normal);
        assert_eq!(lights_status_byte(&lights, 55 + 7), 0x0a);
    }

    #[test]
    fn render_slider_bar_zero_raw_blanks_all_leds() {
        let mut lights = Lights::new();
        lights.set_slider(5, PadColors::Red, Brightness::Normal);
        lights.render_slider_bar(0, PadColors::White, false);
        for i in 0..25 {
            assert_eq!(lights_status_byte(&lights, 55 + i), 0, "led {i}");
        }
    }

    #[test]
    fn render_slider_bar_raw_1_lights_only_first_led_normal() {
        let mut lights = Lights::new();
        lights.render_slider_bar(1, PadColors::White, false);
        let head = ((PadColors::White as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        assert_eq!(lights_status_byte(&lights, 55), head);
        for i in 1..25 {
            assert_eq!(lights_status_byte(&lights, 55 + i), 0, "led {i}");
        }
    }

    #[test]
    fn render_slider_bar_raw_200_lights_all_leds_normal() {
        let mut lights = Lights::new();
        lights.render_slider_bar(200, PadColors::Red, false);
        let lit = ((PadColors::Red as u8) << 2) | (Brightness::Normal as u8 & 0b11);
        for i in 0..25 {
            assert_eq!(lights_status_byte(&lights, 55 + i), lit, "led {i}");
        }
    }

    #[test]
    fn render_slider_bar_stylized_has_dim_trail_and_normal_head() {
        let mut lights = Lights::new();
        let raw = 100u8;
        lights.render_slider_bar(raw, PadColors::Blue, true);

        let lit_count = (raw as i32 - 1 + 5) * 25 / 200 - 1;
        let head_idx = lit_count as usize;

        let trail = ((PadColors::Blue as u8) << 2) | (Brightness::Dim as u8 & 0b11);
        let head = ((PadColors::Blue as u8) << 2) | (Brightness::Normal as u8 & 0b11);

        for i in 0..25 {
            let byte = lights_status_byte(&lights, 55 + i);
            if i < head_idx {
                assert_eq!(byte, trail, "trail led {i}");
            } else if i == head_idx {
                assert_eq!(byte, head, "head led {i}");
            } else {
                assert_eq!(byte, 0, "off led {i}");
            }
        }
    }

    // Helper for byte inspection during tests.
    fn lights_status_byte(lights: &Lights, idx: usize) -> u8 {
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
        }
        let cap = Capture(RefCell::new(Vec::new()));
        lights.write(&cap).unwrap();
        cap.0.borrow()[idx + 1]
    }
}
