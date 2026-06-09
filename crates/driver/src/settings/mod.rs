//! Re-export shim. The settings schema lives in the `settings` crate; the
//! driver keeps persistence (I/O policy) here in `persist`.

pub use ::settings::*;

pub mod persist;
pub use persist::{load_xdg, resolve_and_load_settings};
