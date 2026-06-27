//! Serializable snapshot types — the wire format sent to clients.
//!
//! Kept separate from the live simulation structs so internal fields can change
//! without breaking the client protocol. A snapshot is a flat, render-ready
//! view of the world at a single tick.

use serde::{Deserialize, Serialize};

use crate::bee::{Bee, BeeClass, BeeState, Sex};
use crate::math::Vec3;
use crate::world::{Bounds, Resource, ResourceKind, World};

/// A complete, immutable view of the world at one tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub bounds: Bounds,
    pub bees: Vec<BeeSnapshot>,
    pub resources: Vec<ResourceSnapshot>,
    /// Honey in the colony store as a fraction in `[0, 1]`. Renamed on the wire
    /// to match the `honeyStored` field the frontend already reads (multi-word
    /// fields are camelCase on the wire; single-word ones stay as-is).
    #[serde(rename = "honeyStored")]
    pub honey_stored: f64,
    /// Total comb wax the colony has produced, in grams. Renamed to `waxGrams`
    /// on the wire, matching the `honeyStored` convention.
    #[serde(rename = "waxGrams")]
    pub wax_grams: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeeSnapshot {
    pub id: u64,
    pub position: Vec3,
    pub velocity: Vec3,
    /// The bee's caste. `class` is a reserved word in JS, so it is renamed to
    /// `beeClass` on the wire (multi-word camelCase convention).
    #[serde(rename = "beeClass")]
    pub class: BeeClass,
    /// Biological sex, derived from the caste (see [`BeeClass::sex`]). Carried on
    /// the snapshot so the frontend needn't re-derive it.
    pub sex: Sex,
    pub state: BeeState,
    /// Remaining energy as a fraction in `[0, 1]`. The rail averages this across
    /// the colony for its energy readout.
    pub energy: f64,
    /// Wax scales the bee has secreted (workers only; `0` for every other
    /// caste). Renamed to `waxScales` on the wire.
    #[serde(rename = "waxScales")]
    pub wax_scales: f64,
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
            honey_stored: world.honey_stored,
            wax_grams: world.wax_grams,
        }
    }
}

impl BeeSnapshot {
    fn from_bee(bee: &Bee) -> Self {
        Self {
            id: bee.id.value(),
            position: bee.position,
            velocity: bee.velocity,
            class: bee.class,
            sex: bee.class.sex(),
            state: bee.state,
            energy: bee.energy,
            wax_scales: bee.wax_scales,
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
