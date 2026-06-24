import { Signal } from '@angular/core';
import { WorldSnapshot } from './models';

/**
 * Abstract contract for a simulation source. Components depend on this; the
 * concrete implementation is chosen at build time (see `app.config.ts`):
 *
 * - {@link WebSocketSimulationService} streams from the Rust server (local dev).
 * - {@link WasmSimulationService} runs the engine in-browser via WebAssembly
 *   (the static GitHub Pages build, where there is no backend).
 *
 * Doubles as the DI token, so `inject(SimulationService)` resolves to whichever
 * implementation was provided.
 */
export abstract class SimulationService {
  /** The most recent snapshot, or `null` before the first frame. */
  abstract readonly snapshot: Signal<WorldSnapshot | null>;
  /** Whether the source is live (socket open / wasm engine loaded). */
  abstract readonly connected: Signal<boolean>;

  /** Resume stepping the simulation. */
  abstract start(): void;
  /** Pause the simulation in place. */
  abstract pause(): void;
  /** Reset to a fresh seeded world. */
  abstract reset(): void;
}
