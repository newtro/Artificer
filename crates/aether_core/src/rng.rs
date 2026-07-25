//! Deterministic, dependency-free RNG (xoshiro256** seeded via splitmix64).
//!
//! Simulation code must use this — never `rand::thread_rng` or platform
//! entropy — so scenarios, replays, and server runs stay reproducible.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeededRng {
    state: [u64; 4],
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl SeededRng {
    pub fn new(seed: u64) -> Self {
        let mut sm = seed;
        Self {
            state: [
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
                splitmix64(&mut sm),
            ],
        }
    }

    /// Derive an independent stream (e.g. per subsystem or per entity) that
    /// will not correlate with the parent sequence.
    pub fn fork(&mut self, stream: u64) -> SeededRng {
        let mixed = self.next_u64() ^ stream.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        SeededRng::new(mixed)
    }

    pub fn next_u64(&mut self) -> u64 {
        let result = self.state[1]
            .wrapping_mul(5)
            .rotate_left(7)
            .wrapping_mul(9);
        let t = self.state[1] << 17;
        self.state[2] ^= self.state[0];
        self.state[3] ^= self.state[1];
        self.state[1] ^= self.state[2];
        self.state[0] ^= self.state[3];
        self.state[2] ^= t;
        self.state[3] = self.state[3].rotate_left(45);
        result
    }

    /// Uniform in [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        self.next_f64() as f32
    }

    /// Uniform in [lo, hi).
    pub fn range_f32(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    /// Uniform in [lo, hi) for integers. `hi` must be > `lo`.
    pub fn range_i64(&mut self, lo: i64, hi: i64) -> i64 {
        assert!(hi > lo, "empty integer range");
        let span = (hi - lo) as u64;
        lo + (self.next_u64() % span) as i64
    }

    /// Uniform index in [0, len). `len` must be non-zero.
    pub fn index(&mut self, len: usize) -> usize {
        assert!(len > 0, "index() on empty collection");
        (self.next_u64() % len as u64) as usize
    }

    /// True with probability `p` (clamped to [0,1]).
    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p.clamp(0.0, 1.0)
    }

    /// Fisher-Yates shuffle, deterministic given rng state.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.index(i + 1);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_same_seed() {
        let mut a = SeededRng::new(42);
        let mut b = SeededRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = SeededRng::new(1);
        let mut b = SeededRng::new(2);
        let same = (0..64).filter(|_| a.next_u64() == b.next_u64()).count();
        assert!(same < 4);
    }

    #[test]
    fn f32_in_unit_range() {
        let mut r = SeededRng::new(7);
        for _ in 0..1000 {
            let v = r.next_f32();
            assert!((0.0..1.0).contains(&v));
        }
    }

    #[test]
    fn forked_streams_are_independent_and_deterministic() {
        let mut parent1 = SeededRng::new(9);
        let mut parent2 = SeededRng::new(9);
        let mut f1 = parent1.fork(3);
        let mut f2 = parent2.fork(3);
        for _ in 0..32 {
            assert_eq!(f1.next_u64(), f2.next_u64());
        }
    }
}
