//! The bee entity and its per-tick behavior.

use serde::{Deserialize, Serialize};

use crate::entity::EntityId;
use crate::math::Vec3;
use crate::world::Bounds;

/// What a bee is currently doing.
///
/// This is a *flat superset* of every caste's states, not a per-class nesting:
/// keeping it flat preserves the `snake_case` wire values, the frontend string
/// union, and the `dot--<state>` / `bar-fill--<state>` CSS convention. The
/// caste is the gate — each [`BeeClass`]'s decision process only ever enters its
/// own subset, so an out-of-caste pair (a drone `Foraging`, say) is
/// representable but never reached (guarded by tests). `Resting` is shared by
/// every caste; the rest are caste-specific:
/// - Workers: `Wandering`, `Foraging`, `BuildingComb` (secreting wax at the hive).
/// - Queen: `LayingEggs`.
/// - Drones: `Loafing` (idling near the hive), `Flying` (an orientation flight).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeeState {
    Wandering,
    Foraging,
    Resting,
    BuildingComb,
    LayingEggs,
    Loafing,
    Flying,
}

/// Biological sex of a bee. A pure function of caste (see [`BeeClass::sex`]), so
/// it is never stored on the bee — it is derived when a snapshot is captured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sex {
    Male,
    Female,
}

/// The caste a bee belongs to. Caste fixes both the bee's sex and which
/// behavior states it may occupy, so it is the single discriminator the
/// per-class decision processes branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeeClass {
    Queen,
    Worker,
    Drone,
}

impl BeeClass {
    /// Sex follows caste: drones are the colony's males; the queen and the
    /// workers are female. This is the sole authority on a bee's sex, so the two
    /// can never drift apart.
    pub const fn sex(self) -> Sex {
        match self {
            BeeClass::Drone => Sex::Male,
            BeeClass::Queen | BeeClass::Worker => Sex::Female,
        }
    }

    /// The state a freshly spawned bee of this caste starts in — its resting
    /// "home" activity before the decision process takes over.
    const fn initial_state(self) -> BeeState {
        match self {
            BeeClass::Queen => BeeState::LayingEggs,
            BeeClass::Worker => BeeState::Wandering,
            BeeClass::Drone => BeeState::Loafing,
        }
    }
}

/// Energy an active bee (`Wandering` or `Foraging`) spends per second, as a
/// fraction of a full reserve. At this rate a full bee reaches the rest
/// threshold after 40 s of continuous activity.
const ACTIVE_ENERGY_DRAIN_PER_SECOND: f64 = 0.02;

/// Energy a `Resting` bee recovers per second. Faster than the active drain, so
/// a rest stint is shorter than the wandering stint it interrupts.
const REST_ENERGY_REFILL_PER_SECOND: f64 = 0.05;

/// At or below this energy an active bee gives up and drops to `Resting`.
const REST_ENERGY_THRESHOLD: f64 = 0.2;

/// At or above this energy a `Resting` bee is recovered enough to return to
/// `Wandering`. The gap from [`REST_ENERGY_THRESHOLD`] is hysteresis: a bee
/// must climb the whole band back before leaving rest, so one sitting near a
/// single level can't flip state every tick.
const WAKE_ENERGY_THRESHOLD: f64 = 0.8;

/// Energy the queen spends per second while `LayingEggs`. Brood tending is
/// sedentary work — gentler than the active flight drain — so her laying stints
/// run long between rests.
const QUEEN_LAY_DRAIN_PER_SECOND: f64 = 0.01;

/// Energy a drone spends per second while `Loafing`. Idling around the hive is
/// the cheapest activity in the colony, so a drone coasts a long while before it
/// needs to rest.
const DRONE_LOAF_DRAIN_PER_SECOND: f64 = 0.005;

/// Energy a worker spends per second while `BuildingComb`. Secreting wax is
/// active hive work, costing the same as wandering flight.
const WORKER_BUILD_DRAIN_PER_SECOND: f64 = ACTIVE_ENERGY_DRAIN_PER_SECOND;

/// A single bee in the colony.
#[derive(Debug, Clone, PartialEq)]
pub struct Bee {
    pub id: EntityId,
    pub position: Vec3,
    pub velocity: Vec3,
    /// The caste this bee belongs to — fixed for its lifetime. Gates which
    /// states it may enter and which decision process drives it.
    pub class: BeeClass,
    pub state: BeeState,
    /// Remaining energy as a fraction in `[0, 1]`; bees spawn full at `1.0` and
    /// drain over time (see [`Bee::step`]). Clamped at empty each tick — refill
    /// (resting, nectar) arrives with the foraging/behavior slices.
    pub energy: f64,
    /// Wax scales the bee has secreted, as a running count. Only workers (while
    /// `BuildingComb`) ever accrue any; every other caste holds at `0.0`. A
    /// thousand scales make a single gram of comb wax — the colony total is
    /// summed from these (see [`crate::world::World::wax_grams`]).
    pub wax_scales: f64,
}

impl Bee {
    /// Spawn a bee of `class` at full energy, in that caste's initial state and
    /// with no wax yet secreted.
    pub fn new(id: EntityId, position: Vec3, velocity: Vec3, class: BeeClass) -> Self {
        Self {
            id,
            position,
            velocity,
            class,
            state: class.initial_state(),
            energy: 1.0,
            wax_scales: 0.0,
        }
    }

    /// Advance the bee by one fixed timestep of `dt` seconds.
    ///
    /// Integrates position from velocity and bounces off the world walls so
    /// the bee always remains inside `bounds`: on hitting an edge the position
    /// is clamped to the wall and the offending velocity component is inverted.
    /// All three axes are confined symmetrically — the z (flight) axis is live
    /// even though bees currently start flat at `z = 0` with no vertical
    /// velocity, so the dimension is exercised the moment flight is introduced.
    ///
    /// Energy and behavior advance here too, via [`Bee::step_energy_and_state`].
    /// That stays a pure single-entity update — it reads and writes only this
    /// bee — so the engine's deterministic ordering is unaffected.
    pub fn step(&mut self, dt: f64, bounds: Bounds) {
        self.step_energy_and_state(dt);

        let mut next = self.position.add(self.velocity.scale(dt));

        if next.x < 0.0 {
            next.x = 0.0;
            self.velocity.x = -self.velocity.x;
        } else if next.x > bounds.width {
            next.x = bounds.width;
            self.velocity.x = -self.velocity.x;
        }

        if next.y < 0.0 {
            next.y = 0.0;
            self.velocity.y = -self.velocity.y;
        } else if next.y > bounds.height {
            next.y = bounds.height;
            self.velocity.y = -self.velocity.y;
        }

        if next.z < 0.0 {
            next.z = 0.0;
            self.velocity.z = -self.velocity.z;
        } else if next.z > bounds.depth {
            next.z = bounds.depth;
            self.velocity.z = -self.velocity.z;
        }

        self.position = next;
    }

    /// Advance energy and behavior state by one `dt` tick.
    ///
    /// Active states drain energy; `Resting` recovers it at
    /// [`REST_ENERGY_REFILL_PER_SECOND`]. The active drain rate depends on the
    /// caste's current activity (sedentary laying and loafing cost less than
    /// flight). Energy is clamped to `[0, 1]`, then the state follows it with
    /// hysteresis: an active bee drops to `Resting` only once spent
    /// (≤ [`REST_ENERGY_THRESHOLD`]) and stays resting until fully recovered
    /// (≥ [`WAKE_ENERGY_THRESHOLD`]), waking back into its caste's home activity.
    /// Each caste only ever toggles between its own states here.
    ///
    /// Entry into the *purposeful* states — `Foraging`/`BuildingComb` for a
    /// worker, `Flying` for a drone — belongs to the world-level decision
    /// processes (see [`crate::world`]), mirroring how foraging entry has always
    /// lived there. This method reads those states only as further active
    /// (draining) states and can drop them to `Resting` when energy runs out.
    ///
    /// Pure single-entity: only this bee is read and written, so it leaves the
    /// engine's deterministic stepping order untouched.
    fn step_energy_and_state(&mut self, dt: f64) {
        let rate = match (self.class, self.state) {
            (_, BeeState::Resting) => REST_ENERGY_REFILL_PER_SECOND,
            (BeeClass::Queen, BeeState::LayingEggs) => -QUEEN_LAY_DRAIN_PER_SECOND,
            (BeeClass::Drone, BeeState::Loafing) => -DRONE_LOAF_DRAIN_PER_SECOND,
            (BeeClass::Worker, BeeState::BuildingComb) => -WORKER_BUILD_DRAIN_PER_SECOND,
            // Wandering / Foraging workers and Flying drones — full flight cost.
            _ => -ACTIVE_ENERGY_DRAIN_PER_SECOND,
        };
        self.energy = (self.energy + rate * dt).clamp(0.0, 1.0);

        // Rest/wake hysteresis, scoped to each caste's own state set: a worn-out
        // bee drops to `Resting`; a recovered one wakes into its home activity.
        let spent = self.energy <= REST_ENERGY_THRESHOLD;
        let recovered = self.energy >= WAKE_ENERGY_THRESHOLD;
        match self.class {
            BeeClass::Worker => match self.state {
                BeeState::Resting if recovered => self.state = BeeState::Wandering,
                BeeState::Wandering | BeeState::Foraging | BeeState::BuildingComb if spent => {
                    self.state = BeeState::Resting;
                }
                _ => {}
            },
            BeeClass::Queen => match self.state {
                BeeState::Resting if recovered => self.state = BeeState::LayingEggs,
                BeeState::LayingEggs if spent => self.state = BeeState::Resting,
                _ => {}
            },
            BeeClass::Drone => match self.state {
                BeeState::Resting if recovered => self.state = BeeState::Loafing,
                BeeState::Loafing | BeeState::Flying if spent => self.state = BeeState::Resting,
                _ => {}
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_moves_along_velocity() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(
            EntityId(0),
            Vec3::new(10.0, 10.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
            BeeClass::Worker,
        );
        bee.step(1.0, bounds);
        assert_eq!(bee.position, Vec3::new(15.0, 10.0, 0.0));
    }

    #[test]
    fn bee_stays_within_bounds_and_bounces() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        // Aimed past the right wall.
        let mut bee = Bee::new(
            EntityId(0),
            Vec3::new(98.0, 50.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            BeeClass::Worker,
        );
        bee.step(1.0, bounds);
        assert!(bee.position.x <= bounds.width);
        assert!(bee.position.x >= 0.0);
        // Velocity reflected, so it now heads away from the wall.
        assert!(bee.velocity.x < 0.0);
    }

    #[test]
    fn bee_spawns_with_full_energy() {
        let bee = Bee::new(EntityId(0), Vec3::ZERO, Vec3::ZERO, BeeClass::Worker);
        assert_eq!(bee.energy, 1.0);
        assert_eq!(bee.state, BeeState::Wandering);
        // No caste has secreted wax at spawn — workers earn it only by building.
        assert_eq!(bee.wax_scales, 0.0);
    }

    #[test]
    fn sex_follows_caste() {
        assert_eq!(BeeClass::Drone.sex(), Sex::Male);
        assert_eq!(BeeClass::Worker.sex(), Sex::Female);
        assert_eq!(BeeClass::Queen.sex(), Sex::Female);
    }

    #[test]
    fn each_caste_spawns_in_its_home_state() {
        let at = |class| Bee::new(EntityId(0), Vec3::ZERO, Vec3::ZERO, class).state;
        assert_eq!(at(BeeClass::Queen), BeeState::LayingEggs);
        assert_eq!(at(BeeClass::Worker), BeeState::Wandering);
        assert_eq!(at(BeeClass::Drone), BeeState::Loafing);
    }

    #[test]
    fn stepping_drains_energy_while_active() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        bee.step(1.0, bounds);
        // A full second of the active drain rate; still wandering, not yet spent.
        assert_eq!(bee.energy, 1.0 - ACTIVE_ENERGY_DRAIN_PER_SECOND);
        assert_eq!(bee.state, BeeState::Wandering);
    }

    #[test]
    fn energy_clamps_at_empty() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        // One huge active step overshoots empty; the clamp floors it at 0 before
        // the same tick flips the spent bee to Resting (refill only starts next
        // tick, once it is already resting).
        bee.step(1_000.0, bounds);
        assert_eq!(bee.energy, 0.0);
        assert_eq!(bee.state, BeeState::Resting);
    }

    #[test]
    fn active_bee_drops_to_rest_then_recovers() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        let dt = 1.0 / 30.0;

        // Drain long enough to fall through the rest threshold.
        let mut went_to_rest = false;
        for _ in 0..2_000 {
            bee.step(dt, bounds);
            if bee.state == BeeState::Resting {
                went_to_rest = true;
                break;
            }
        }
        assert!(went_to_rest, "an idle active bee should eventually rest");
        assert!(bee.energy <= REST_ENERGY_THRESHOLD);

        // Keep resting and it should recover and wake back up.
        let mut woke = false;
        for _ in 0..2_000 {
            bee.step(dt, bounds);
            if bee.state == BeeState::Wandering {
                woke = true;
                break;
            }
        }
        assert!(woke, "a resting bee should recover and wander again");
        assert!(bee.energy >= WAKE_ENERGY_THRESHOLD);
    }

    #[test]
    fn rest_wake_thresholds_have_hysteresis() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let dt = 1.0 / 30.0;

        // Midway through the band, a resting bee keeps resting (it has not
        // reached the wake threshold) rather than flipping back immediately.
        let mut resting =
            Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        resting.state = BeeState::Resting;
        resting.energy = 0.5;
        resting.step(dt, bounds);
        assert_eq!(resting.state, BeeState::Resting);

        // Symmetrically, an active bee at the same mid-band energy keeps
        // wandering: it is above the rest threshold.
        let mut active =
            Bee::new(EntityId(1), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO, BeeClass::Worker);
        active.energy = 0.5;
        active.step(dt, bounds);
        assert_eq!(active.state, BeeState::Wandering);
    }

    #[test]
    fn bee_bounces_off_floor_and_ceiling() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        // Aimed up past the ceiling on the z (flight) axis.
        let mut bee = Bee::new(
            EntityId(0),
            Vec3::new(50.0, 50.0, 98.0),
            Vec3::new(0.0, 0.0, 10.0),
            BeeClass::Worker,
        );
        bee.step(1.0, bounds);
        assert!(bee.position.z <= bounds.depth);
        assert!(bee.position.z >= 0.0);
        // Vertical velocity reflected back down.
        assert!(bee.velocity.z < 0.0);
    }
}
