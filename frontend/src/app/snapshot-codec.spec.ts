import { SnapshotDecoder } from './snapshot-codec';
import { WorldSnapshot } from './models';

/**
 * The shared cross-language fixture: these hex strings are pinned
 * byte-for-byte by `fixture_encodes_to_pinned_bytes` in
 * `backend/colony-core/src/wire.rs`. If the wire layout changes, that test's
 * failure output is the source for the new strings — update both places
 * together. All numeric values in the fixture are exactly representable in
 * `f32`, so the decoded numbers compare exactly.
 */
const ROSTER_HEX =
  '01010000000000484400001644000048430200000001000000000000000007000000010900000000000020440000f04300000000';
const MOTION_HEX =
  '020100000000000000000045400000803e0000c03f0000203f0000404100000000010000000000000000000000010000000000000000000000020000000000000000c03f00002040000000000000c8420000494200000000000000bf0000804000000000000040400000c0bf000000000000403f0000003f00000000000040410401';
/** The same tick culled to just the worker (roster index 1) — a sparse frame. */
const SPARSE_MOTION_HEX =
  '030100000000000000000045400000803e0000c03f0000203f000040410000000001000000000000000000000001000000000000000000000001000000000000010000000000c8420000494200000000000040400000c0bf000000000000003f0000404101';

/** The snapshot those bytes encode (see `fixture_snapshot()` in `wire.rs`). */
const FIXTURE_SNAPSHOT: WorldSnapshot = {
  tick: 42,
  bounds: { width: 800, height: 600, depth: 200 },
  bees: [
    {
      id: 0,
      position: { x: 1.5, y: 2.5, z: 0 },
      velocity: { x: -0.5, y: 4, z: 0 },
      beeClass: 'queen',
      sex: 'female',
      state: 'laying_eggs',
      energy: 0.75,
      waxScales: 0,
    },
    {
      id: 7,
      position: { x: 100, y: 50.25, z: 0 },
      velocity: { x: 3, y: -1.5, z: 0 },
      beeClass: 'worker',
      sex: 'female',
      state: 'foraging',
      energy: 0.5,
      waxScales: 12,
    },
  ],
  resources: [{ id: 9, position: { x: 640, y: 480, z: 0 }, kind: 'nectar' }],
  stats: {
    population: 2,
    casteCounts: { queen: 1, worker: 1, drone: 0 },
    stateCounts: {
      wandering: 0,
      foraging: 1,
      resting: 0,
      building_comb: 0,
      laying_eggs: 1,
      loafing: 0,
      flying: 0,
    },
    avgEnergy: 0.625,
    waxScalesTotal: 12,
  },
  honeyStored: 0.25,
  waxGrams: 1.5,
};

function bytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/** Concatenate messages into one packet, as the wasm engine does. */
function packet(...parts: Uint8Array[]): Uint8Array {
  const out = new Uint8Array(parts.reduce((len, p) => len + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

describe('SnapshotDecoder', () => {
  it('decodes the Rust-pinned fixture into the full parsed snapshot', () => {
    // A combined roster+motion packet — the wasm engine's first frame.
    const decoder = new SnapshotDecoder();
    const snapshot = decoder.decode(packet(bytes(ROSTER_HEX), bytes(MOTION_HEX)));
    expect(snapshot).toEqual(FIXTURE_SNAPSHOT);
  });

  it('decodes roster and motion arriving as separate frames', () => {
    // The WebSocket transport's shape: one message per frame.
    const decoder = new SnapshotDecoder();
    expect(decoder.decode(bytes(ROSTER_HEX))).toBeNull();
    expect(decoder.decode(bytes(MOTION_HEX))).toEqual(FIXTURE_SNAPSHOT);
  });

  it('reuses the held roster across consecutive motion frames', () => {
    const decoder = new SnapshotDecoder();
    decoder.decode(bytes(ROSTER_HEX));
    expect(decoder.decode(bytes(MOTION_HEX))).toEqual(FIXTURE_SNAPSHOT);
    expect(decoder.decode(bytes(MOTION_HEX))).toEqual(FIXTURE_SNAPSHOT);
  });

  it('accepts an ArrayBuffer as well as a Uint8Array', () => {
    const decoder = new SnapshotDecoder();
    const buf = packet(bytes(ROSTER_HEX), bytes(MOTION_HEX));
    const copy = new ArrayBuffer(buf.byteLength);
    new Uint8Array(copy).set(buf);
    expect(decoder.decode(copy)).toEqual(FIXTURE_SNAPSHOT);
  });

  it('decodes a sparse frame: visible subset, whole-colony stats', () => {
    const decoder = new SnapshotDecoder();
    decoder.decode(bytes(ROSTER_HEX));
    const snapshot = decoder.decode(bytes(SPARSE_MOTION_HEX))!;
    expect(snapshot).not.toBeNull();
    // Only the worker (roster index 1) is on screen…
    expect(snapshot.bees).toEqual([FIXTURE_SNAPSHOT.bees[1]]);
    // …but the aggregates, resources, and totals still describe the colony.
    expect(snapshot.stats).toEqual(FIXTURE_SNAPSHOT.stats);
    expect(snapshot.resources).toEqual(FIXTURE_SNAPSHOT.resources);
    expect(snapshot.tick).toBe(FIXTURE_SNAPSHOT.tick);
    expect(snapshot.honeyStored).toBe(FIXTURE_SNAPSHOT.honeyStored);
  });

  it('drops a sparse frame whose roster index is out of range', () => {
    const decoder = new SnapshotDecoder();
    decoder.decode(bytes(ROSTER_HEX));
    const corrupt = bytes(SPARSE_MOTION_HEX);
    corrupt[64] = 9; // the lone roster index (u32 LE at offset 64) — roster has 2 bees
    expect(decoder.decode(corrupt)).toBeNull();
    // Warn-and-drop, not a thrown decode: the next good frame still lands.
    expect(decoder.decode(bytes(SPARSE_MOTION_HEX))).not.toBeNull();
  });

  it('drops a motion frame with no roster to join against', () => {
    const decoder = new SnapshotDecoder();
    expect(decoder.decode(bytes(MOTION_HEX))).toBeNull();
  });

  it('drops a motion frame whose roster version does not match', () => {
    const decoder = new SnapshotDecoder();
    decoder.decode(bytes(ROSTER_HEX));
    const stale = bytes(MOTION_HEX);
    stale[1] = 99; // forge the roster version (u32 LE at offset 1)
    expect(decoder.decode(stale)).toBeNull();
    // The held roster is untouched: a good motion frame still decodes.
    expect(decoder.decode(bytes(MOTION_HEX))).toEqual(FIXTURE_SNAPSHOT);
  });

  it('drops a packet with an unknown message type', () => {
    const decoder = new SnapshotDecoder();
    expect(decoder.decode(new Uint8Array([255, 1, 2, 3]))).toBeNull();
  });
});
