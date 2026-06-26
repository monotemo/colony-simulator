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

/// Energy a bee spends per second, as a fraction of a full reserve. A flat
/// drain for this slice — the behavior state machine will scale it by what the
/// bee is doing (active states drain faster, resting refills). At this rate a
/// bee runs from full to empty in 50 s of continuous activity.
const ENERGY_DRAIN_PER_SECOND: f64 = 0.02;

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
    /// Energy is spent here too: a flat drain per tick, clamped at empty so it
    /// never goes negative. This stays a pure single-entity update — the drain
    /// depends only on `dt`, not on neighbours — so determinism is unaffected.
    pub fn step(&mut self, dt: f64, bounds: Bounds) {
        self.energy = (self.energy - ENERGY_DRAIN_PER_SECOND * dt).max(0.0);

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
    fn stepping_drains_energy() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        bee.step(1.0, bounds);
        // A full second of the flat drain rate, and never above the start value.
        assert_eq!(bee.energy, 1.0 - ENERGY_DRAIN_PER_SECOND);
        assert!(bee.energy < 1.0);
    }

    #[test]
    fn energy_clamps_at_empty() {
        let bounds = Bounds::new(100.0, 100.0, 100.0);
        let mut bee = Bee::new(EntityId(0), Vec3::new(50.0, 50.0, 0.0), Vec3::ZERO);
        // Far more time than it takes to exhaust the reserve; it must floor at 0.
        bee.step(1_000.0, bounds);
        assert_eq!(bee.energy, 0.0);
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
