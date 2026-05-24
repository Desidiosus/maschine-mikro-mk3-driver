use hidapi::HidError;
use std::fmt;

/// Categorised driver runtime error: HID I/O, settings, MIDI, or bridge.
#[derive(Debug)]
pub enum DriverError {
    Hid(HidError),
    Settings(String),
    Midi(String),
    Bridge(String),
}

pub type DriverResult<T> = Result<T, DriverError>;

impl fmt::Display for DriverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hid(err) => write!(f, "HID I/O error: {err}"),
            Self::Settings(message) => write!(f, "invalid settings: {message}"),
            Self::Midi(message) => write!(f, "MIDI error: {message}"),
            Self::Bridge(message) => write!(f, "MIDI bridge error: {message}"),
        }
    }
}

impl std::error::Error for DriverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Hid(err) => Some(err),
            _ => None,
        }
    }
}

impl From<HidError> for DriverError {
    fn from(value: HidError) -> Self {
        Self::Hid(value)
    }
}
