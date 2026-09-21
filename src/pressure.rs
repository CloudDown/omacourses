use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

/// Dernière pression stylet (bits d'un f32). 0 = inconnue.
#[derive(Clone)]
pub struct Pressure {
    bits: Arc<AtomicU32>,
}

impl Pressure {
    pub fn start() -> Self {
        Self {
            bits: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn latest(&self) -> Option<f32> {
        let v = self.bits.load(Ordering::Relaxed);
        if v == 0 {
            None
        } else {
            Some(f32::from_bits(v).clamp(0.08, 1.0))
        }
    }

    pub fn push_touch(&self, force: f32) {
        if force > 0.02 {
            self.bits.store(force.to_bits(), Ordering::Relaxed);
        }
    }
}
