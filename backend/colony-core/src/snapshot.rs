//! Serializable snapshot types — the render-ready view of the world.
//!
//! Kept separate from the live simulation structs so internal fields can change
//! without breaking the client protocol. A snapshot is a flat, render-ready
//! view of the world at a single tick.
//!
//! Snapshots no longer ship as JSON: the wire encoding is the compact binary
//! format in [`crate::wire`] (roster + motion messages), decoded by the
//! frontend's `snapshot-codec.ts` back into this exact shape (`models.ts`).
//! The serde derives are kept — the camelCase renames still match `models.ts`,
//! so a JSON dump of a snapshot remains a faithful debug view of what the
//! client parses.

use serde::{Deserialize, Serialize};

use crate::bee::{Bee, BeeClass, BeeState, Sex};

impl ColonyStats {
    /// Aggregate over the live bee population in one pass. Sums run in
    /// bee-index order, matching how the world itself accumulates totals.
    fn tally(bees: &[Bee]) -> Self {
        let mut castes = CasteCounts { queen: 0, worker: 0, drone: 0 };
        let mut states = StateCounts {
            wandering: 0,
            foraging: 0,
            resting: 0,
            building_comb: 0,
            laying_eggs: 0,
            loafing: 0,
            flying: 0,
        };
        let mut energy_sum = 0.0;
        let mut wax_sum = 0.0;
        for bee in bees {
            match bee.class {
                BeeClass::Queen => castes.queen += 1,
                BeeClass::Worker => castes.worker += 1,
                BeeClass::Drone => castes.drone += 1,
            }
            match bee.state {
                BeeState::Wandering => states.wandering += 1,
                BeeState::Foraging => states.foraging += 1,
                BeeState::Resting => states.resting += 1,
                BeeState::BuildingComb => states.building_comb += 1,
                BeeState::LayingEggs => states.laying_eggs += 1,
                BeeState::Loafing => states.loafing += 1,
                BeeState::Flying => states.flying += 1,
            }
            energy_sum += bee.energy;
            wax_sum += bee.wax_scales;
        }
        Self {
            population: bees.len() as u32,
            caste_counts: castes,
            state_counts: states,
            avg_energy: if bees.is_empty() { 0.0 } else { energy_sum / bees.len() as f64 },
            wax_scales_total: wax_sum,
        }
    }
}
use crate::math::Vec3;
use crate::world::{Bounds, Resource, ResourceKind, World};

/// Colony-wide aggregates, computed once per snapshot so clients need not
/// (and, when the wire culls the bee list to a viewport, *cannot*) derive them
/// from per-bee data. The stats always describe the whole colony even when
/// `bees` carries only a visible subset.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColonyStats {
    /// Total number of bees alive, regardless of any viewport culling.
    pub population: u32,
    #[serde(rename = "casteCounts")]
    pub caste_counts: CasteCounts,
    #[serde(rename = "stateCounts")]
    pub state_counts: StateCounts,
    /// Mean of every bee's energy fraction, in `[0, 1]` (`0` for an empty world).
    #[serde(rename = "avgEnergy")]
    pub avg_energy: f64,
    /// Sum of every bee's secreted wax scales.
    #[serde(rename = "waxScalesTotal")]
    pub wax_scales_total: f64,
}

/// How many bees of each caste are alive.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CasteCounts {
    pub queen: u32,
    pub worker: u32,
    pub drone: u32,
}

/// How many bees are in each behavior state. Field order mirrors [`BeeState`]
/// declaration order — the wire codec writes them positionally (see
/// `crate::wire`), so keep the two in lockstep.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StateCounts {
    pub wandering: u32,
    pub foraging: u32,
    pub resting: u32,
    pub building_comb: u32,
    pub laying_eggs: u32,
    pub loafing: u32,
    pub flying: u32,
}

/// A complete, immutable view of the world at one tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub tick: u64,
    pub bounds: Bounds,
    pub bees: Vec<BeeSnapshot>,
    pub resources: Vec<ResourceSnapshot>,
    /// Colony-wide aggregates over *all* bees — the stats rail's source of
    /// truth, valid even when `bees` is culled to a viewport on the wire.
    pub stats: ColonyStats,
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
            stats: ColonyStats::tally(&world.bees),
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
