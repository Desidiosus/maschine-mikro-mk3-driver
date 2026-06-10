//! The top-level `Message` type for the GUI application.

use std::sync::mpsc::Sender;

use protocol::{DriverToGui, GuiToDriver};

#[derive(Debug, Clone)]
pub enum Message {
    /// Connection established; carries the channel to send requests to the driver.
    Ready(Sender<GuiToDriver>),
    /// A frame arrived from the driver.
    Frame(DriverToGui),
    Disconnected,
    Error(String),
    /// Periodic redraw tick.
    Tick,
}
