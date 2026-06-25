import {
  Component,
  ChangeDetectionStrategy,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { WorldCanvas } from './world-canvas';
import { SimulationService } from './simulation.service';
import { BeeState } from './models';

/** Tick-rate multipliers offered by the speed segmented control. */
type Speed = 0.5 | 1 | 2;

/** One row of the live behavior breakdown in the stats rail. */
interface BehaviorRow {
  state: BeeState;
  label: string;
  count: number;
  /** Bar width as a fraction in `[0, 1]`, relative to the largest state. */
  fraction: number;
}

/**
 * The "Hearth" dashboard: header bar, the live three.js world with a floating
 * control dock, and a stats rail. All readouts derive from the live
 * {@link SimulationService} snapshot; controls delegate to the service and the
 * embedded {@link WorldCanvas}.
 */
@Component({
  selector: 'app-root',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [WorldCanvas],
  templateUrl: './app.html',
  styleUrl: './app.scss',
})
export class App {
  private readonly sim = inject(SimulationService);
  private readonly world = viewChild(WorldCanvas);

  /** Whether the engine is currently stepping (drives Start/Pause emphasis). */
  readonly running = this.sim.running;
  /** Active tick-rate multiplier. */
  readonly speed = signal<Speed>(1);
  readonly speeds: readonly Speed[] = [0.5, 1, 2];

  readonly connected = this.sim.connected;
  private readonly snapshot = this.sim.snapshot;

  /** Base engine tick rate, in Hz (both transports step at 30 Hz at 1× speed). */
  private readonly baseTickRate = 30;
  /** Effective tick rate shown in the header, scaled by the active speed. */
  readonly tickRate = computed(() => this.baseTickRate * this.speed());

  readonly tick = computed(() => this.snapshot()?.tick ?? 0);
  readonly population = computed(() => this.snapshot()?.bees.length ?? 0);

  /** Live per-state counts, with bar widths scaled to the busiest state. */
  readonly behavior = computed<BehaviorRow[]>(() => {
    const bees = this.snapshot()?.bees ?? [];
    const counts: Record<BeeState, number> = {
      wandering: 0,
      foraging: 0,
      resting: 0,
    };
    for (const bee of bees) {
      counts[bee.state] = (counts[bee.state] ?? 0) + 1;
    }
    const max = Math.max(1, counts.wandering, counts.foraging, counts.resting);
    const rows: { state: BeeState; label: string }[] = [
      { state: 'wandering', label: 'Wandering' },
      { state: 'foraging', label: 'Foraging' },
      { state: 'resting', label: 'Resting' },
    ];
    return rows.map((row) => ({
      ...row,
      count: counts[row.state],
      fraction: counts[row.state] / max,
    }));
  });

  /**
   * Average colony energy as a whole percentage. The engine does not model
   * per-bee energy yet, so when no bee reports it we show full (100%).
   */
  readonly colonyEnergy = computed(() => {
    const bees = this.snapshot()?.bees ?? [];
    if (bees.length === 0) {
      return 0;
    }
    const withEnergy = bees.filter((bee) => bee.energy != null);
    if (withEnergy.length === 0) {
      return 100;
    }
    const avg =
      withEnergy.reduce((sum, bee) => sum + (bee.energy ?? 0), 0) /
      withEnergy.length;
    return Math.round(avg * 100);
  });

  readonly nectarCount = computed(
    () =>
      this.snapshot()?.resources.filter((r) => r.kind === 'nectar').length ?? 0,
  );

  /** Honey in store as a whole percentage (0% until the engine models it). */
  readonly honeyStored = computed(() =>
    Math.round((this.snapshot()?.honeyStored ?? 0) * 100),
  );

  readonly zoomPercent = computed(() => this.world()?.zoomPercent() ?? 100);

  start(): void {
    this.sim.start();
  }

  pause(): void {
    this.sim.pause();
  }

  reset(): void {
    this.sim.reset();
  }

  setSpeed(speed: Speed): void {
    this.speed.set(speed);
    this.sim.setSpeed(speed);
  }

  zoomIn(): void {
    this.world()?.zoomIn();
  }

  zoomOut(): void {
    this.world()?.zoomOut();
  }
}
