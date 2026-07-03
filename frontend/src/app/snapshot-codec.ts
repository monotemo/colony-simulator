/**
 * Binary snapshot decoder — a hand-maintained mirror of
 * `backend/colony-core/src/wire.rs`, exactly like `models.ts` mirrors
 * `snapshot.rs`. **Change one, change the other**, and keep the shared
 * byte-for-byte fixture in `snapshot-codec.spec.ts` and `wire.rs` in lockstep.
 *
 * The stream carries two message kinds (all values little-endian):
 *
 * - **Roster** (rare): the world's membership — bounds, each bee's id and
 *   caste, the resource nodes. Sent when a client connects and whenever
 *   membership changes (spawn, added nectar, reset), stamped with a version.
 * - **Motion** (every published tick): positions, velocities, energy, wax and
 *   behavior state as `f32` arrays keyed by index into the roster, plus the
 *   tick, colony totals, and the colony-wide aggregates the stats rail
 *   consumes. Carries the roster version it was built against, so a motion
 *   frame can never be applied to the wrong membership. Two shapes: **dense**
 *   (type 2, every bee in roster order) and **sparse** (type 3, only the bees
 *   inside the viewport this client reported, prefixed with their roster
 *   indices). Sparse aggregates still describe the whole colony.
 *
 * A packet handed to {@link SnapshotDecoder.decode} may hold one message (the
 * WebSocket transport sends roster and motion as separate frames) or several
 * concatenated (the wasm engine prepends the roster to the motion when it
 * changed). Both decode through the same loop; the decoder assembles the full
 * parsed {@link WorldSnapshot} the rest of the app already consumes, so
 * nothing outside the two transport services knows the wire changed.
 *
 * See `wire.rs` for the byte-level layout tables.
 */

import {
  BeeClass,
  BeeSnapshot,
  BeeState,
  Bounds,
  CasteCounts,
  ColonyStats,
  ResourceKind,
  ResourceSnapshot,
  Sex,
  WorldSnapshot,
} from './models';

/** First byte of a roster message (`wire.rs` `ROSTER_MESSAGE`). */
const ROSTER_MESSAGE = 1;
/** First byte of a dense motion message (`wire.rs` `MOTION_MESSAGE`). */
const MOTION_MESSAGE = 2;
/** First byte of a sparse motion message (`wire.rs` `SPARSE_MOTION_MESSAGE`). */
const SPARSE_MOTION_MESSAGE = 3;

/** Fixed bytes of a motion message before the index/array blocks begin. */
const MOTION_HEADER_LEN = 64;
/** Per-bee bytes in a motion message (position + velocity + energy + wax + state). */
const MOTION_BYTES_PER_BEE = 33;
/** Per-bee bytes in a sparse motion message (a u32 roster index on top). */
const SPARSE_BYTES_PER_BEE = MOTION_BYTES_PER_BEE + 4;

/** Wire byte → caste, in the exact order of `wire.rs` `class_byte`. */
const BEE_CLASSES: readonly BeeClass[] = ['queen', 'worker', 'drone'];

/** Wire byte → behavior state, in the exact order of `wire.rs` `state_byte`. */
const BEE_STATES: readonly BeeState[] = [
  'wandering',
  'foraging',
  'resting',
  'building_comb',
  'laying_eggs',
  'loafing',
  'flying',
];

/** Wire byte → resource kind, in the exact order of `wire.rs` `kind_byte`. */
const RESOURCE_KINDS: readonly ResourceKind[] = ['nectar'];

/**
 * Sex is a pure function of caste (`BeeClass::sex` in Rust), so it is not on
 * the wire — deriving it here keeps the two sides from ever drifting apart.
 */
const SEX_OF: Readonly<Record<BeeClass, Sex>> = {
  queen: 'female',
  worker: 'female',
  drone: 'male',
};

/** The decoded, immutable half of the stream: identity in motion-array order. */
interface Roster {
  version: number;
  bounds: Bounds;
  bees: readonly { id: number; beeClass: BeeClass; sex: Sex }[];
  resources: readonly ResourceSnapshot[];
  /** Caste tallies, derived once at parse — the roster always lists the whole colony. */
  casteCounts: CasteCounts;
}

/**
 * Stateful decoder: holds the current roster and joins each motion message
 * against it. One per transport connection — the server always (re)sends the
 * roster before any motion a fresh connection sees, so starting empty is safe.
 */
export class SnapshotDecoder {
  private roster: Roster | null = null;

  /**
   * Decode one packet (one or more concatenated wire messages). Returns the
   * assembled snapshot when the packet held a usable motion message, or `null`
   * for a roster-only packet or a motion that can't be applied (no roster yet,
   * or a version mismatch — possible only if frames arrive out of order, which
   * an ordered transport never produces; dropping the frame just means waiting
   * for the next roster).
   */
  decode(packet: ArrayBuffer | Uint8Array): WorldSnapshot | null {
    const view =
      packet instanceof Uint8Array
        ? new DataView(packet.buffer, packet.byteOffset, packet.byteLength)
        : new DataView(packet);

    let offset = 0;
    let snapshot: WorldSnapshot | null = null;
    while (offset < view.byteLength) {
      const type = view.getUint8(offset);
      if (type === ROSTER_MESSAGE) {
        offset = this.readRoster(view, offset);
      } else if (type === MOTION_MESSAGE || type === SPARSE_MOTION_MESSAGE) {
        const motion = this.readMotion(view, offset, type === SPARSE_MOTION_MESSAGE);
        offset = motion.next;
        snapshot = motion.snapshot ?? snapshot;
      } else {
        // Unknown message: the length is unknowable, so the rest of the packet
        // is undecodable. Drop it rather than misread garbage.
        console.warn(`unknown wire message type ${type}; dropping packet remainder`);
        break;
      }
    }
    return snapshot;
  }

  /** Parse a roster message at `offset`, adopt it, and return the next offset. */
  private readRoster(view: DataView, offset: number): number {
    const version = view.getUint32(offset + 1, true);
    const bounds: Bounds = {
      width: view.getFloat32(offset + 5, true),
      height: view.getFloat32(offset + 9, true),
      depth: view.getFloat32(offset + 13, true),
    };
    const beeCount = view.getUint32(offset + 17, true);
    const resourceCount = view.getUint32(offset + 21, true);
    let at = offset + 25;

    const bees = new Array<Roster['bees'][number]>(beeCount);
    const casteCounts: CasteCounts = { queen: 0, worker: 0, drone: 0 };
    for (let i = 0; i < beeCount; i++) {
      const id = view.getUint32(at, true);
      const beeClass = BEE_CLASSES[view.getUint8(at + 4)];
      bees[i] = { id, beeClass, sex: SEX_OF[beeClass] };
      casteCounts[beeClass]++;
      at += 5;
    }

    const resources = new Array<ResourceSnapshot>(resourceCount);
    for (let i = 0; i < resourceCount; i++) {
      resources[i] = {
        id: view.getUint32(at, true),
        kind: RESOURCE_KINDS[view.getUint8(at + 4)],
        position: {
          x: view.getFloat32(at + 5, true),
          y: view.getFloat32(at + 9, true),
          z: view.getFloat32(at + 13, true),
        },
      };
      at += 17;
    }

    this.roster = { version, bounds, bees, resources, casteCounts };
    return at;
  }

  /**
   * Parse a motion message (dense or sparse) at `offset` and join it with the
   * current roster. Always computes `next` (the message length depends only on
   * the bee count and shape), so decoding continues even when the snapshot
   * itself is unusable.
   */
  private readMotion(
    view: DataView,
    offset: number,
    sparse: boolean,
  ): { snapshot: WorldSnapshot | null; next: number } {
    const version = view.getUint32(offset + 1, true);
    const tick = view.getFloat64(offset + 5, true);
    const honeyStored = view.getFloat32(offset + 13, true);
    const waxGrams = view.getFloat32(offset + 17, true);
    const avgEnergy = view.getFloat32(offset + 21, true);
    const waxScalesTotal = view.getFloat32(offset + 25, true);
    const stateCounts = {} as Record<BeeState, number>;
    BEE_STATES.forEach((state, i) => {
      stateCounts[state] = view.getUint32(offset + 29 + i * 4, true);
    });
    // Bees carried by *this message* — the whole colony for a dense frame,
    // the visible subset for a sparse one.
    const beeCount = view.getUint32(offset + 57, true);
    const perBee = sparse ? SPARSE_BYTES_PER_BEE : MOTION_BYTES_PER_BEE;
    const next = offset + MOTION_HEADER_LEN + beeCount * perBee;

    const roster = this.roster;
    const countUsable = sparse
      ? beeCount <= (roster?.bees.length ?? 0)
      : beeCount === roster?.bees.length;
    if (!roster || roster.version !== version || !countUsable) {
      console.warn(
        `motion frame for roster v${version} (${beeCount} bees) does not match ` +
          `held roster ${roster ? `v${roster.version} (${roster.bees.length} bees)` : '(none)'}; dropping`,
      );
      return { snapshot: null, next };
    }

    // Sparse frames lead with the roster index of each carried bee; a dense
    // frame's arrays are simply in roster order.
    const indices = offset + MOTION_HEADER_LEN;
    const rosterIndex = sparse
      ? (i: number) => view.getUint32(indices + i * 4, true)
      : (i: number) => i;

    // Struct-of-arrays: each block is contiguous, per-bee values by index.
    const positions = indices + (sparse ? beeCount * 4 : 0);
    const velocities = positions + beeCount * 12;
    const energies = velocities + beeCount * 12;
    const waxScales = energies + beeCount * 4;
    const states = waxScales + beeCount * 4;

    const bees = new Array<BeeSnapshot>(beeCount);
    for (let i = 0; i < beeCount; i++) {
      const identity = roster.bees[rosterIndex(i)];
      bees[i] = {
        id: identity.id,
        beeClass: identity.beeClass,
        sex: identity.sex,
        position: {
          x: view.getFloat32(positions + i * 12, true),
          y: view.getFloat32(positions + i * 12 + 4, true),
          z: view.getFloat32(positions + i * 12 + 8, true),
        },
        velocity: {
          x: view.getFloat32(velocities + i * 12, true),
          y: view.getFloat32(velocities + i * 12 + 4, true),
          z: view.getFloat32(velocities + i * 12 + 8, true),
        },
        energy: view.getFloat32(energies + i * 4, true),
        waxScales: view.getFloat32(waxScales + i * 4, true),
        state: BEE_STATES[view.getUint8(states + i)],
      };
    }

    const stats: ColonyStats = {
      // The roster always lists the whole colony, so population and caste
      // tallies come from it; the per-tick aggregates ride the header.
      population: roster.bees.length,
      casteCounts: roster.casteCounts,
      stateCounts,
      avgEnergy,
      waxScalesTotal,
    };

    return {
      snapshot: {
        tick,
        bounds: roster.bounds,
        bees,
        resources: roster.resources.slice(),
        stats,
        honeyStored,
        waxGrams,
      },
      next,
    };
  }
}
