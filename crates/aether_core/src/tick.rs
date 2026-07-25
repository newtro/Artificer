use serde::{Deserialize, Serialize};

/// A discrete simulation tick. Monotonic, starts at 0.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct Tick(pub u64);

impl Tick {
    pub fn next(self) -> Tick {
        Tick(self.0 + 1)
    }

    /// Seconds elapsed since tick 0 at the given rate.
    pub fn as_seconds(self, tick_rate: f64) -> f64 {
        self.0 as f64 / tick_rate
    }
}

impl std::fmt::Display for Tick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "t{}", self.0)
    }
}

/// Fixed-timestep accumulator. Frame loops feed it real elapsed seconds;
/// it answers how many fixed ticks must run to stay caught up.
///
/// A hard cap on ticks-per-advance prevents the spiral of death after a
/// long stall (window drag, tab background, debugger pause).
#[derive(Debug, Clone)]
pub struct FixedTicker {
    dt: f64,
    accumulator: f64,
    max_ticks_per_advance: u32,
    tick: Tick,
}

impl FixedTicker {
    pub fn new(tick_rate: f64) -> Self {
        assert!(tick_rate > 0.0, "tick rate must be positive");
        Self {
            dt: 1.0 / tick_rate,
            accumulator: 0.0,
            max_ticks_per_advance: 8,
            tick: Tick(0),
        }
    }

    pub fn with_max_ticks_per_advance(mut self, max: u32) -> Self {
        self.max_ticks_per_advance = max.max(1);
        self
    }

    /// Fixed delta-time in seconds.
    pub fn dt(&self) -> f64 {
        self.dt
    }

    pub fn dt_f32(&self) -> f32 {
        self.dt as f32
    }

    /// The tick that will run next.
    pub fn current_tick(&self) -> Tick {
        self.tick
    }

    /// Feed elapsed wall-clock seconds; returns the ticks to simulate now.
    /// Excess backlog beyond the cap is dropped (time dilation, not death).
    pub fn advance(&mut self, elapsed_seconds: f64) -> u32 {
        self.accumulator += elapsed_seconds.max(0.0);
        let mut ticks = (self.accumulator / self.dt) as u32;
        if ticks > self.max_ticks_per_advance {
            ticks = self.max_ticks_per_advance;
            // Drop the un-simulated backlog so we do not chase it forever.
            self.accumulator = 0.0;
        } else {
            self.accumulator -= ticks as f64 * self.dt;
        }
        self.tick.0 += ticks as u64;
        ticks
    }

    /// Interpolation alpha in [0,1) between the last simulated tick and the
    /// next, for smooth rendering between fixed steps.
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.dt).clamp(0.0, 1.0) as f32
    }

    /// Force the ticker to a specific tick (server resync / replay).
    pub fn reset_to(&mut self, tick: Tick) {
        self.tick = tick;
        self.accumulator = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_exact_ticks() {
        let mut t = FixedTicker::new(30.0);
        assert_eq!(t.advance(1.0 / 30.0), 1);
        assert_eq!(t.advance(2.0 / 30.0), 2);
        assert_eq!(t.current_tick(), Tick(3));
    }

    #[test]
    fn caps_backlog() {
        let mut t = FixedTicker::new(30.0).with_max_ticks_per_advance(4);
        assert_eq!(t.advance(10.0), 4);
        // Backlog dropped: a normal frame afterwards yields normal ticks.
        assert_eq!(t.advance(1.0 / 30.0), 1);
    }

    #[test]
    fn fractional_frames_carry() {
        let mut t = FixedTicker::new(60.0);
        assert_eq!(t.advance(0.5 / 60.0), 0);
        assert_eq!(t.advance(0.6 / 60.0), 1);
    }
}
