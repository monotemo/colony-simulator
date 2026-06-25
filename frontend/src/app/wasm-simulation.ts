import { Injectable, signal, NgZone, inject, DestroyRef } from '@angular/core';
import { SimulationService } from './simulation.service';
import { WorldSnapshot } from './models';

/** Tick rate of the in-browser engine, matching the server's 30 Hz. */
const TICK_HZ = 30;

/** Minimal shape of the wasm-pack engine class (see `colony_wasm.d.ts`). */
interface WasmEngine {
  step(dt: number): void;
  reset(): void;
  snapshot_json(): string;
  free(): void;
}

/** Minimal shape of the wasm-pack ES module (`--target web`). */
interface WasmModule {
  default: (input?: unknown) => Promise<unknown>;
  WasmEngine: new () => WasmEngine;
}

/**
 * Runs the simulation entirely in the browser via WebAssembly — no backend
 * required. This powers the static GitHub Pages deployment.
 *
 * The wasm-pack output lives under `public/wasm/` and is copied to the site
 * root at build time. We load it from a runtime URL (relative to the page's
 * base href) so the bundler leaves it alone and it resolves correctly under the
 * `/colony-simulator/` Pages path.
 */
@Injectable({ providedIn: 'root' })
export class WasmSimulationService extends SimulationService {
  private readonly zone = inject(NgZone);

  readonly snapshot = signal<WorldSnapshot | null>(null);
  readonly connected = signal(false);
  // The engine begins stepping as soon as it loads.
  readonly running = signal(true);

  private engine?: WasmEngine;
  private timer?: ReturnType<typeof setInterval>;
  private disposed = false;
  /** Tick-rate multiplier set via {@link setSpeed} (0.5×, 1×, 2×). */
  private speed = 1;

  constructor() {
    super();
    void this.init();
    inject(DestroyRef).onDestroy(() => this.dispose());
  }

  start(): void {
    this.running.set(true);
  }

  pause(): void {
    this.running.set(false);
  }

  reset(): void {
    this.engine?.reset();
    this.publish();
  }

  override setSpeed(multiplier: number): void {
    if (multiplier <= 0 || multiplier === this.speed) {
      return;
    }
    this.speed = multiplier;
    // Re-arm the stepping loop at the new cadence (steps per real second). The
    // fixed `dt` keeps the physics stable; stepping more/less often scales the
    // simulation's apparent speed.
    if (this.timer !== undefined) {
      this.startLoop();
    }
  }

  private async init(): Promise<void> {
    // Resolve against the document base href so it works under the Pages
    // subpath; the `@vite-ignore` keeps the bundler from trying to resolve it.
    const url = new URL('wasm/colony_wasm.js', document.baseURI).href;
    const mod = (await import(/* @vite-ignore */ url)) as WasmModule;
    await mod.default();

    if (this.disposed) {
      return;
    }

    this.engine = new mod.WasmEngine();
    this.zone.run(() => this.connected.set(true));
    this.publish();

    this.startLoop();
  }

  /** (Re)start the stepping loop at the current {@link speed} multiplier. */
  private startLoop(): void {
    clearInterval(this.timer);
    const dt = 1 / TICK_HZ;
    this.timer = setInterval(() => {
      if (!this.running() || !this.engine) {
        return;
      }
      this.engine.step(dt);
      this.publish();
    }, 1000 / TICK_HZ / this.speed);
  }

  private publish(): void {
    if (!this.engine) {
      return;
    }
    const snapshot = JSON.parse(this.engine.snapshot_json()) as WorldSnapshot;
    this.zone.run(() => this.snapshot.set(snapshot));
  }

  private dispose(): void {
    this.disposed = true;
    clearInterval(this.timer);
    this.engine?.free();
    this.engine = undefined;
  }
}
