/**
 * Wire types mirroring `colony_core::snapshot` on the Rust side.
 * Keep these in sync with `backend/colony-core/src/snapshot.rs`.
 */

export interface Vec3 {
  x: number;
  y: number;
  /**
   * The vertical / flight axis. Currently `0` for every entity (the world is
   * seeded flat) until flight behavior and depth rendering land; the canvas
   * renderer projects to the x/y plane and ignores `z` for now.
   */
  z: number;
}

export interface Bounds {
  width: number;
  height: number;
  /** Extent of the world along the z (flight) axis. */
  depth: number;
}

/**
 * What a bee is currently doing. All three variants now exist in the Rust
 * `BeeState` enum, but the engine doesn't transition between them yet — every
 * bee sits in `wandering` until the behavior state machine lands, at which point
 * the UI breakdown lights up automatically.
 */
export type BeeState = 'wandering' | 'foraging' | 'resting';

export interface BeeSnapshot {
  id: number;
  position: Vec3;
  velocity: Vec3;
  state: BeeState;
  /** Remaining energy as a fraction in `[0, 1]`; the engine reports it for every bee. */
  energy: number;
}

export type ResourceKind = 'nectar';

export interface ResourceSnapshot {
  id: number;
  position: Vec3;
  kind: ResourceKind;
}

/** A complete view of the world at a single tick. */
export interface WorldSnapshot {
  tick: number;
  bounds: Bounds;
  bees: BeeSnapshot[];
  resources: ResourceSnapshot[];
  /** Honey in store as a fraction in `[0, 1]`; the engine reports it each tick. */
  honeyStored: number;
}

/**
 * Control commands accepted by `POST /api/control`, mirroring
 * `colony_server::sim::Command` (serde externally tagged): the simple commands
 * are bare strings, while `set_speed` carries its tick-rate multiplier.
 */
export type ControlCommand = 'start' | 'pause' | 'reset' | { set_speed: number };
