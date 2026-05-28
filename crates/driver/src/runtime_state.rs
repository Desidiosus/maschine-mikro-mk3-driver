use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Debug, Default, Clone)]
pub struct RuntimeState {
    pub encoder_absolute: Arc<AtomicU8>,
}

impl RuntimeState {
    pub fn encoder_value(&self) -> u8 {
        self.encoder_absolute.load(Ordering::Relaxed)
    }

    pub fn set_encoder_value(&self, v: u8) {
        self.encoder_absolute.store(v, Ordering::Relaxed);
    }
}
