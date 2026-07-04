/**
 * Snapshot types mirroring `colony_core::snapshot` on the Rust side.
 * Keep these in sync with `backend/colony-core/src/snapshot.rs`.
 *
 * These are the *parsed* shapes the app consumes. On the wire they travel as
 * the compact binary format defined in `backend/colony-core/src/wire.rs` and
 * decoded by `snapshot-codec.ts` — that pair is its own mirrored contract.
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
  'wandering' | 'foraging' | 'resting' | 'building_comb' | 'laying_eggs' | 'loafing' | 'flying';

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

/** How many bees of each caste are alive, mirroring the Rust `CasteCounts`. */
export interface CasteCounts {
  queen: number;
  worker: number;
  drone: number;
}

/**
 * Colony-wide aggregates, mirroring the Rust `ColonyStats`. Computed engine-
 * side once per snapshot so the stats rail never needs the full bee list —
 * which matters because `WorldSnapshot.bees` may be culled to the viewport
 * the client reported (see `setViewport`). Always describes the whole colony.
 */
export interface ColonyStats {
  /** Total bees alive, regardless of any viewport culling of `bees`. */
  population: number;
  casteCounts: CasteCounts;
  stateCounts: Record<BeeState, number>;
  /** Mean energy fraction across every bee, in `[0, 1]` (`0` when empty). */
  avgEnergy: number;
  /** Sum of every bee's secreted wax scales. */
  waxScalesTotal: number;
}

/**
 * A world-space rectangle of what a client's camera can see. Sent to the
 * server (snake_case keys on the wire, mirroring `colony_server`'s `Viewport`)
 * so it can cull per-tick motion to the visible bees.
 */
export interface ViewportRect {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
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
  /**
   * The bees this client can see. Usually the whole colony, but a client that
   * reported a viewport receives only the bees inside it — derive colony-wide
   * readouts from {@link stats}, never by aggregating this list.
   */
  bees: BeeSnapshot[];
  resources: ResourceSnapshot[];
  /** Colony-wide aggregates, valid even when `bees` is a culled subset. */
  stats: ColonyStats;
  /** Honey in store as a fraction in `[0, 1]`; the engine reports it each tick. */
  honeyStored: number;
  /** Total comb wax the colony has produced, in grams (1000 secreted scales = 1 gram). */
  waxGrams: number;
}

/**
 * Control commands accepted by `POST /api/control`, mirroring
 * `colony_server::sim::Command` (serde externally tagged): the simple commands
 * are bare strings, while the structured ones carry a payload — `set_speed` a
 * tick-rate multiplier, and the world-perturbation commands a world-space point.
 */
export type ControlCommand =
  | 'start'
  | 'pause'
  | 'reset'
  | { set_speed: number }
  | { spawn_bee: { x: number; y: number } }
  | { add_nectar: { x: number; y: number } };
