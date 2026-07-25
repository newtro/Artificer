//! Headless app lifecycle: a deterministic fixed-tick harness composed of
//! plugins. Servers, scenario runners, and agents build on this; rendering
//! clients use `aether_render`'s frame loop and drive their own `FixedTicker`.

use crate::rng::SeededRng;
use crate::tick::{FixedTicker, Tick};
use crate::time::SimTime;

/// Shared context passed to every plugin each fixed update.
pub struct SimContext {
    pub time: SimTime,
    pub rng: SeededRng,
    /// True once a plugin has requested shutdown; remaining plugins still see
    /// the current tick, then the loop stops.
    stop_requested: bool,
}

impl SimContext {
    pub fn request_stop(&mut self) {
        self.stop_requested = true;
    }

    pub fn stop_requested(&self) -> bool {
        self.stop_requested
    }
}

/// A unit of headless simulation behavior with a stable identity.
pub trait SimPlugin {
    fn name(&self) -> &'static str;

    /// Called once before the first tick.
    fn init(&mut self, _ctx: &mut SimContext) {}

    /// Called once per fixed tick, in registration order.
    fn fixed_update(&mut self, ctx: &mut SimContext);
}

/// Deterministic headless application: fixed ticker + ordered plugins.
pub struct SimApp {
    ticker: FixedTicker,
    plugins: Vec<Box<dyn SimPlugin>>,
    ctx: SimContext,
    initialized: bool,
}

impl SimApp {
    pub fn new(tick_rate: f64, seed: u64) -> Self {
        let ticker = FixedTicker::new(tick_rate);
        let dt = ticker.dt();
        Self {
            ticker,
            plugins: Vec::new(),
            ctx: SimContext {
                time: SimTime { tick: Tick(0), dt },
                rng: SeededRng::new(seed),
                stop_requested: false,
            },
            initialized: false,
        }
    }

    pub fn add_plugin(mut self, plugin: impl SimPlugin + 'static) -> Self {
        self.plugins.push(Box::new(plugin));
        self
    }

    pub fn current_tick(&self) -> Tick {
        self.ctx.time.tick
    }

    pub fn dt(&self) -> f64 {
        self.ticker.dt()
    }

    fn ensure_init(&mut self) {
        if !self.initialized {
            for p in self.plugins.iter_mut() {
                log::debug!("init plugin: {}", p.name());
                p.init(&mut self.ctx);
            }
            self.initialized = true;
        }
    }

    /// Run exactly one fixed tick through all plugins.
    pub fn step(&mut self) {
        self.ensure_init();
        for p in self.plugins.iter_mut() {
            p.fixed_update(&mut self.ctx);
        }
        self.ctx.time.tick = self.ctx.time.tick.next();
    }

    /// Run `n` ticks or until a plugin requests stop. Returns ticks executed.
    pub fn run_ticks(&mut self, n: u64) -> u64 {
        self.ensure_init();
        let mut executed = 0;
        for _ in 0..n {
            if self.ctx.stop_requested {
                break;
            }
            self.step();
            executed += 1;
        }
        executed
    }

    /// Feed wall-clock elapsed seconds (real-time headless loops).
    /// Returns ticks executed.
    pub fn advance_realtime(&mut self, elapsed_seconds: f64) -> u32 {
        self.ensure_init();
        let ticks = self.ticker.advance(elapsed_seconds);
        for _ in 0..ticks {
            if self.ctx.stop_requested {
                break;
            }
            self.step();
        }
        ticks
    }

    pub fn stop_requested(&self) -> bool {
        self.ctx.stop_requested
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter {
        count: u64,
        stop_at: u64,
    }

    impl SimPlugin for Counter {
        fn name(&self) -> &'static str {
            "counter"
        }

        fn fixed_update(&mut self, ctx: &mut SimContext) {
            self.count += 1;
            if self.count >= self.stop_at {
                ctx.request_stop();
            }
        }
    }

    #[test]
    fn runs_requested_ticks() {
        let mut app = SimApp::new(30.0, 1).add_plugin(Counter {
            count: 0,
            stop_at: u64::MAX,
        });
        assert_eq!(app.run_ticks(10), 10);
        assert_eq!(app.current_tick(), Tick(10));
    }

    #[test]
    fn stop_request_halts_loop() {
        let mut app = SimApp::new(30.0, 1).add_plugin(Counter {
            count: 0,
            stop_at: 3,
        });
        let executed = app.run_ticks(100);
        assert_eq!(executed, 3);
    }
}
