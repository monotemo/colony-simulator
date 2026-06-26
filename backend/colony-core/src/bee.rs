//! The bee entity and its per-tick behavior.

use serde::{Deserialize, Serialize};

use crate::entity::EntityId;
use crate::math::Vec3;
use crate::world::Bounds;

/// What a bee is currently doing.
///
/// `Foraging` (heading to or harvesting nectar) and `Resting` (recovering
/// energy) join the original `Wandering`. The variants exist so snapshots can
/// carry them and the renderer can tint by state; the logic that *transitions*
/// a bee between them lands with the behavior state machine, so for now every
/// bee stays `Wandering`. Further states (`Returning`, …) slot in the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeeState {
    Wandering,
    Foraging,
    Resting,
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

/// A single bee in the colony.
#[derive(Debug, Clone, PartialEq)]
pub struct Bee {
    pub id: EntityId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub state: BeeState,
    /// Remaining energy as a fraction in `[0, 1]`; bees spawn full at `1.0` and
    /// drain over time (see [`Bee::step`]). Clamped at empty each tick — refill
    /// (resting, nectar) arrives with the foraging/behavior slices.
    pub energy: f64,
}

impl Bee {
    pub fn new(id: EntityId, position: Vec3, velocity: Vec3) -> Self {
        Self {
            id,
            position,
            velocity,
            state: BeeState::Wandering,
            energy: 1.0,
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
    /// Active states spend energy at [`ACTIVE_ENERGY_DRAIN_PER_SECOND`];
    /// `Resting` recovers it at [`REST_ENERGY_REFILL_PER_SECOND`]. Energy is
    /// clamped to `[0, 1]`, then the state follows it with hysteresis: an active
    /// bee drops to `Resting` only once spent (≤ [`REST_ENERGY_THRESHOLD`]), and
    /// stays resting until fully recovered (≥ [`WAKE_ENERGY_THRESHOLD`]), so a
    /// bee hovering near one level can't toggle every tick. Entry into
    /// `Foraging` belongs to the foraging system, not here — this method never
    /// puts a bee into `Foraging`, only reads it as another active (draining)
    /// state and can move it to `Resting` when it runs low.
    ///
    /// Pure single-entity: only this bee is read and written, so it leaves the
    /// engine's deterministic stepping order untouched.
    fn step_energy_and_state(&mut self, dt: f64) {
        let rate = match self.state {
            BeeState::Resting => REST_ENERGY_REFILL_PER_SECOND,
            BeeState::Wandering | BeeState::Foraging => -ACTIVE_ENERGY_DRAIN_PER_SECOND,
        };
        self.energy = (self.energy + rate * dt).clamp(0.0, 1.0);

        match self.state {
            BeeState::Resting if self.energy >= WAKE_ENERGY_THRESHOLD => {
                self.state = BeeState::Wandering;
            }
            BeeState::Wandering | BeeState::Foraging if self.energy <= REST_ENERGY_THRESHOLD => {
                self.state = BeeState::Resting;
            }
            _ => {}
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
        );
        bee.step(1.0, bounds);
        assert!(bee.position.x <= bounds.width);
        assert!(bee.position.x >= 0.0);
        // Velocity reflected, so it now heads away from the wall.
        assert!(bee.velocity.x < 0.0);
    }

    #[test]
    fn bee_spawns_with_full_energy() {
        let bee = Bee::new(EntityId(0), Vec3::ZERO, Vec3::ZERO);
        assert_eq!(bee.energy, 1.0);
        assert_eq!(bee.state, BeeState::Wandering);
    }

    #[test]
    fn stepping_drains_energy_while_active() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        bee.step(1.0, bounds);
        // A full second of the active drain rate; still wandering, not yet spent.
        assert_eq!(bee.energy, 1.0 - ACTIVE_ENERGY_DRAIN_PER_SECOND);
        assert_eq!(bee.state, BeeState::Wandering);
    }

    #[test]
    fn energy_clamps_at_empty() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
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
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
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
        let mut resting = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        resting.state = BeeState::Resting;
        resting.energy = 0.5;
        resting.step(dt, bounds);
        assert_eq!(resting.state, BeeState::Resting);

        // Symmetrically, an active bee at the same mid-band energy keeps
        // wandering: it is above the rest threshold.
        let mut active = Bee::new(EntityId(1), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
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
        );
        bee.step(1.0, bounds);
        assert!(bee.position.z <= bounds.depth);
        assert!(bee.position.z >= 0.0);
        // Vertical velocity reflected back down.
        assert!(bee.velocity.z < 0.0);
    }
}
