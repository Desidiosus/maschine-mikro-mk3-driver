use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::settings::Settings;

/// Hot-swappable, lock-free-read handle to the live `Settings`.
///
/// Readers (the runtime loop, the backend, and the midir callback thread) call
/// `.load()`; the single writer (`apply_delta`, driven by the IPC thread) calls
/// `.store(Arc::new(merged))`. The read-modify-write in `apply_delta` is not
/// atomic, so exactly one writer must exist — concurrent `apply_delta` calls
/// would risk a lost update (last store wins).
pub type SharedSettings = Arc<ArcSwap<Settings>>;

/// Wrap an owned `Settings` in a fresh shared handle.
pub fn new_shared(settings: Settings) -> SharedSettings {
    Arc::new(ArcSwap::from_pointee(settings))
}
