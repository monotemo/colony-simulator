//! Entity identity.
//!
//! For this first slice the only concrete entity is the [`crate::bee::Bee`],
//! but identity is factored out here so additional entity kinds (predators,
//! plants, the hive itself) can share the same id space later.

use serde::{Deserialize, Serialize};

/// A stable, unique identifier for an entity within a [`crate::world::World`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntityId(pub u64);

impl EntityId {
    pub fn value(self) -> u64 {
        self.0
    }
}

/// Hands out monotonically increasing [`EntityId`]s.
///
/// Lives on the [`crate::world::World`] so spawning never reuses an id.
#[derive(Debug, Clone, Default)]
pub struct IdAllocator {
    next: u64,
}

impl IdAllocator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self) -> EntityId {
        let id = EntityId(self.next);
        self.next += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_increasing() {
        let mut alloc = IdAllocator::new();
        let a = alloc.alloc();
        let b = alloc.alloc();
        assert_ne!(a, b);
        assert!(b > a);
    }
}
