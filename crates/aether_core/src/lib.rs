//! Aether engine core: app lifecycle, fixed ticks, ids, events, deterministic RNG.
//!
//! This crate is intentionally free of rendering, physics, and transport
//! dependencies so that servers, agents, and tests compile fast and run
//! headless anywhere.

pub mod events;
pub mod id;
pub mod rng;
pub mod tick;
pub mod time;

pub use events::EventQueue;
pub use id::{EntityId, IdAllocator};
pub use rng::SeededRng;
pub use tick::{FixedTicker, Tick};
pub use time::SimTime;

/// Engine semantic identity. Games log this at boot for reproducibility.
pub const ENGINE_NAME: &str = "aether";
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
