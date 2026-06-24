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
    /// Create an engine with the default seeded world.
    #[wasm_bindgen(constructor)]
    pub fn new() -> WasmEngine {
        WasmEngine {
            engine: Engine::seeded(),
        }
    }

    /// Advance the simulation by one fixed timestep of `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.engine.step(dt);
    }

    /// Reset to a fresh seeded world at tick 0.
    pub fn reset(&mut self) {
        self.engine.reset();
    }

    /// Serialize the current world state as a `WorldSnapshot` JSON string.
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.engine.snapshot()).expect("snapshot serializes")
    }
}

impl Default for WasmEngine {
    fn default() -> Self {
        Self::new()
    }
}
