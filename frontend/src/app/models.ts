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
 * What a bee is currently doing — a flat superset of every caste's states,
 * mirroring the Rust `BeeState` enum. Which states a bee actually visits is
 * gated by its {@link BeeClass}: workers wander/forage/rest and build comb,
 * the queen lays eggs (or rests), and drones loaf/fly (or rest).
 */
export type BeeState =
  | 'wandering'
  | 'foraging'
  | 'resting'
  | 'building_comb'
  | 'laying_eggs'
  | 'loafing'
  | 'flying';

/** The caste a bee belongs to, mirroring the Rust `BeeClass` enum. */
export type BeeClass = 'queen' | 'worker' | 'drone';

/** Biological sex, derived from caste on the Rust side (drones male; queen and workers female). */
export type Sex = 'male' | 'female';

export interface BeeSnapshot {
  id: number;
  position: Vec3;
  velocity: Vec3;
  /** The bee's caste (`class` is reserved in JS, so the wire key is `beeClass`). */
  beeClass: BeeClass;
  /** Biological sex, carried so the UI needn't re-derive it from the caste. */
  sex: Sex;
  state: BeeState;
  /** Remaining energy as a fraction in `[0, 1]`; the engine reports it for every bee. */
  energy: number;
  /** Wax scales secreted by this bee (workers only; `0` for other castes). */
  waxScales: number;
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
  /** Total comb wax the colony has produced, in grams (1000 secreted scales = 1 gram). */
  waxGrams: number;
}

/**
 * Control commands accepted by `POST /api/control`, mirroring
 * `colony_server::sim::Command` (serde externally tagged): the simple commands
 * are bare strings, while `set_speed` carries its tick-rate multiplier.
 */
export type ControlCommand = 'start' | 'pause' | 'reset' | { set_speed: number };
