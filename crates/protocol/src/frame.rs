use serde::Serialize;
use serde::de::DeserializeOwned;

/// Hard cap on a single frame's payload length, guarding against a corrupt or
/// hostile length prefix allocating unbounded memory.
pub const MAX_FRAME_LEN: u32 = 8 * 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    Encode(String),
    Decode(String),
    /// The declared payload length exceeds `MAX_FRAME_LEN`.
    TooLong(u32),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Encode(e) => write!(f, "frame encode error: {e}"),
            FrameError::Decode(e) => write!(f, "frame decode error: {e}"),
            FrameError::TooLong(n) => write!(f, "frame length {n} exceeds MAX_FRAME_LEN"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Serialize `value` to a `u32` LE length-prefixed CBOR frame.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload).map_err(|e| FrameError::Encode(e.to_string()))?;
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::TooLong(u32::MAX))?;
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLong(len));
    }
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Try to decode one frame from the front of `buf`.
///
/// - `Ok(Some((value, consumed)))` — a full frame was decoded; `consumed` bytes
///   should be drained from the front of `buf`.
/// - `Ok(None)` — `buf` does not yet contain a complete frame; read more bytes.
/// - `Err(_)` — malformed length prefix or payload.
pub fn decode_frame<T: DeserializeOwned>(buf: &[u8]) -> Result<Option<(T, usize)>, FrameError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if len > MAX_FRAME_LEN {
        return Err(FrameError::TooLong(len));
    }
    let len = len as usize;
    let end = 4 + len;
    if buf.len() < end {
        return Ok(None);
    }
    let value =
        ciborium::from_reader(&buf[4..end]).map_err(|e| FrameError::Decode(e.to_string()))?;
    Ok(Some((value, end)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::messages::{DriverToGui, GuiToDriver, MidiDir};

    #[test]
    fn frame_round_trips_a_message() {
        let msg = GuiToDriver::GetSettings;
        let bytes = encode_frame(&msg).unwrap();
        let (back, consumed): (GuiToDriver, usize) = decode_frame(&bytes).unwrap().unwrap();
        assert_eq!(back, msg);
        assert_eq!(consumed, bytes.len());
    }

    #[test]
    fn decode_returns_none_on_short_length_prefix() {
        let buf = [0u8, 1, 2]; // < 4 bytes
        let res: Option<(GuiToDriver, usize)> = decode_frame(&buf).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn decode_returns_none_on_incomplete_payload() {
        let msg = DriverToGui::MidiActivity { dir: MidiDir::Out };
        let bytes = encode_frame(&msg).unwrap();
        // Drop the last payload byte: full length prefix, truncated payload.
        let truncated = &bytes[..bytes.len() - 1];
        let res: Option<(DriverToGui, usize)> = decode_frame(truncated).unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn decode_reports_consumed_and_leaves_trailing_bytes() {
        let first = GuiToDriver::GetSettings;
        let second = GuiToDriver::SubscribeEvents;
        let mut stream = encode_frame(&first).unwrap();
        stream.extend_from_slice(&encode_frame(&second).unwrap());

        let (m1, consumed): (GuiToDriver, usize) = decode_frame(&stream).unwrap().unwrap();
        assert_eq!(m1, first);
        let (m2, _): (GuiToDriver, usize) = decode_frame(&stream[consumed..]).unwrap().unwrap();
        assert_eq!(m2, second);
    }

    #[test]
    fn decode_rejects_oversized_length_prefix() {
        let mut buf = (MAX_FRAME_LEN + 1).to_le_bytes().to_vec();
        buf.extend_from_slice(&[0u8; 8]);
        let res: Result<Option<(GuiToDriver, usize)>, FrameError> = decode_frame(&buf);
        assert!(matches!(res, Err(FrameError::TooLong(_))));
    }
}
