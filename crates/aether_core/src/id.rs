use serde::{Deserialize, Serialize};

/// Stable simulation entity id, unique within one world.
///
/// Not a renderer entity: the render adapter maps these to its own ids.
/// 0 is reserved as "invalid" so the id fits `Option`-free wire formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub u64);

impl EntityId {
    pub const INVALID: EntityId = EntityId(0);

    pub fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl std::fmt::Display for EntityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "e{}", self.0)
    }
}

/// Monotonic id allocator. Deterministic: allocation order fully defines ids,
/// which keeps replays and server/client agreement stable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdAllocator {
    next: u64,
}

impl Default for IdAllocator {
    fn default() -> Self {
        Self { next: 1 }
    }
}

impl IdAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start allocating from a given floor (e.g. after loading persisted
    /// entities whose ids must remain stable).
    pub fn starting_at(next: u64) -> Self {
        Self { next: next.max(1) }
    }

    pub fn allocate(&mut self) -> EntityId {
        let id = EntityId(self.next);
        self.next += 1;
        id
    }

    /// Ensure future allocations never collide with an externally loaded id.
    pub fn reserve(&mut self, id: EntityId) {
        if id.0 >= self.next {
            self.next = id.0 + 1;
        }
    }

    pub fn peek_next(&self) -> u64 {
        self.next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocates_monotonic_from_one() {
        let mut a = IdAllocator::new();
        assert_eq!(a.allocate(), EntityId(1));
        assert_eq!(a.allocate(), EntityId(2));
    }

    #[test]
    fn reserve_prevents_collision() {
        let mut a = IdAllocator::new();
        a.reserve(EntityId(10));
        assert_eq!(a.allocate(), EntityId(11));
    }
}
