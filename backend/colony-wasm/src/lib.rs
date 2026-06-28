//! WebAssembly bindings for the colony simulation.
//!
//! Wraps the pure [`colony_core::Engine`] so the browser can run the simulation
//! locally — this is what powers the static GitHub Pages deployment, where no
//! backend server is available. The native Axum server path remains the
//! authority for local development and future multi-client scenarios.

use colony_core::Engine;
use wasm_bindgen::prelude::*;

/// A simulation engine exposed to JavaScript.
///
/// Mirrors the small slice of [`Engine`] the frontend needs: advance time,
/// reset, and read a snapshot. The snapshot is returned as a JSON string in the
/// exact `WorldSnapshot` shape the WebSocket transport already sends, so the
/// frontend parses both identically.
#[wasm_bindgen]
pub struct WasmEngine {
    engine: Engine,
}

#[wasm_bindgen]
impl WasmEngine {
    /// Create an engine seeded from `seed`. The browser passes an entropy value
    /// (e.g. `Math.random() * 2**32`), so each page load grows a different
    /// colony; the engine stays reproducible for any fixed seed. A 32-bit seed
    /// is plenty of variety here and avoids the `u64`/BigInt boundary in JS.
    #[wasm_bindgen(constructor)]
    pub fn new(seed: u32) -> WasmEngine {
        WasmEngine {
            engine: Engine::from_seed(seed as u64),
        }
    }

    /// Advance the simulation by one fixed timestep of `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.engine.step(dt);
    }

    /// Reshuffle into a fresh colony at tick 0 using `seed` (the browser passes a
    /// new entropy value), so reset is a new colony rather than a replay.
    pub fn reset(&mut self, seed: u32) {
        self.engine.reset_with_seed(seed as u64);
    }

    /// Spawn a worker at a world-space point — the interactive "add a bee".
    pub fn spawn_bee(&mut self, x: f64, y: f64) {
        self.engine.spawn_worker_at(x, y);
    }

    /// Drop a nectar source at a world-space point — interactive "add nectar".
    pub fn add_nectar(&mut self, x: f64, y: f64) {
        self.engine.add_nectar_at(x, y);
    }

    /// Serialize the current world state as a `WorldSnapshot` JSON string.
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.engine.snapshot()).expect("snapshot serializes")
    }
}

impl Default for WasmEngine {
    fn default() -> Self {
        // A fixed-seed colony for the trait's no-argument constructor; live
        // callers always pass an entropy seed via `new`.
        Self::new(0)
    }
}
