/**
 * Wire types mirroring `colony_core::snapshot` on the Rust side.
 * Keep these in sync with `backend/colony-core/src/snapshot.rs`.
 */

export interface Vec2 {
  x: number;
  y: number;
}

export interface Bounds {
  width: number;
  height: number;
}

export type BeeState = 'wandering';

export interface BeeSnapshot {
  id: number;
  position: Vec2;
  state: BeeState;
}

export type ResourceKind = 'nectar';

export interface ResourceSnapshot {
  id: number;
  position: Vec2;
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
