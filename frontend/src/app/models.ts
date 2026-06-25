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

export type BeeState = 'wandering';

export interface BeeSnapshot {
  id: number;
  position: Vec3;
  velocity: Vec3;
  state: BeeState;
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
}

/** Control commands accepted by `POST /api/control`. */
export type ControlCommand = 'start' | 'pause' | 'reset';
