//! The bee entity and its per-tick behavior.

use serde::{Deserialize, Serialize};

use crate::entity::EntityId;
use crate::math::Vec3;
use crate::world::Bounds;

/// What a bee is currently doing.
///
/// Intentionally minimal for this slice — additional states (`Foraging`,
/// `Returning`, `Resting`, …) slot in here as behavior grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeeState {
    Wandering,
}

/// A single bee in the colony.
#[derive(Debug, Clone, PartialEq)]
pub struct Bee {
    pub id: EntityId,
    pub position: Vec3,
    pub velocity: Vec3,
    pub state: BeeState,
}

impl Bee {
    pub fn new(id: EntityId, position: Vec3, velocity: Vec3) -> Self {
        Self {
            id,
            position,
            velocity,
            state: BeeState::Wandering,
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
    pub fn step(&mut self, dt: f64, bounds: Bounds) {
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
