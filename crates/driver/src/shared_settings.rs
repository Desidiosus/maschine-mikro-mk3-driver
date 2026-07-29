use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::settings::Settings;

/// Hot-swappable, lock-free-read handle to the live `Settings`.
///
/// Readers (the runtime loop, the backend, and the midir callback thread) call
/// `.load()`; writers (`apply_delta` on the IPC thread and the page-apply writer
/// thread) call `.store(Arc::new(merged))`. The read-modify-write in `apply_delta`
/// is not atomic, so every writer must serialize through the shared `WriteLock`
/// (`settings::writer::WriteLock`); two concurrent unlocked `apply_delta` calls
/// would risk a lost update (last store wins).
pub type SharedSettings = Arc<ArcSwap<Settings>>;

/// Wrap an owned `Settings` in a fresh shared handle.
pub fn new_shared(settings: Settings) -> SharedSettings {
    Arc::new(ArcSwap::from_pointee(settings))
}
