pub mod endpoint;
pub mod frame;
pub mod messages;

pub use endpoint::socket_path;
pub use messages::{ControlRef, DriverToGui, GuiToDriver, MidiDir};
