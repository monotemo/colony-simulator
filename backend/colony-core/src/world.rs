//! The world: physical bounds, resources, and the entities living inside it.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use serde::{Deserialize, Serialize};

use crate::bee::{Bee, BeeState};
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

/// Energy at or below which a wandering bee peels off to forage, provided there
/// is nectar to seek. Deliberately above the rest threshold in `bee.rs`: with
/// food available a bee refuels at a flower rather than dropping to rest, so
/// `Resting` becomes the fallback for when no nectar is reachable.
const FORAGE_ENERGY_THRESHOLD: f64 = 0.4;

/// Energy at which a feeding bee is satisfied and returns to wandering.
const FORAGE_SATISFIED_ENERGY: f64 = 0.9;

/// Distance from a nectar source at which a bee counts as "at" it and feeds.
const FORAGE_REACH_RADIUS: f64 = 20.0;

/// Energy a feeding bee gains per second at a nectar source. Set well above the
/// active drain in `bee.rs` so net energy climbs while feeding even though the
/// flight cost still applies.
const FORAGE_REFILL_PER_SECOND: f64 = 0.6;

/// Steering acceleration (world units / s²) pulling a foraging bee toward its
/// target nectar — a magnitude-based seek, mirroring the separation push.
const FORAGE_SEEK_STRENGTH: f64 = 200.0;

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
    /// Reusable scratch for the per-tick separation broad phase. Holds no
    /// simulation state — it is cleared and rebuilt every [`World::step`] — so
    /// it never participates in equality or determinism, only in keeping the
    /// hot path allocation-free.
    grid: SpatialGrid,
}

impl World {
    /// Create an empty world with the given bounds.
    pub fn empty(bounds: Bounds) -> Self {
        Self {
            bounds,
            bees: Vec::new(),
            resources: Vec::new(),
            ids: IdAllocator::new(),
            grid: SpatialGrid::default(),
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

            // Stagger starting energy across the colony. Identical full reserves
            // would drain in lockstep, so every bee would hit the rest threshold
            // on the same tick and the behavior breakdown would be all-or-nothing.
            // Spreading the initial energy desynchronizes the rest/wake cycle so a
            // live mix of wandering and resting bees is always on screen. Like the
            // position and velocity above, it is parameterized on the bee index —
            // deterministic, no RNG.
            world.bees.last_mut().expect("just spawned a bee").energy = 0.5 + 0.5 * fract(t * 5.0);
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
    ///    acceleration (read-only — see [`SpatialGrid::separation_accelerations`]).
    /// 2. Fold that acceleration into velocity, clamp to `MAX_SPEED`, then let
    ///    each bee integrate and bounce off the walls as before.
    ///
    /// Steering only ever nudges velocity; [`Bee::step`] stays the sole
    /// authority that confines a bee to the world, so neither avoidance nor the
    /// foraging seek can eject one through a wall.
    pub fn step(&mut self, dt: f64) {
        // Disjoint borrows: the grid fills its own buffer from `&self.bees`,
        // then the integration loop mutates `self.bees` while reading the
        // returned slice, `self.resources`, and `self.bounds`.
        let accelerations = self.grid.separation_accelerations(&self.bees);
        for (bee, &separation) in self.bees.iter_mut().zip(accelerations) {
            // Foraging overlay: a hungry bee peels off to seek the nearest
            // nectar and refuels on arrival, contributing a seek force folded in
            // alongside separation. It reads only this bee and the immutable
            // resources — never other bees — so it can't perturb the separation
            // pass's ordering and determinism is preserved.
            let seek = forage(bee, &self.resources, dt);

            let mut velocity = bee.velocity.add(separation.add(seek).scale(dt));
            let speed_squared = velocity.length_squared();
            if speed_squared > MAX_SPEED * MAX_SPEED {
                velocity = velocity.normalized().scale(MAX_SPEED);
            }
            bee.velocity = velocity;
            bee.step(dt, self.bounds);
        }
    }

    /// Separation accelerations for every bee, in bee order — a test-facing
    /// wrapper over [`SpatialGrid::separation_accelerations`] that hands back an
    /// owned copy so the borrow on the grid is released immediately.
    #[cfg(test)]
    fn separation_accelerations(&mut self) -> Vec<Vec3> {
        self.grid.separation_accelerations(&self.bees).to_vec()
    }

    /// Naive O(n²) all-pairs separation: the readable reference the grid is
    /// validated against (`grid_matches_naive`). Walks every `i < j` pair in a
    /// fixed order, which is the summation order the grid is built to reproduce.
    #[cfg(test)]
    fn separation_accelerations_all_pairs(&self) -> Vec<Vec3> {
        let mut accelerations = vec![Vec3::ZERO; self.bees.len()];
        for i in 0..self.bees.len() {
            for j in (i + 1)..self.bees.len() {
                if let Some(push) =
                    separation_push(self.bees[i].position, self.bees[j].position)
                {
                    accelerations[i] = accelerations[i].add(push);
                    accelerations[j] = accelerations[j].sub(push);
                }
            }
        }
        accelerations
    }
}

/// Reusable scratch for the separation broad phase. Lives on the [`World`] so
/// the per-tick grid build allocates once and then reuses its buffers across
/// ticks instead of churning the heap 30 times a second. Carries no simulation
/// state: every buffer is cleared and rebuilt at the start of each call.
#[derive(Debug, Clone, Default)]
struct SpatialGrid {
    /// Bee indices bucketed by cell, hashed with the cheap deterministic
    /// [`CellHasher`] rather than SipHash. Cleared each tick — which keeps the
    /// table's capacity so the table itself is not reallocated — then rebuilt
    /// from the live positions.
    buckets: CellMap,
    /// Neighbour candidates for the bee currently being processed.
    candidates: Vec<usize>,
    /// Output accelerations, one per bee, in bee order.
    accelerations: Vec<Vec3>,
}

impl SpatialGrid {
    /// Compute the separation (collision-avoidance) acceleration for every bee
    /// into the reusable `accelerations` buffer and return it.
    ///
    /// Each pair of bees closer than `SEPARATION_RADIUS` contributes an
    /// equal-and-opposite push along the line between them (see
    /// [`separation_push`]). Bees with no close neighbour stay at [`Vec3::ZERO`].
    ///
    /// Broad phase is a uniform grid with cell size `SEPARATION_RADIUS`: each bee
    /// is bucketed by its cell, and only the 3×3×3 block of cells around it can
    /// hold a bee within range, so we never test the far-field pairs that dominate
    /// the O(n²) cost. Within each bee we gather candidate neighbours and process
    /// them in ascending index order, which reproduces the naive all-pairs
    /// summation order exactly — the `grid_matches_naive` test guards that
    /// equivalence. The grid is read only via fixed-key lookups, so the hash's
    /// bucket layout never affects the result: determinism holds.
    ///
    /// The grid is used at every population. Below ~1k bees a plain all-pairs scan
    /// is actually faster in absolute terms, but only by hundreds of nanoseconds
    /// against a 33ms frame budget — far below anything we'd feel — so we keep one
    /// strategy rather than branch on size. The all-pairs scan survives as the
    /// test-only correctness oracle.
    fn separation_accelerations(&mut self, bees: &[Bee]) -> &[Vec3] {
        let n = bees.len();
        // clear() + resize() (not resize() alone) so every slot is reset to
        // ZERO; a bare resize would leave last tick's values in slots [0, n).
        self.accelerations.clear();
        self.accelerations.resize(n, Vec3::ZERO);

        // Drop last tick's entries (retaining table capacity), then re-bucket
        // every bee. Indices are pushed in ascending order, so each bucket stays
        // sorted for free.
        self.buckets.clear();
        for (i, bee) in bees.iter().enumerate() {
            self.buckets.entry(cell_of(bee.position)).or_default().push(i);
        }

        for i in 0..n {
            let (cx, cy, cz) = cell_of(bees[i].position);
            self.candidates.clear();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(bucket) = self.buckets.get(&(cx + dx, cy + dy, cz + dz)) {
                            self.candidates
                                .extend(bucket.iter().copied().filter(|&j| j > i));
                        }
                    }
                }
            }
            // Ascending `j` so the running sums into `accelerations[i]` and the
            // matching subtractions into `accelerations[j]` land in the same
            // order as the naive reference, keeping the totals bit-identical.
            self.candidates.sort_unstable();
            for &j in &self.candidates {
                if let Some(push) = separation_push(bees[i].position, bees[j].position) {
                    self.accelerations[i] = self.accelerations[i].add(push);
                    self.accelerations[j] = self.accelerations[j].sub(push);
                }
            }
        }

        &self.accelerations
    }
}

/// The push that the bee at `a` feels away from the bee at `b` (the bee at `b`
/// feels its negation). `None` when the pair is out of range, or coincident — a
/// zero-length offset has no direction to push along, so such bees stay put
/// until something else nudges them apart. The magnitude ramps linearly from
/// zero at `SEPARATION_RADIUS` to `SEPARATION_STRENGTH` at full overlap.
fn separation_push(a: Vec3, b: Vec3) -> Option<Vec3> {
    let offset = a.sub(b);
    let distance_squared = offset.length_squared();
    if distance_squared >= SEPARATION_RADIUS * SEPARATION_RADIUS || distance_squared == 0.0 {
        return None;
    }
    let distance = distance_squared.sqrt();
    let closeness = (SEPARATION_RADIUS - distance) / SEPARATION_RADIUS;
    Some(offset.scale(SEPARATION_STRENGTH * closeness / distance))
}

/// Foraging behavior for a single bee, run once per tick before integration.
///
/// Drives the entry and exit of [`BeeState::Foraging`] and returns the seek
/// acceleration to fold into this bee's steering (`Vec3::ZERO` when it isn't
/// foraging). A wandering bee that drops to [`FORAGE_ENERGY_THRESHOLD`] commits
/// to foraging if there is nectar to chase; a foraging bee steers toward the
/// nearest source and, once within [`FORAGE_REACH_RADIUS`], feeds until
/// satisfied and returns to wandering — keeping its momentum so it drifts back
/// out naturally.
///
/// Reads only `bee` and the immutable `resources`, never other bees, so it is
/// order-independent and leaves the engine's determinism intact. Nectar is an
/// inexhaustible well for this slice — feeding doesn't deplete it. The active
/// energy drain still lives in [`Bee::step_energy_and_state`]; the refill here
/// is set to outpace it, so net energy rises while feeding.
fn forage(bee: &mut Bee, resources: &[Resource], dt: f64) -> Vec3 {
    // A hungry wanderer commits to foraging, but only if there's nectar to find.
    if bee.state == BeeState::Wandering
        && bee.energy <= FORAGE_ENERGY_THRESHOLD
        && nearest_nectar(bee.position, resources).is_some()
    {
        bee.state = BeeState::Foraging;
    }

    if bee.state != BeeState::Foraging {
        return Vec3::ZERO;
    }

    let Some((target, distance_squared)) = nearest_nectar(bee.position, resources) else {
        // No nectar to chase (today's world never empties, but stay robust):
        // abandon the pursuit and let the wander/rest cycle resume.
        bee.state = BeeState::Wandering;
        return Vec3::ZERO;
    };

    if distance_squared <= FORAGE_REACH_RADIUS * FORAGE_REACH_RADIUS {
        // At the flower: feed. The refill outpaces the active drain that
        // `step_energy_and_state` still applies this tick, so net energy climbs.
        bee.energy = (bee.energy + FORAGE_REFILL_PER_SECOND * dt).clamp(0.0, 1.0);
        if bee.energy >= FORAGE_SATISFIED_ENERGY {
            bee.state = BeeState::Wandering;
        }
    }

    // Steer toward the flower; `normalized` yields ZERO when coincident, so a
    // bee sitting exactly on a source simply feeds in place.
    target
        .sub(bee.position)
        .normalized()
        .scale(FORAGE_SEEK_STRENGTH)
}

/// The position of the nectar source nearest `position`, with its squared
/// distance, or `None` when there is no nectar. Ties on distance are broken by
/// the lower slice index — and since resources are pushed in id order, that is
/// the lower entity id — so the choice is pinned and foraging stays
/// deterministic regardless of how floats happen to land.
fn nearest_nectar(position: Vec3, resources: &[Resource]) -> Option<(Vec3, f64)> {
    let mut best: Option<(Vec3, f64)> = None;
    for resource in resources {
        if resource.kind != ResourceKind::Nectar {
            continue;
        }
        let distance_squared = position.distance_squared(resource.position);
        // Strictly closer wins; an equal distance leaves the earlier (lower-id)
        // source in place, which is what pins the tie-break.
        if best.is_none_or(|(_, best_distance)| distance_squared < best_distance) {
            best = Some((resource.position, distance_squared));
        }
    }
    best
}

/// A grid keyed by integer cell coordinates, hashed with [`CellHasher`].
type CellMap = HashMap<(i64, i64, i64), Vec<usize>, BuildHasherDefault<CellHasher>>;

/// A tiny deterministic hasher for integer grid-cell keys. SipHash (the standard
/// library default) is robust against adversarial keys but, at tens of thousands
/// of cell lookups per tick, is the dominant cost of the broad phase. Cell keys
/// are trusted internal integers, so we mix the three coordinates with a
/// multiply-rotate instead — far cheaper, and with no random seed it keeps cell
/// hashing deterministic across runs.
#[derive(Default)]
struct CellHasher(u64);

impl Hasher for CellHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        // Fallback path; the cell key only ever hashes via `write_i64`.
        for &byte in bytes {
            self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn write_i64(&mut self, value: i64) {
        self.0 = (self.0.rotate_left(13) ^ value as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// Grid cell coordinate for a position, at cell size `SEPARATION_RADIUS`. Two
/// bees within that radius differ by at most one cell on each axis, so the
/// 3×3×3 neighbourhood of a cell is guaranteed to contain every in-range pair.
fn cell_of(position: Vec3) -> (i64, i64, i64) {
    (
        (position.x / SEPARATION_RADIUS).floor() as i64,
        (position.y / SEPARATION_RADIUS).floor() as i64,
        (position.z / SEPARATION_RADIUS).floor() as i64,
    )
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
    fn hungry_bee_forages_toward_nectar_and_refuels() {
        let mut world = World::empty(Bounds::new(400.0, 400.0, 400.0));
        let flower = Vec3::new(200.0, 200.0, 0.0);
        world.add_resource(flower, ResourceKind::Nectar);
        // A hungry bee a little way off, drifting nowhere in particular.
        world.spawn_bee(Vec3::new(120.0, 200.0, 0.0), Vec3::ZERO);
        world.bees[0].energy = FORAGE_ENERGY_THRESHOLD;

        // With nectar present and energy at the threshold, the first step commits
        // the bee to foraging.
        world.step(1.0 / 30.0);
        assert_eq!(world.bees[0].state, BeeState::Foraging);

        // Let it fly in and feed, tracking how close it gets and how high its
        // energy climbs (it may satisfy and wander off again before the end).
        let start_distance = world.bees[0].position.distance_squared(flower);
        let mut min_distance = start_distance;
        let mut max_energy = world.bees[0].energy;
        for _ in 0..600 {
            world.step(1.0 / 30.0);
            min_distance = min_distance.min(world.bees[0].position.distance_squared(flower));
            max_energy = max_energy.max(world.bees[0].energy);
        }

        assert!(min_distance < start_distance, "bee should move toward the nectar");
        assert!(
            max_energy > FORAGE_ENERGY_THRESHOLD,
            "bee should regain energy by foraging"
        );
    }

    #[test]
    fn nearest_nectar_breaks_ties_by_lower_id() {
        // Two nectar sources equidistant from the bee: the one added first (the
        // lower id, hence lower slice index) must win, deterministically.
        let mut world = World::empty(Bounds::new(400.0, 400.0, 400.0));
        let first = Vec3::new(100.0, 200.0, 0.0);
        let second = Vec3::new(300.0, 200.0, 0.0);
        world.add_resource(first, ResourceKind::Nectar);
        world.add_resource(second, ResourceKind::Nectar);
        let from = Vec3::new(200.0, 200.0, 0.0); // exactly between them
        let (target, _) = nearest_nectar(from, &world.resources).expect("nectar exists");
        assert_eq!(target, first);
    }

    #[test]
    fn grid_matches_naive() {
        // The spatial-grid broad phase must reproduce the all-pairs reference
        // bit-for-bit, including after the swarm has clustered for a while.
        let mut world = World::seeded_with_count(500);
        for _ in 0..50 {
            world.step(1.0 / 30.0);
        }
        assert_eq!(
            world.separation_accelerations(),
            world.separation_accelerations_all_pairs()
        );
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
