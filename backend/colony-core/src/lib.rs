//! Core simulation domain for the bee colony simulator.
//!
//! This crate is pure: no async, no networking. It models the [`world::World`]
//! (bounds, bees, resources), the [`engine::Engine`] that advances time, the
//! [`snapshot::WorldSnapshot`] view of a tick, and the [`wire`] codec that
//! encodes snapshots into the compact binary frames streamed to clients. The
//! server and wasm crates drive the engine and ship those frames.

pub mod bee;
pub mod engine;
pub mod entity;
pub mod math;
pub mod rng;
pub mod snapshot;
pub mod wire;
pub mod world;

pub use bee::{Bee, BeeClass, BeeState, Sex};
pub use engine::Engine;
pub use entity::EntityId;
pub use math::Vec3;
pub use snapshot::{BeeSnapshot, ResourceSnapshot, WorldSnapshot};
pub use wire::{TickFrames, WireEncoder};
pub use world::{Bounds, Resource, ResourceKind, World};
