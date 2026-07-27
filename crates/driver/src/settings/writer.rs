use std::sync::{Arc, Mutex};

/// Serializes every settings write so concurrent writers cannot lose a
/// read-modify-write against `SharedSettings`.
pub type WriteLock = Arc<Mutex<()>>;

pub fn new_write_lock() -> WriteLock {
    Arc::new(Mutex::new(()))
}
