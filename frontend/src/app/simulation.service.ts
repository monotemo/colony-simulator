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
  /**
   * Whether the simulation is currently stepping. Both transports start in the
   * running state, so this is the source of truth for Start/Pause emphasis —
   * the UI binds to it rather than tracking its own guess.
   */
  abstract readonly running: Signal<boolean>;

  /** Resume stepping the simulation. */
  abstract start(): void;
  /** Pause the simulation in place. */
  abstract pause(): void;
  /** Reset to a fresh seeded world. */
  abstract reset(): void;

  /**
   * Set the tick-rate multiplier (e.g. `0.5`, `1`, `2`). Sources that cannot
   * vary their rate (the fixed-rate server stream) may ignore this; the
   * in-browser wasm engine honours it. Defaults to a no-op.
   */
  setSpeed(_multiplier: number): void {}
}
