//! The simulation task: owns the [`Engine`] and runs the fixed-timestep loop.
//!
//! The engine lives entirely inside one Tokio task. The outside world talks to
//! it through two channels:
//! - an [`mpsc`] command channel *in* (start / pause / reset), and
//! - a [`watch`] channel *out* carrying the latest [`WorldSnapshot`].
//!
//! A `watch` channel is the right tool for the outbound side: clients only ever
//! want the most recent frame, not a backlog of every historical tick.

use std::time::Duration;

use colony_core::{Engine, WorldSnapshot};
use serde::Deserialize;
use tokio::sync::{mpsc, watch};

/// Simulation ticks per second.
const TICK_HZ: f64 = 30.0;

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
}

/// Handles for talking to a running simulation task.
#[derive(Clone)]
pub struct SimHandle {
    /// Latest world snapshot; subscribe with `.clone()` per WebSocket client.
    pub snapshots: watch::Receiver<WorldSnapshot>,
    /// Send control commands to the simulation task.
    pub commands: mpsc::Sender<Command>,
}

/// Spawn the simulation task and return handles to it.
///
/// The simulation starts in the running state.
pub fn spawn() -> SimHandle {
    let mut engine = Engine::seeded();
    let (snap_tx, snap_rx) = watch::channel(engine.snapshot());
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
                        engine.reset();
                        // Publish immediately so clients see the reset even
                        // while paused.
                        let _ = snap_tx.send(engine.snapshot());
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
                }
            }

            if running {
                engine.step(dt);
                let _ = snap_tx.send(engine.snapshot());
            }
        }
    });

    SimHandle {
        snapshots: snap_rx,
        commands: cmd_tx,
    }
}
