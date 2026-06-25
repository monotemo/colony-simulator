//! The world: physical bounds, resources, and the entities living inside it.

use serde::{Deserialize, Serialize};

use crate::bee::Bee;
use crate::entity::{EntityId, IdAllocator};
use crate::math::Vec3;

/// The box-shaped extent of the world, in world units. The origin is the
/// top-left-front corner; valid positions satisfy `0 <= x <= width`,
/// `0 <= y <= height`, and `0 <= z <= depth`.
///
/// `depth` is the third (vertical/flight) axis. Until flight behavior lands the
/// world is seeded flat at `z = 0`, but bounds carry real depth so the
/// integration loop confines bees in 3D from the start.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub width: f64,
    pub height: f64,
    pub depth: f64,
}

impl Bounds {
    pub const fn new(width: f64, height: f64, depth: f64) -> Self {
        Self {
            width,
            height,
            depth,
        }
    }
}

/// Kinds of resource a bee may eventually interact with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// A nectar source (flower). Rendered now; foraging comes in a later slice.
    Nectar,
}

/// A static resource node in the world.
#[derive(Debug, Clone, PartialEq)]
pub struct Resource {
    pub id: EntityId,
    pub position: Vec3,
    pub kind: ResourceKind,
}

/// The simulation world: bounds, the bee population, and resource nodes.
#[derive(Debug, Clone)]
pub struct World {
    pub bounds: Bounds,
    pub bees: Vec<Bee>,
    pub resources: Vec<Resource>,
    ids: IdAllocator,
}

impl World {
    /// Create an empty world with the given bounds.
    pub fn empty(bounds: Bounds) -> Self {
        Self {
            bounds,
            bees: Vec::new(),
            resources: Vec::new(),
            ids: IdAllocator::new(),
        }
    }

    /// Spawn a bee at `position` with `velocity`, allocating it a fresh id.
    pub fn spawn_bee(&mut self, position: Vec3, velocity: Vec3) -> EntityId {
        let id = self.ids.alloc();
        self.bees.push(Bee::new(id, position, velocity));
        id
    }

    /// Add a resource node at `position`, allocating it a fresh id.
    pub fn add_resource(&mut self, position: Vec3, kind: ResourceKind) -> EntityId {
        let id = self.ids.alloc();
        self.resources.push(Resource {
            id,
            position,
            kind,
        });
        id
    }

    /// Build the default seeded starting world: 24 bees with deterministic,
    /// varied velocities plus a few nectar sources. Deterministic so the
    /// initial state is reproducible without pulling in an RNG dependency.
    pub fn seeded() -> Self {
        Self::seeded_with_count(24)
    }

    /// Build a seeded world with `bee_count` bees. The placement and velocity
    /// math is parameterized on `t = i / bee_count`, so the layout shape is
    /// the same at any population — at `bee_count == 24` it is byte-identical
    /// to [`World::seeded`]. Exists so benchmarks and scale tests can stress
    /// arbitrary populations without an RNG; production still uses `seeded()`.
    pub fn seeded_with_count(bee_count: usize) -> Self {
        // Depth is sized for the eventual flight volume; bees and resources are
        // seeded flat at z = 0 until flight behavior lands, so visuals are
        // unchanged while the third axis exists for real in the geometry.
        let bounds = Bounds::new(800.0, 600.0, 400.0);
        let mut world = World::empty(bounds);

        for i in 0..bee_count {
            let t = i as f64 / bee_count as f64;
            // Spread starting positions across the interior, on the z = 0 plane.
            let position = Vec3::new(
                bounds.width * (0.2 + 0.6 * fract(t * 7.0)),
                bounds.height * (0.2 + 0.6 * fract(t * 3.0)),
                0.0,
            );
            // Varied directions at a steady speed, in the z = 0 plane for now.
            let angle = t * std::f64::consts::TAU * 3.0;
            let speed = 60.0 * fract(t + 0.5);
            let velocity = Vec3::new(angle.cos() * speed, angle.sin() * speed, 0.0);
            world.spawn_bee(position, velocity);
        }

        for (fx, fy) in [(0.25, 0.3), (0.7, 0.25), (0.5, 0.75), (0.8, 0.7)] {
            world.add_resource(
                Vec3::new(bounds.width * fx, bounds.height * fy, 0.0),
                ResourceKind::Nectar,
            );
        }

        world
    }

    /// Advance every bee by one fixed timestep of `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        for bee in &mut self.bees {
            bee.step(dt, self.bounds);
        }
    }
}

/// Fractional part of `x`, in `[0, 1)`.
fn fract(x: f64) -> f64 {
    x - x.floor()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_assigns_unique_ids() {
        let mut world = World::empty(Bounds::new(100.0, 100.0, 100.0));
        let a = world.spawn_bee(Vec3::ZERO, Vec3::ZERO);
        let b = world.spawn_bee(Vec3::ZERO, Vec3::ZERO);
        let r = world.add_resource(Vec3::ZERO, ResourceKind::Nectar);
        assert_ne!(a, b);
        assert_ne!(b, r);
    }

    #[test]
    fn seeded_matches_seeded_with_count_24() {
        // `seeded()` must stay byte-identical to the parameterized builder at
        // its production population, so adding the benchmarking hook can't
        // silently perturb the default world.
        let default = World::seeded();
        let explicit = World::seeded_with_count(24);
        assert_eq!(default.bees, explicit.bees);
        assert_eq!(default.resources, explicit.resources);
        assert_eq!(default.bounds, explicit.bounds);
    }

    #[test]
    fn seeded_world_has_population_and_resources() {
        let world = World::seeded();
        assert!(!world.bees.is_empty());
        assert!(!world.resources.is_empty());
        // Every bee starts inside the bounds, flat on the z = 0 plane for now.
        for bee in &world.bees {
            assert!(bee.position.x >= 0.0 && bee.position.x <= world.bounds.width);
            assert!(bee.position.y >= 0.0 && bee.position.y <= world.bounds.height);
            assert_eq!(bee.position.z, 0.0);
        }
    }

    #[test]
    fn step_keeps_all_bees_in_bounds() {
        let mut world = World::seeded();
        for _ in 0..1000 {
            world.step(1.0 / 30.0);
        }
        for bee in &world.bees {
            assert!(bee.position.x >= 0.0 && bee.position.x <= world.bounds.width);
            assert!(bee.position.y >= 0.0 && bee.position.y <= world.bounds.height);
            assert!(bee.position.z >= 0.0 && bee.position.z <= world.bounds.depth);
        }
    }
}
