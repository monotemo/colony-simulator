//! The simulation task: owns the [`Engine`] and runs the fixed-timestep loop.
//!
//! The engine lives entirely inside one Tokio task. The outside world talks to
//! it through two channels:
//! - an [`mpsc`] command channel *in* (start / pause / reset), and
//! - a [`watch`] channel *out* carrying the latest [`WireFrame`] — the tick
//!   already encoded to wire bytes (see `colony_core::wire`), **once**, here.
//!   Connections only forward those shared bytes, so the encode cost no longer
//!   scales with the number of clients the way per-connection JSON did.
//!
//! A `watch` channel is the right tool for the outbound side: clients only ever
//! want the most recent frame, not a backlog of every historical tick. Because
//! a slow client can therefore *skip* frames, the roster (the rarely-changing
//! membership message) rides along inside every `WireFrame` with its version:
//! coalescing can never lose it, and each connection re-sends it only when its
//! peer hasn't seen the current version yet.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use colony_core::{Engine, WireEncoder};
use serde::Deserialize;
use tokio::sync::{mpsc, watch};

/// Simulation ticks per second.
const TICK_HZ: f64 = 30.0;

/// Draw a fresh entropy seed for a new or reshuffled colony. The engine itself
/// stays pure (no clock, no RNG of its own); entropy enters only here, at the
/// host boundary, so each launch and each reset yields a different colony while
/// the core remains reproducible given whatever seed we hand it.
///
/// We mix the wall clock with a process-lifetime counter so two resets landing
/// in the same nanosecond (or a coarse clock) still produce distinct seeds.
fn entropy_seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    nanos ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17)
}

/// A control command sent to the simulation task.
///
/// Also used directly as the body of `POST /api/control` (see
/// [`crate::ControlRequest`]). Serde's default (externally tagged) enum
/// encoding means the unit variants are bare strings (`"start"`) while
/// `SetSpeed` carries its payload (`{ "set_speed": 2.0 }`).
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    /// Resume stepping the simulation.
    Start,
    /// Stop stepping; state is held in place.
    Pause,
    /// Rebuild a fresh seeded world at tick 0.
    Reset,
    /// Scale the wall-clock tick rate by a multiplier (e.g. `0.5`, `1`, `2`).
    /// The step `dt` stays fixed for stable physics; only how often we step
    /// changes, so the simulation's apparent speed scales with it.
    SetSpeed(f64),
    /// Spawn a worker at a world-space point — the interactive "add a bee"
    /// perturbation. Encodes as `{ "spawn_bee": { "x": .., "y": .. } }`.
    SpawnBee { x: f64, y: f64 },
    /// Drop a nectar source at a world-space point — the interactive "add
    /// nectar" perturbation. Encodes as `{ "add_nectar": { "x": .., "y": .. } }`.
    AddNectar { x: f64, y: f64 },
}

/// One tick, encoded to wire bytes exactly once and shared by every client.
///
/// `roster` is always the *current* roster message (not just the last change),
/// so a connection that joined late — or skipped frames on the coalescing
/// watch channel — can always catch up from the latest frame alone. The `Arc`s
/// make the per-client clone a refcount bump, not a copy.
#[derive(Clone)]
pub struct WireFrame {
    /// Version of the roster in `roster`; connections track what they've sent.
    pub roster_version: u32,
    /// The complete current roster message (membership: bounds, castes, resources).
    pub roster: Arc<[u8]>,
    /// This tick's motion message (positions, velocities, energy, states).
    pub motion: Arc<[u8]>,
}

/// Handles for talking to a running simulation task.
#[derive(Clone)]
pub struct SimHandle {
    /// Latest encoded tick; subscribe with `.clone()` per WebSocket client.
    pub frames: watch::Receiver<WireFrame>,
    /// Send control commands to the simulation task.
    pub commands: mpsc::Sender<Command>,
}

/// Encode the engine's current state and publish it to subscribers.
fn publish(engine: &Engine, encoder: &mut WireEncoder, tx: &watch::Sender<WireFrame>) {
    let _ = tx.send(encode_frame(engine, encoder));
}

/// Encode one tick into the shared [`WireFrame`].
fn encode_frame(engine: &Engine, encoder: &mut WireEncoder) -> WireFrame {
    let frames = encoder.encode(&engine.snapshot());
    WireFrame {
        roster_version: frames.roster_version,
        roster: frames.roster,
        motion: frames.motion.into(),
    }
}

/// Spawn the simulation task and return handles to it.
///
/// The simulation starts in the running state.
pub fn spawn() -> SimHandle {
    // Seed from entropy so each server launch grows a different colony.
    let mut engine = Engine::from_seed(entropy_seed());
    let mut encoder = WireEncoder::new();
    let (snap_tx, snap_rx) = watch::channel(encode_frame(&engine, &mut encoder));
    let (cmd_tx, mut cmd_rx) = mpsc::channel::<Command>(32);

    tokio::spawn(async move {
        // Fixed physics step; only the wall-clock cadence below varies with speed.
        let dt = 1.0 / TICK_HZ;
        let mut speed = 1.0_f64;
        let mut interval = tokio::time::interval(Duration::from_secs_f64(dt / speed));
        let mut running = true;

        loop {
            interval.tick().await;

            // Drain any pending control commands for this tick.
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    Command::Start => running = true,
                    Command::Pause => running = false,
                    Command::Reset => {
                        // "Reshuffle": a fresh entropy seed, not a replay of the
                        // previous colony — determinism is no longer the goal.
                        engine.reset_with_seed(entropy_seed());
                        // Publish immediately so clients see the reset even
                        // while paused.
                        publish(&engine, &mut encoder, &snap_tx);
                    }
                    Command::SetSpeed(multiplier) => {
                        // Ignore non-positive/NaN multipliers; re-arm the timer
                        // at the new cadence only when it actually changed.
                        if multiplier > 0.0 && multiplier != speed {
                            speed = multiplier;
                            interval =
                                tokio::time::interval(Duration::from_secs_f64(dt / speed));
                        }
                    }
                    Command::SpawnBee { x, y } => {
                        engine.spawn_worker_at(x, y);
                        // Publish so the added bee shows up at once, even paused.
                        publish(&engine, &mut encoder, &snap_tx);
                    }
                    Command::AddNectar { x, y } => {
                        engine.add_nectar_at(x, y);
                        publish(&engine, &mut encoder, &snap_tx);
                    }
                }
            }

            if running {
                engine.step(dt);
                publish(&engine, &mut encoder, &snap_tx);
            }
        }
    });

    SimHandle {
        frames: snap_rx,
        commands: cmd_tx,
    }
}
