//! Serializable snapshot types — the wire format sent to clients.
//!
//! Kept separate from the live simulation structs so internal fields can change
//! without breaking the client protocol. A snapshot is a flat, render-ready
//! view of the world at a single tick.

use serde::{Deserialize, Serialize};

use crate::bee::{Bee, BeeState};
use crate::math::Vec3;
use crate::world::{Bounds, Resource, ResourceKind, World};

/// A complete, immutable view of the world at one tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub bounds: Bounds,
    pub bees: Vec<BeeSnapshot>,
    pub resources: Vec<ResourceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeeSnapshot {
    pub id: u64,
    pub position: Vec3,
    pub velocity: Vec3,
    pub state: BeeState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub id: u64,
    pub position: Vec3,
    pub kind: ResourceKind,
}

impl WorldSnapshot {
    /// Build a snapshot of `world` at the given `tick`.
    pub fn capture(world: &World, tick: u64) -> Self {
        Self {
            tick,
            bounds: world.bounds,
            bees: world.bees.iter().map(BeeSnapshot::from_bee).collect(),
            resources: world
                .resources
                .iter()
                .map(ResourceSnapshot::from_resource)
                .collect(),
        }
    }
}

impl BeeSnapshot {
    fn from_bee(bee: &Bee) -> Self {
        Self {
            id: bee.id.value(),
            position: bee.position,
            velocity: bee.velocity,
            state: bee.state,
        }
    }
}

impl ResourceSnapshot {
    fn from_resource(resource: &Resource) -> Self {
        Self {
            id: resource.id.value(),
            position: resource.position,
            kind: resource.kind,
        }
    }
}
