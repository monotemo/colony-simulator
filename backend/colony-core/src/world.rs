//! The world: physical bounds, resources, and the entities living inside it.

use serde::{Deserialize, Serialize};

use crate::bee::Bee;
use crate::entity::{EntityId, IdAllocator};
use crate::math::Vec3;

/// Effective body radius of a bee, in world units. Bees are treated as spheres
/// of this radius for collision avoidance.
const BEE_RADIUS: f64 = 6.0;

/// Range at which two bees start steering apart — two body radii, i.e. the
/// distance at which their spheres touch. Inside this the separation force
/// ramps up linearly to its full strength at full overlap.
const SEPARATION_RADIUS: f64 = 2.0 * BEE_RADIUS;

/// Peak separation acceleration (world units / s²) applied when two bees are
/// fully overlapping. Falls off linearly to zero at `SEPARATION_RADIUS`.
const SEPARATION_STRENGTH: f64 = 400.0;

/// Ceiling on a bee's speed (world units / s) after steering, so accumulated
/// separation pushes can't fling a bee arbitrarily fast.
const MAX_SPEED: f64 = 120.0;

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
    ///
    /// Runs in two strict passes so the result is independent of iteration
    /// order and therefore deterministic:
    /// 1. Read every bee's position and compute each one's separation
    ///    acceleration (read-only — see [`World::separation_accelerations`]).
    /// 2. Fold that acceleration into velocity, clamp to `MAX_SPEED`, then let
    ///    each bee integrate and bounce off the walls as before.
    ///
    /// Steering only ever nudges velocity; [`Bee::step`] stays the sole
    /// authority that confines a bee to the world, so avoidance can never eject
    /// one through a wall.
    pub fn step(&mut self, dt: f64) {
        let accelerations = self.separation_accelerations();
        for (bee, accel) in self.bees.iter_mut().zip(&accelerations) {
            let mut velocity = bee.velocity.add(accel.scale(dt));
            let speed_squared = velocity.length_squared();
            if speed_squared > MAX_SPEED * MAX_SPEED {
                velocity = velocity.normalized().scale(MAX_SPEED);
            }
            bee.velocity = velocity;
            bee.step(dt, self.bounds);
        }
    }

    /// Compute the separation (collision-avoidance) acceleration for every bee,
    /// returned in bee order. Pure and read-only: it touches no mutable state,
    /// which is what lets [`World::step`] apply the whole batch atomically.
    ///
    /// Each unordered pair of bees closer than `SEPARATION_RADIUS` contributes
    /// an equal-and-opposite push along the line between them, ramping linearly
    /// from zero at the radius to `SEPARATION_STRENGTH` at full overlap. Walking
    /// the pairs in a fixed `i < j` order pins the floating-point summation
    /// order, so the totals are bit-for-bit reproducible. Bees with no close
    /// neighbour accumulate [`Vec3::ZERO`].
    ///
    /// This is the naive O(n²) reference. A spatial-grid broad phase can replace
    /// the inner loop while preserving this exact contract.
    fn separation_accelerations(&self) -> Vec<Vec3> {
        let mut accelerations = vec![Vec3::ZERO; self.bees.len()];
        let radius_squared = SEPARATION_RADIUS * SEPARATION_RADIUS;

        for i in 0..self.bees.len() {
            for j in (i + 1)..self.bees.len() {
                let offset = self.bees[i].position.sub(self.bees[j].position);
                let distance_squared = offset.length_squared();
                // Skip pairs out of range, and coincident bees (offset has no
                // direction to push along — they stay put until something else
                // nudges them apart).
                if distance_squared >= radius_squared || distance_squared == 0.0 {
                    continue;
                }
                let distance = distance_squared.sqrt();
                let closeness = (SEPARATION_RADIUS - distance) / SEPARATION_RADIUS;
                let push = offset.scale(SEPARATION_STRENGTH * closeness / distance);
                accelerations[i] = accelerations[i].add(push);
                accelerations[j] = accelerations[j].sub(push);
            }
        }

        accelerations
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

    #[test]
    fn lone_bee_has_zero_separation() {
        // Nothing to avoid, so no steering force.
        let mut world = World::empty(Bounds::new(100.0, 100.0, 100.0));
        world.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        assert_eq!(world.separation_accelerations(), vec![Vec3::ZERO]);
    }

    #[test]
    fn overlapping_bees_steer_apart() {
        // Two stationary bees well inside the separation radius should be
        // pushed further apart after a step.
        let mut world = World::empty(Bounds::new(100.0, 100.0, 100.0));
        world.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        world.spawn_bee(Vec3::new(54.0, 50.0, 0.0), Vec3::ZERO);
        let before = world.bees[0].position.distance_squared(world.bees[1].position);
        world.step(1.0 / 30.0);
        let after = world.bees[0].position.distance_squared(world.bees[1].position);
        assert!(after > before, "bees should separate: {before} -> {after}");
    }

    #[test]
    fn separation_is_equal_opposite_and_order_independent() {
        // A close pair pushes with equal and opposite force...
        let mut world = World::empty(Bounds::new(100.0, 100.0, 100.0));
        world.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        world.spawn_bee(Vec3::new(55.0, 52.0, 0.0), Vec3::ZERO);
        let accel = world.separation_accelerations();
        assert_eq!(accel[0], accel[1].scale(-1.0));
        assert_ne!(accel[0], Vec3::ZERO);

        // ...and swapping storage order just swaps the results: the force a bee
        // feels doesn't depend on where it sits in the Vec.
        let mut swapped = World::empty(Bounds::new(100.0, 100.0, 100.0));
        swapped.spawn_bee(Vec3::new(55.0, 52.0, 0.0), Vec3::ZERO);
        swapped.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        let swapped_accel = swapped.separation_accelerations();
        assert_eq!(accel[0], swapped_accel[1]);
        assert_eq!(accel[1], swapped_accel[0]);
    }

    #[test]
    fn avoidance_keeps_a_dense_swarm_in_bounds() {
        // A large, deliberately crowded population stresses the separation
        // forces; the wall-bounce must still confine everyone.
        let mut world = World::seeded_with_count(500);
        for _ in 0..300 {
            world.step(1.0 / 30.0);
        }
        for bee in &world.bees {
            assert!(bee.position.x >= 0.0 && bee.position.x <= world.bounds.width);
            assert!(bee.position.y >= 0.0 && bee.position.y <= world.bounds.height);
            assert!(bee.position.z >= 0.0 && bee.position.z <= world.bounds.depth);
        }
    }
}
