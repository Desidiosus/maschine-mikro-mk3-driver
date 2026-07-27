use crate::font::Font;
use crate::hid::HidIo;
use hidapi::HidResult;

const HEADER_HI: [u8; 9] = [0xe0, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x02, 0x00];
const HEADER_LO: [u8; 9] = [0xe0, 0x00, 0x00, 0x02, 0x00, 0x80, 0x00, 0x02, 0x00];

pub const SCREEN_WIDTH: usize = 128;
pub const SCREEN_HEIGHT: usize = 32;
pub const SCREEN_TEXT_CHAR_WIDTH: usize = 8;
pub const SCREEN_TEXT_Y_POSITION: usize = 12;
pub const SCREEN_TEXT_SCALE: usize = 1;

const SYSEX_MANUFACTURER: [u8; 3] = [0x00, 0x21, 0x09];
const SYSEX_CMD_TEXT: u8 = 0x01;
const SYSEX_CMD_CLEAR: u8 = 0x02;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScreenCommand {
    Text(String),
    Clear,
}

pub fn parse_sysex_command(message: &[u8]) -> Option<ScreenCommand> {
    if message.len() < 6
        || message.first().copied() != Some(0xF0)
        || message.last().copied() != Some(0xF7)
    {
        return None;
    }

    if message[1..4] != SYSEX_MANUFACTURER {
        return None;
    }

    match message[4] {
        SYSEX_CMD_TEXT => {
            let text = String::from_utf8_lossy(&message[5..message.len().saturating_sub(1)]);
            Some(ScreenCommand::Text(text.into_owned()))
        }
        SYSEX_CMD_CLEAR => Some(ScreenCommand::Clear),
        _ => None,
    }
}

pub fn render_centered_text_with_style(screen: &mut Screen, text: &str, scale: usize, y: usize) {
    screen.reset();

    let text_width = text.chars().count() * SCREEN_TEXT_CHAR_WIDTH * scale;
    let x_start = if text_width < SCREEN_WIDTH {
        (SCREEN_WIDTH - text_width) / 2
    } else {
        0
    };

    Font::write_str(screen, y, x_start, text, scale);
}

pub fn render_centered_text(screen: &mut Screen, text: &str) {
    render_centered_text_with_style(screen, text, SCREEN_TEXT_SCALE, SCREEN_TEXT_Y_POSITION);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    buffer: [u8; 512],
}

impl Screen {
    #[allow(clippy::new_without_default, reason = "intentional")]
    pub fn new() -> Self {
        Self {
            buffer: [0xff; 512],
        }
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0xff);
    }

    #[allow(dead_code)]
    pub fn get(&self, i: usize, j: usize) -> bool {
        let chunk = i / 8;
        let imod = i % 8;
        let idx = chunk * 128 + j;
        let val = self.buffer[idx] & (1 << imod);
        val == 0
    }

    pub fn set(&mut self, i: usize, j: usize, val: bool) {
        let chunk = i / 8;
        let imod: u8 = (i % 8) as u8;
        let idx = chunk * 128 + j;
        let mask: u8 = 1 << imod;
        if val {
            self.buffer[idx] &= !mask;
        } else {
            self.buffer[idx] |= mask;
        }
    }

    pub fn write(&self, h: &impl HidIo) -> HidResult<()> {
        let mut buf = [0u8; 265];
        buf[..9].copy_from_slice(&HEADER_HI);
        buf[9..].copy_from_slice(&self.buffer[..256]);
        h.write(&buf)?;

        buf[..9].copy_from_slice(&HEADER_LO);
        buf[9..].copy_from_slice(&self.buffer[256..]);
        h.write(&buf)?;
        Ok(())
    }
}
