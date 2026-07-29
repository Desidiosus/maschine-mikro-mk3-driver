//! Re-export shim. The settings schema lives in the `settings` crate; the
//! driver keeps persistence (I/O policy) here in `persist`.

pub use ::settings::*;

pub mod persist;
pub use persist::{LoadedConfig, load_config, load_xdg};

pub mod writer;
pub use writer::{PageApplyMsg, WriteLock, new_write_lock, spawn_page_apply_writer};
