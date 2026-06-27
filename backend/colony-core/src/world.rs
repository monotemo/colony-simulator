//! The world: physical bounds, resources, and the entities living inside it.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

use serde::{Deserialize, Serialize};

use crate::bee::{Bee, BeeClass, BeeState};
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

/// Honey a single bee adds to the colony store per second while feeding at a
/// nectar source, as a fraction of the store's capacity. The store is tracked
/// normalized to `[0, 1]` (see [`World::honey_stored`]); a handful of bees
/// feeding fills it over tens of seconds.
const HONEY_HARVEST_PER_BEE_PER_SECOND: f64 = 0.02;

/// Honey the colony consumes per second, as a fraction of capacity. A slow,
/// constant draw so the store ebbs when little foraging is happening rather than
/// only ever climbing — the level settles where harvest and consumption balance.
const HONEY_CONSUMPTION_PER_SECOND: f64 = 0.01;

/// Wax scales a building worker secretes per second. A worker makes roughly
/// eight scales over a twelve-hour shift, so the rate is `8 / (12 · 3600)`
/// scales/second — tiny per tick, but it adds up over a long build stint.
const WAX_SCALES_PER_SECOND: f64 = 8.0 / (12.0 * 3600.0);

/// Wax scales that make up a single gram of comb wax. The colony's wax total is
/// tracked in grams, converted from the scales its workers secrete.
const SCALES_PER_GRAM: f64 = 1000.0;

/// Energy at or above which a wandering worker, with nothing more urgent to do,
/// settles at the hive to secrete wax and build comb. Set above
/// [`FORAGE_ENERGY_THRESHOLD`] so a hungry worker forages rather than builds.
const BUILD_ENERGY_THRESHOLD: f64 = 0.7;

/// Energy at which a building worker has spent enough and drifts back to
/// wandering. Comfortably above the rest threshold, so an ordinary build stint
/// ends by choice rather than by exhaustion.
const BUILD_STOP_ENERGY: f64 = 0.5;

/// Energy at or above which a loafing drone launches into an orientation flight.
const DRONE_FLY_ENERGY: f64 = 0.8;

/// Energy at which a flying drone gives up the flight and settles back to loaf.
const DRONE_LAND_ENERGY: f64 = 0.4;

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
    /// Honey in the colony store, as a fraction in `[0, 1]`. Rises as bees
    /// harvest nectar while feeding (see [`forage`]) and ebbs from a slow,
    /// constant consumption, clamped to the range each tick. Surfaced on the
    /// snapshot as `honeyStored`.
    pub honey_stored: f64,
    /// Total comb wax the colony has produced, in grams. Accumulated from the
    /// scales its workers secrete while `BuildingComb` (1000 scales = 1 gram)
    /// and summed in bee-index order each tick so the total is reproducible.
    /// Monotonic — wax is built, never spent here. Surfaced as `waxGrams`.
    pub wax_grams: f64,
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
            honey_stored: 0.0,
            wax_grams: 0.0,
            ids: IdAllocator::new(),
            grid: SpatialGrid::default(),
        }
    }

    /// Spawn a bee of `class` at `position` with `velocity`, allocating it a
    /// fresh id.
    pub fn spawn_bee(&mut self, position: Vec3, velocity: Vec3, class: BeeClass) -> EntityId {
        let id = self.ids.alloc();
        self.bees.push(Bee::new(id, position, velocity, class));
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

    /// Build the default seeded starting world: 500 bees with deterministic,
    /// varied velocities plus a few nectar sources. Deterministic so the
    /// initial state is reproducible without pulling in an RNG dependency. The
    /// colony is cast as exactly one queen, a few drones, and workers for the
    /// rest (see [`class_for`]) — at 500 that is 1 queen, 83 drones, 416 workers.
    pub fn seeded() -> Self {
        Self::seeded_with_count(500)
    }

    /// Build a seeded world with `bee_count` bees. The placement and velocity
    /// math is parameterized on `t = i / bee_count`, so the layout shape is
    /// the same at any population — at `bee_count == 500` it is byte-identical
    /// to [`World::seeded`]. Exists so benchmarks and scale tests can stress
    /// arbitrary populations without an RNG; production still uses `seeded()`.
    pub fn seeded_with_count(bee_count: usize) -> Self {
        // A roomy box for a large colony: a 4000×4000 floor gives the swarm
        // space to spread without packing into a wall, and the depth is sized
        // for the eventual flight volume. Bees and resources are seeded flat at
        // z = 0 until flight behavior lands, so the third axis exists for real
        // in the geometry while visuals stay top-down for now.
        let bounds = Bounds::new(4000.0, 4000.0, 1000.0);
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
            world.spawn_bee(position, velocity, class_for(i));

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
        // Honey harvested and wax scales secreted across the colony this tick,
        // each summed in bee-index order into a local before it touches a store,
        // so both totals are reproducible regardless of how floats land.
        let mut harvested = 0.0;
        let mut wax_produced = 0.0;
        for (bee, &separation) in self.bees.iter_mut().zip(accelerations) {
            // Per-caste decision process, dispatched on the bee's class. Each
            // arm reads only this bee and the immutable resources — never other
            // bees — so it can't perturb the separation pass's ordering and
            // determinism is preserved. It yields a steering seek to fold in
            // alongside separation, plus any honey/wax it produced this tick.
            let (seek, honey, scales) = match bee.class {
                BeeClass::Worker => {
                    // A hungry worker forages; an idle, well-fed one builds comb.
                    // Foraging is checked first so food always wins over wax.
                    let (seek, honey) = forage(bee, &self.resources, dt);
                    let scales = build_comb(bee, dt);
                    (seek, honey, scales)
                }
                BeeClass::Drone => (drone_behavior(bee), 0.0, 0.0),
                // The queen tends the brood at the hive; her laying/rest cycle is
                // driven entirely by the energy machine in `Bee::step`, and she
                // neither forages nor builds, so she contributes no steering.
                BeeClass::Queen => (Vec3::ZERO, 0.0, 0.0),
            };
            harvested += honey;
            wax_produced += scales;

            let mut velocity = bee.velocity.add(separation.add(seek).scale(dt));
            let speed_squared = velocity.length_squared();
            if speed_squared > MAX_SPEED * MAX_SPEED {
                velocity = velocity.normalized().scale(MAX_SPEED);
            }
            bee.velocity = velocity;
            bee.step(dt, self.bounds);
        }

        // Fold this tick's harvest into the store and let the colony's slow
        // consumption draw it down, clamped to [0, 1].
        self.honey_stored =
            (self.honey_stored + harvested - HONEY_CONSUMPTION_PER_SECOND * dt).clamp(0.0, 1.0);
        // Bank this tick's wax. The store is monotonic — wax is built, not
        // consumed — so it simply accumulates in grams.
        self.wax_grams += wax_produced / SCALES_PER_GRAM;
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
/// acceleration to fold into this bee's steering, paired with the honey it
/// harvests this tick (both zero when it isn't feeding). A wandering bee that
/// drops to [`FORAGE_ENERGY_THRESHOLD`] commits to foraging if there is nectar
/// to chase; a foraging bee steers toward the nearest source and, once within
/// [`FORAGE_REACH_RADIUS`], feeds — refuelling itself and harvesting honey for
/// the colony — until satisfied, then returns to wandering, keeping its momentum
/// so it drifts back out naturally.
///
/// Reads only `bee` and the immutable `resources`, never other bees, so it is
/// order-independent and leaves the engine's determinism intact. Nectar is an
/// inexhaustible well for this slice — feeding depletes neither the source nor
/// disturbs other bees. The active energy drain still lives in
/// [`Bee::step_energy_and_state`]; the refill here is set to outpace it, so net
/// energy rises while feeding.
fn forage(bee: &mut Bee, resources: &[Resource], dt: f64) -> (Vec3, f64) {
    // A hungry wanderer commits to foraging, but only if there's nectar to find.
    if bee.state == BeeState::Wandering
        && bee.energy <= FORAGE_ENERGY_THRESHOLD
        && nearest_nectar(bee.position, resources).is_some()
    {
        bee.state = BeeState::Foraging;
    }

    if bee.state != BeeState::Foraging {
        return (Vec3::ZERO, 0.0);
    }

    let Some((target, distance_squared)) = nearest_nectar(bee.position, resources) else {
        // No nectar to chase (today's world never empties, but stay robust):
        // abandon the pursuit and let the wander/rest cycle resume.
        bee.state = BeeState::Wandering;
        return (Vec3::ZERO, 0.0);
    };

    let mut harvested = 0.0;
    if distance_squared <= FORAGE_REACH_RADIUS * FORAGE_REACH_RADIUS {
        // At the flower: feed. The refill outpaces the active drain that
        // `step_energy_and_state` still applies this tick, so net energy climbs,
        // and the colony banks honey for as long as the bee keeps feeding.
        bee.energy = (bee.energy + FORAGE_REFILL_PER_SECOND * dt).clamp(0.0, 1.0);
        harvested = HONEY_HARVEST_PER_BEE_PER_SECOND * dt;
        if bee.energy >= FORAGE_SATISFIED_ENERGY {
            bee.state = BeeState::Wandering;
        }
    }

    // Steer toward the flower; `normalized` yields ZERO when coincident, so a
    // bee sitting exactly on a source simply feeds in place.
    let seek = target
        .sub(bee.position)
        .normalized()
        .scale(FORAGE_SEEK_STRENGTH);
    (seek, harvested)
}

/// Wax-building behavior for a single worker, run each tick after [`forage`].
///
/// Drives the entry and exit of [`BeeState::BuildingComb`] and returns the wax
/// scales the worker secretes this tick (zero unless it is building). A
/// well-fed worker that isn't foraging settles at the hive to build comb; it
/// keeps secreting until it has spent enough energy ([`BUILD_STOP_ENERGY`]) and
/// drifts back to wandering, or until the energy machine drops it to `Resting`.
///
/// Reads and writes only `bee`, never other bees or the world, so it is
/// order-independent and leaves the engine's determinism intact. Wax production
/// is gated on the caste here too: a non-worker never enters the build state, so
/// only workers ever return a non-zero amount.
fn build_comb(bee: &mut Bee, dt: f64) -> f64 {
    if bee.class != BeeClass::Worker {
        return 0.0;
    }

    // A well-fed wanderer with no foraging to do commits to building comb.
    if bee.state == BeeState::Wandering && bee.energy >= BUILD_ENERGY_THRESHOLD {
        bee.state = BeeState::BuildingComb;
    }

    if bee.state != BeeState::BuildingComb {
        return 0.0;
    }

    // Spent enough on this stint: drift back out to wander (the build drain in
    // `step_energy_and_state` is what brought it down to here).
    if bee.energy <= BUILD_STOP_ENERGY {
        bee.state = BeeState::Wandering;
        return 0.0;
    }

    let scales = WAX_SCALES_PER_SECOND * dt;
    bee.wax_scales += scales;
    scales
}

/// Loafing/flight behavior for a single drone, run each tick before integration.
///
/// Drones neither forage nor build; they idle near the hive and take the
/// occasional orientation flight. A rested drone launches into [`BeeState::Flying`]
/// once well-fed ([`DRONE_FLY_ENERGY`]) and settles back to [`BeeState::Loafing`]
/// when the flight has worn it down ([`DRONE_LAND_ENERGY`]); the energy machine
/// drops either state to `Resting` if it bottoms out. Returns no steering force —
/// a drone coasts on its existing velocity and the separation push — and reads
/// only `bee`, so it stays order-independent and deterministic.
fn drone_behavior(bee: &mut Bee) -> Vec3 {
    match bee.state {
        BeeState::Loafing if bee.energy >= DRONE_FLY_ENERGY => bee.state = BeeState::Flying,
        BeeState::Flying if bee.energy <= DRONE_LAND_ENERGY => bee.state = BeeState::Loafing,
        _ => {}
    }
    Vec3::ZERO
}

/// The caste of the `i`-th seeded bee. Deterministic and index-based (no RNG):
/// bee 0 is the colony's sole queen, every sixth bee after that is a drone, and
/// the rest are workers — a queen-led colony that is mostly workers with a few
/// drones, matching real hive composition.
fn class_for(i: usize) -> BeeClass {
    if i == 0 {
        BeeClass::Queen
    } else if i.is_multiple_of(6) {
        BeeClass::Drone
    } else {
        BeeClass::Worker
    }
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
        let a = world.spawn_bee(Vec3::ZERO, Vec3::ZERO, BeeClass::Worker);
        let b = world.spawn_bee(Vec3::ZERO, Vec3::ZERO, BeeClass::Worker);
        let r = world.add_resource(Vec3::ZERO, ResourceKind::Nectar);
        assert_ne!(a, b);
        assert_ne!(b, r);
    }

    #[test]
    fn seeded_matches_seeded_with_count_500() {
        // `seeded()` must stay byte-identical to the parameterized builder at
        // its production population, so adding the benchmarking hook can't
        // silently perturb the default world.
        let default = World::seeded();
        let explicit = World::seeded_with_count(500);
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
        world.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        assert_eq!(world.separation_accelerations(), vec![Vec3::ZERO]);
    }

    #[test]
    fn overlapping_bees_steer_apart() {
        // Two stationary bees well inside the separation radius should be
        // pushed further apart after a step.
        let mut world = World::empty(Bounds::new(100.0, 100.0, 100.0));
        world.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        world.spawn_bee(Vec3::new(54.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        let before = world.bees[0].position.distance_squared(world.bees[1].position);
        world.step(1.0 / 30.0);
        let after = world.bees[0].position.distance_squared(world.bees[1].position);
        assert!(after > before, "bees should separate: {before} -> {after}");
    }

    #[test]
    fn separation_is_equal_opposite_and_order_independent() {
        // A close pair pushes with equal and opposite force...
        let mut world = World::empty(Bounds::new(100.0, 100.0, 100.0));
        world.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        world.spawn_bee(Vec3::new(55.0, 52.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        let accel = world.separation_accelerations();
        assert_eq!(accel[0], accel[1].scale(-1.0));
        assert_ne!(accel[0], Vec3::ZERO);

        // ...and swapping storage order just swaps the results: the force a bee
        // feels doesn't depend on where it sits in the Vec.
        let mut swapped = World::empty(Bounds::new(100.0, 100.0, 100.0));
        swapped.spawn_bee(Vec3::new(55.0, 52.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        swapped.spawn_bee(Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
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
        world.spawn_bee(Vec3::new(120.0, 200.0, 0.0), Vec3::ZERO, BeeClass::Worker);
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
    fn foraging_banks_honey_within_bounds() {
        let mut world = World::seeded();
        assert_eq!(world.honey_stored, 0.0);

        let mut max_honey = 0.0_f64;
        for _ in 0..1_200 {
            world.step(1.0 / 30.0);
            assert!(
                (0.0..=1.0).contains(&world.honey_stored),
                "honey must stay in [0, 1], was {}",
                world.honey_stored
            );
            max_honey = max_honey.max(world.honey_stored);
        }
        assert!(max_honey > 0.0, "bees foraging should bank honey for the colony");
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

    #[test]
    fn seeded_colony_is_one_queen_a_few_drones_and_workers() {
        // The default colony must cast exactly one queen, a few drones, and
        // workers for the remainder — at 500 that is 1 / 83 / 416 (bee 0 is the
        // queen, every sixth bee after is a drone, the rest are workers).
        let world = World::seeded();
        let count = |class| world.bees.iter().filter(|b| b.class == class).count();
        assert_eq!(count(BeeClass::Queen), 1);
        assert_eq!(count(BeeClass::Drone), 83);
        assert_eq!(count(BeeClass::Worker), 416);
        assert_eq!(world.bees.len(), 500);
    }

    #[test]
    fn building_worker_secretes_wax_at_the_expected_rate() {
        // A worker held in the build state accrues scales at the per-second rate.
        let dt = 1.0 / 30.0;
        let mut bee = Bee::new(EntityId(0), Vec3::ZERO, Vec3::ZERO, BeeClass::Worker);
        bee.state = BeeState::BuildingComb;
        bee.energy = 1.0; // well above BUILD_STOP_ENERGY, so it keeps building.

        let steps = 300;
        let mut produced = 0.0;
        for _ in 0..steps {
            produced += build_comb(&mut bee, dt);
        }
        let expected = WAX_SCALES_PER_SECOND * dt * steps as f64;
        assert!((bee.wax_scales - expected).abs() < 1e-12);
        assert!((produced - expected).abs() < 1e-12);
    }

    #[test]
    fn only_workers_ever_secrete_wax() {
        let dt = 1.0 / 30.0;
        for class in [BeeClass::Queen, BeeClass::Drone] {
            let mut bee = Bee::new(EntityId(0), Vec3::ZERO, Vec3::ZERO, class);
            // Even if forced into the build state, a non-worker produces nothing.
            bee.state = BeeState::BuildingComb;
            bee.energy = 1.0;
            assert_eq!(build_comb(&mut bee, dt), 0.0);
            assert_eq!(bee.wax_scales, 0.0);
        }
    }

    #[test]
    fn colony_banks_wax_grams_from_worker_scales() {
        // Over a run the workers build comb, so the colony's gram total climbs,
        // and it matches the sum of every bee's scales divided by 1000.
        let mut world = World::seeded();
        assert_eq!(world.wax_grams, 0.0);
        for _ in 0..3_000 {
            world.step(1.0 / 30.0);
        }
        assert!(world.wax_grams > 0.0, "workers building comb should bank wax");

        let total_scales: f64 = world.bees.iter().map(|b| b.wax_scales).sum();
        assert!((world.wax_grams - total_scales / SCALES_PER_GRAM).abs() < 1e-9);
    }

    #[test]
    fn each_caste_stays_within_its_own_states() {
        // The flat state enum lets an out-of-caste pair be represented, but the
        // per-caste decision processes must never produce one over a long run.
        let mut world = World::seeded();
        for _ in 0..5_000 {
            world.step(1.0 / 30.0);
            for bee in &world.bees {
                let allowed = match bee.class {
                    BeeClass::Worker => matches!(
                        bee.state,
                        BeeState::Wandering
                            | BeeState::Foraging
                            | BeeState::Resting
                            | BeeState::BuildingComb
                    ),
                    BeeClass::Queen => {
                        matches!(bee.state, BeeState::LayingEggs | BeeState::Resting)
                    }
                    BeeClass::Drone => matches!(
                        bee.state,
                        BeeState::Loafing | BeeState::Flying | BeeState::Resting
                    ),
                };
                assert!(allowed, "{:?} reached invalid state {:?}", bee.class, bee.state);
            }
        }
    }
}
