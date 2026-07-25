use crate::tick::Tick;
use serde::{Deserialize, Serialize};

/// Snapshot of simulation time handed to plugins each fixed update.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SimTime {
    /// The tick currently being simulated.
    pub tick: Tick,
    /// Fixed delta seconds per tick.
    pub dt: f64,
}

impl SimTime {
    pub fn seconds_since_start(&self) -> f64 {
        self.tick.0 as f64 * self.dt
    }

    pub fn dt_f32(&self) -> f32 {
        self.dt as f32
    }
}
