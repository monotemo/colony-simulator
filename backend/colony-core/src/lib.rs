//! Core simulation domain for the bee colony simulator.
//!
//! This crate is pure: no async, no networking. It models the [`world::World`]
//! (bounds, bees, resources), the [`engine::Engine`] that advances time, and
//! the serializable [`snapshot::WorldSnapshot`] wire format. The server crate
//! drives the engine and streams snapshots to clients.

pub mod bee;
pub mod engine;
pub mod entity;
pub mod math;
pub mod snapshot;
pub mod world;

pub use bee::{Bee, BeeClass, BeeState, Sex};
pub use engine::Engine;
pub use entity::EntityId;
pub use math::Vec3;
pub use snapshot::{BeeSnapshot, ResourceSnapshot, WorldSnapshot};
pub use world::{Bounds, Resource, ResourceKind, World};
