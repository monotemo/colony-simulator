//! Binary wire codec — the compact frame format streamed to clients.
//!
//! This replaces per-tick JSON: a full snapshot serialized as text cost ~300
//! bytes *per bee* per tick, re-serialized for every client. The binary codec
//! splits the stream into two message kinds so immutable data leaves the wire
//! entirely:
//!
//! - **Roster** (rare): everything about the world that only changes when its
//!   *membership* changes — bounds, each bee's id and caste, and the resource
//!   nodes (whose id/kind/position are fixed for their lifetime). Re-issued
//!   only when its content actually differs (spawn, added nectar, reset), and
//!   stamped with a version.
//! - **Motion** (every tick): the per-tick dynamics — positions, velocities,
//!   energy, wax and behavior state — as little-endian `f32` arrays keyed *by
//!   index into the roster*, plus the tick and colony totals. Carries the
//!   roster version it was built against so a client can never mis-apply it.
//!
//! The decoder is `frontend/src/app/snapshot-codec.ts` — a hand-maintained
//! mirror of this module, exactly like `models.ts` mirrors `snapshot.rs`.
//! **Change one, change the other**, and keep the byte-for-byte fixture tests
//! at the bottom of this file and in `snapshot-codec.spec.ts` in lockstep: they
//! pin the same encoded bytes on both sides of the language boundary.
//!
//! ## Layout (all integers/floats little-endian)
//!
//! Roster message (`5 + 20 + 5·bees + 17·resources` bytes):
//!
//! | offset | type | field                                  |
//! |--------|------|----------------------------------------|
//! | 0      | u8   | message type = 1                       |
//! | 1      | u32  | roster version (starts at 1)           |
//! | 5      | f32  | bounds.width                           |
//! | 9      | f32  | bounds.height                          |
//! | 13     | f32  | bounds.depth                           |
//! | 17     | u32  | bee count `n`                          |
//! | 21     | u32  | resource count `m`                     |
//! | 25     | n ×  | { u32 id, u8 class }                   |
//! | 25+5n  | m ×  | { u32 id, u8 kind, f32 x, f32 y, f32 z }|
//!
//! Motion message (`28 + 33·bees` bytes; the arrays are struct-of-arrays so a
//! future renderer can view them as typed arrays without re-parsing):
//!
//! | offset  | type | field                                 |
//! |---------|------|---------------------------------------|
//! | 0       | u8   | message type = 2                      |
//! | 1       | u32  | roster version this motion refers to  |
//! | 5       | f64  | tick (exact for ticks < 2^53)         |
//! | 13      | f32  | honey_stored                          |
//! | 17     | f32  | wax_grams                              |
//! | 21      | u32  | bee count `n` (must match the roster) |
//! | 25      | 3×u8 | zero padding (4-aligns the arrays)    |
//! | 28      | n×3×f32 | positions (x, y, z per bee)        |
//! | 28+12n  | n×3×f32 | velocities (x, y, z per bee)       |
//! | 28+24n  | n×f32   | energies                           |
//! | 28+28n  | n×f32   | wax scales                         |
//! | 28+32n  | n×u8    | behavior states                    |
//!
//! Positions ship as `f32`: the renderer draws pixels, and a ~7-digit mantissa
//! is far more precision than a screen needs. The simulation itself stays
//! `f64` — narrowing happens only here, at the wire.
//!
//! A bee's sex is *not* on the wire: it is a pure function of caste
//! ([`BeeClass::sex`]), so the TS decoder derives it, keeping the two from
//! ever drifting apart.

use std::sync::Arc;

use crate::bee::{BeeClass, BeeState};
use crate::snapshot::WorldSnapshot;
use crate::world::ResourceKind;

/// First byte of a roster message.
pub const ROSTER_MESSAGE: u8 = 1;
/// First byte of a motion message.
pub const MOTION_MESSAGE: u8 = 2;

/// Fixed bytes of a motion message before the per-bee arrays begin.
const MOTION_HEADER_LEN: usize = 28;
/// Per-bee bytes in a motion message (position + velocity + energy + wax + state).
const MOTION_BYTES_PER_BEE: usize = 33;

/// Caste on the wire, in `BeeClass` declaration order. The TS decoder's
/// `BEE_CLASSES` table must list the names in this exact order.
fn class_byte(class: BeeClass) -> u8 {
    match class {
        BeeClass::Queen => 0,
        BeeClass::Worker => 1,
        BeeClass::Drone => 2,
    }
}

/// Behavior state on the wire, in `BeeState` declaration order. The TS
/// decoder's `BEE_STATES` table must list the names in this exact order.
fn state_byte(state: BeeState) -> u8 {
    match state {
        BeeState::Wandering => 0,
        BeeState::Foraging => 1,
        BeeState::Resting => 2,
        BeeState::BuildingComb => 3,
        BeeState::LayingEggs => 4,
        BeeState::Loafing => 5,
        BeeState::Flying => 6,
    }
}

/// Resource kind on the wire, in `ResourceKind` declaration order. The TS
/// decoder's `RESOURCE_KINDS` table must list the names in this exact order.
fn kind_byte(kind: ResourceKind) -> u8 {
    match kind {
        ResourceKind::Nectar => 0,
    }
}

fn put_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_le_bytes());
}

fn put_f32(buf: &mut Vec<u8>, v: f64) {
    buf.extend_from_slice(&(v as f32).to_le_bytes());
}

fn put_f64(buf: &mut Vec<u8>, v: f64) {
    buf.extend_from_slice(&v.to_le_bytes());
}

/// An entity id narrowed for the wire. Ids are handed out monotonically from
/// zero, so u32 gives four billion spawns of headroom; the assert makes an
/// overflow loud rather than a silent aliasing of two entities.
fn put_id(buf: &mut Vec<u8>, id: u64) {
    let id = u32::try_from(id).expect("entity id fits in u32 on the wire");
    put_u32(buf, id);
}

/// The encoded frames for one tick: the motion message, plus a handle to the
/// current roster so a host can (re)send it to any client that lacks it.
pub struct TickFrames {
    /// Version of the roster the motion was built against.
    pub roster_version: u32,
    /// True when *this* encode re-issued the roster (membership changed, or
    /// this was the first encode).
    pub roster_changed: bool,
    /// The complete current roster message. Shared, not copied — the server
    /// hands the same allocation to every connection.
    pub roster: Arc<[u8]>,
    /// This tick's motion message.
    pub motion: Vec<u8>,
}

/// Stateful encoder: tracks the roster it last issued so unchanged membership
/// costs no roster bytes on the wire.
///
/// One encoder per engine (the server's sim task and the wasm wrapper each own
/// one). Change detection is by *content*: the candidate roster body is
/// rebuilt each tick into a reused scratch buffer and byte-compared, so any
/// membership change — spawn, added nectar, a reset to a fresh colony — is
/// caught without the engine having to report events.
#[derive(Debug, Default)]
pub struct WireEncoder {
    /// Version of the roster currently in `roster`; 0 means none issued yet.
    version: u32,
    /// Body (everything after the type byte and version) of the current roster.
    body: Vec<u8>,
    /// The complete current roster message, shared with hosts.
    roster: Arc<[u8]>,
    /// Reused build buffer for the candidate body, so unchanged ticks allocate
    /// nothing for the roster path.
    scratch: Vec<u8>,
}

impl WireEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Encode one tick. Always produces a motion message; re-issues the roster
    /// only when its content changed since the last call.
    pub fn encode(&mut self, snapshot: &WorldSnapshot) -> TickFrames {
        self.scratch.clear();
        write_roster_body(&mut self.scratch, snapshot);

        let changed = self.version == 0 || self.scratch != self.body;
        if changed {
            self.version += 1;
            std::mem::swap(&mut self.body, &mut self.scratch);
            let mut msg = Vec::with_capacity(5 + self.body.len());
            msg.push(ROSTER_MESSAGE);
            put_u32(&mut msg, self.version);
            msg.extend_from_slice(&self.body);
            self.roster = msg.into();
        }

        TickFrames {
            roster_version: self.version,
            roster_changed: changed,
            roster: Arc::clone(&self.roster),
            motion: encode_motion(snapshot, self.version),
        }
    }
}

/// Everything in the roster except the leading type byte and version — the
/// part whose bytes decide whether a new roster needs to go out.
fn write_roster_body(buf: &mut Vec<u8>, snapshot: &WorldSnapshot) {
    buf.reserve(20 + snapshot.bees.len() * 5 + snapshot.resources.len() * 17);
    put_f32(buf, snapshot.bounds.width);
    put_f32(buf, snapshot.bounds.height);
    put_f32(buf, snapshot.bounds.depth);
    put_u32(buf, snapshot.bees.len() as u32);
    put_u32(buf, snapshot.resources.len() as u32);
    for bee in &snapshot.bees {
        put_id(buf, bee.id);
        buf.push(class_byte(bee.class));
    }
    for res in &snapshot.resources {
        put_id(buf, res.id);
        buf.push(kind_byte(res.kind));
        put_f32(buf, res.position.x);
        put_f32(buf, res.position.y);
        put_f32(buf, res.position.z);
    }
}

/// The per-tick dynamics, keyed by index into the roster of `version`.
fn encode_motion(snapshot: &WorldSnapshot, version: u32) -> Vec<u8> {
    let n = snapshot.bees.len();
    let mut buf = Vec::with_capacity(MOTION_HEADER_LEN + n * MOTION_BYTES_PER_BEE);

    buf.push(MOTION_MESSAGE);
    put_u32(&mut buf, version);
    put_f64(&mut buf, snapshot.tick as f64);
    put_f32(&mut buf, snapshot.honey_stored);
    put_f32(&mut buf, snapshot.wax_grams);
    put_u32(&mut buf, n as u32);
    buf.extend_from_slice(&[0u8; 3]);

    for bee in &snapshot.bees {
        put_f32(&mut buf, bee.position.x);
        put_f32(&mut buf, bee.position.y);
        put_f32(&mut buf, bee.position.z);
    }
    for bee in &snapshot.bees {
        put_f32(&mut buf, bee.velocity.x);
        put_f32(&mut buf, bee.velocity.y);
        put_f32(&mut buf, bee.velocity.z);
    }
    for bee in &snapshot.bees {
        put_f32(&mut buf, bee.energy);
    }
    for bee in &snapshot.bees {
        put_f32(&mut buf, bee.wax_scales);
    }
    for bee in &snapshot.bees {
        buf.push(state_byte(bee.state));
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::math::Vec3;
    use crate::snapshot::{BeeSnapshot, ResourceSnapshot};
    use crate::world::Bounds;
    use crate::Sex;

    /// A tiny hand-built snapshot with values exactly representable in `f32`,
    /// so the encoded bytes are stable and the TS decoder can assert exact
    /// round-tripped numbers. This is the shared fixture with
    /// `snapshot-codec.spec.ts` — change it there too.
    fn fixture_snapshot() -> WorldSnapshot {
        WorldSnapshot {
            tick: 42,
            bounds: Bounds::new(800.0, 600.0, 200.0),
            bees: vec![
                BeeSnapshot {
                    id: 0,
                    position: Vec3::new(1.5, 2.5, 0.0),
                    velocity: Vec3::new(-0.5, 4.0, 0.0),
                    class: BeeClass::Queen,
                    sex: Sex::Female,
                    state: BeeState::LayingEggs,
                    energy: 0.75,
                    wax_scales: 0.0,
                },
                BeeSnapshot {
                    id: 7,
                    position: Vec3::new(100.0, 50.25, 0.0),
                    velocity: Vec3::new(3.0, -1.5, 0.0),
                    class: BeeClass::Worker,
                    sex: Sex::Female,
                    state: BeeState::Foraging,
                    energy: 0.5,
                    wax_scales: 12.0,
                },
            ],
            resources: vec![ResourceSnapshot {
                id: 9,
                position: Vec3::new(640.0, 480.0, 0.0),
                kind: ResourceKind::Nectar,
            }],
            honey_stored: 0.25,
            wax_grams: 1.5,
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The exact bytes both codecs must agree on. Mirrored (same hex strings)
    /// in `frontend/src/app/snapshot-codec.spec.ts`; regenerate by updating
    /// the expected strings from this test's failure output.
    #[test]
    fn fixture_encodes_to_pinned_bytes() {
        let mut encoder = WireEncoder::new();
        let frames = encoder.encode(&fixture_snapshot());

        assert_eq!(
            hex(&frames.roster),
            "01010000000000484400001644000048430200000001000000000000000007000000010900000000000020440000f04300000000"
        );
        assert_eq!(
            hex(&frames.motion),
            "020100000000000000000045400000803e0000c03f020000000000000000c03f00002040000000000000c8420000494200000000000000bf0000804000000000000040400000c0bf000000000000403f0000003f00000000000040410401"
        );
    }

    #[test]
    fn roster_and_motion_have_documented_sizes() {
        let snapshot = fixture_snapshot();
        let mut encoder = WireEncoder::new();
        let frames = encoder.encode(&snapshot);
        let (n, m) = (snapshot.bees.len(), snapshot.resources.len());
        assert_eq!(frames.roster.len(), 25 + 5 * n + 17 * m);
        assert_eq!(frames.motion.len(), MOTION_HEADER_LEN + MOTION_BYTES_PER_BEE * n);
    }

    #[test]
    fn first_encode_issues_the_roster_at_version_one() {
        let mut encoder = WireEncoder::new();
        let frames = encoder.encode(&fixture_snapshot());
        assert!(frames.roster_changed);
        assert_eq!(frames.roster_version, 1);
        assert_eq!(frames.roster[0], ROSTER_MESSAGE);
        assert_eq!(frames.motion[0], MOTION_MESSAGE);
    }

    #[test]
    fn unchanged_membership_reuses_the_roster() {
        let mut engine = Engine::seeded();
        let mut encoder = WireEncoder::new();
        let first = encoder.encode(&engine.snapshot());

        engine.step(1.0 / 30.0);
        let second = encoder.encode(&engine.snapshot());

        assert!(!second.roster_changed);
        assert_eq!(second.roster_version, first.roster_version);
        // Same shared allocation, not merely equal bytes.
        assert!(Arc::ptr_eq(&second.roster, &first.roster));
        // The dynamics still advanced.
        assert_ne!(second.motion, first.motion);
    }

    #[test]
    fn membership_changes_bump_the_roster_version() {
        let mut engine = Engine::seeded();
        let mut encoder = WireEncoder::new();
        let first = encoder.encode(&engine.snapshot());

        // Each perturbation that alters membership must re-issue the roster…
        engine.spawn_worker_at(50.0, 50.0);
        let spawned = encoder.encode(&engine.snapshot());
        assert!(spawned.roster_changed);
        assert_eq!(spawned.roster_version, first.roster_version + 1);

        engine.add_nectar_at(10.0, 10.0);
        let fed = encoder.encode(&engine.snapshot());
        assert!(fed.roster_changed);
        assert_eq!(fed.roster_version, spawned.roster_version + 1);

        // …including a reset, which rebuilds the colony wholesale.
        engine.reset_with_seed(12345);
        let reset = encoder.encode(&engine.snapshot());
        assert!(reset.roster_changed);
        assert_eq!(reset.roster_version, fed.roster_version + 1);
    }

    #[test]
    fn motion_carries_its_roster_version() {
        let mut engine = Engine::seeded();
        let mut encoder = WireEncoder::new();
        let frames = encoder.encode(&engine.snapshot());
        let version = u32::from_le_bytes(frames.motion[1..5].try_into().unwrap());
        assert_eq!(version, frames.roster_version);

        engine.spawn_worker_at(50.0, 50.0);
        let bumped = encoder.encode(&engine.snapshot());
        let version = u32::from_le_bytes(bumped.motion[1..5].try_into().unwrap());
        assert_eq!(version, bumped.roster_version);
    }

    #[test]
    fn every_state_and_class_has_a_distinct_wire_byte() {
        // The byte tables are position-sensitive mirrors of the TS decoder;
        // exhaustiveness is enforced by the `match`es, distinctness here.
        let states = [
            BeeState::Wandering,
            BeeState::Foraging,
            BeeState::Resting,
            BeeState::BuildingComb,
            BeeState::LayingEggs,
            BeeState::Loafing,
            BeeState::Flying,
        ];
        for (i, s) in states.iter().enumerate() {
            assert_eq!(state_byte(*s) as usize, i);
        }
        let classes = [BeeClass::Queen, BeeClass::Worker, BeeClass::Drone];
        for (i, c) in classes.iter().enumerate() {
            assert_eq!(class_byte(*c) as usize, i);
        }
        assert_eq!(kind_byte(ResourceKind::Nectar), 0);
    }
}
