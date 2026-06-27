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
import { BeeClass, BeeState } from './models';

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

/** One row of the live caste breakdown in the stats rail. */
interface CasteRow {
  caste: BeeClass;
  label: string;
  count: number;
}

/** Display order and labels for every behavior state the engine can report. */
const STATE_LABELS: readonly { state: BeeState; label: string }[] = [
  { state: 'wandering', label: 'Wandering' },
  { state: 'foraging', label: 'Foraging' },
  { state: 'building_comb', label: 'Building comb' },
  { state: 'laying_eggs', label: 'Laying eggs' },
  { state: 'loafing', label: 'Loafing' },
  { state: 'flying', label: 'Flying' },
  { state: 'resting', label: 'Resting' },
];

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

  /**
   * Live per-state counts, with bar widths scaled to the busiest state. Only
   * states actually present this tick get a row, so the breakdown grows to cover
   * caste-specific states (building comb, laying eggs, loafing, flying) as those
   * bees appear without ever showing empty rows.
   */
  readonly behavior = computed<BehaviorRow[]>(() => {
    const bees = this.snapshot()?.bees ?? [];
    const counts = new Map<BeeState, number>();
    for (const bee of bees) {
      counts.set(bee.state, (counts.get(bee.state) ?? 0) + 1);
    }
    const max = Math.max(1, ...counts.values());
    return STATE_LABELS.filter((row) => (counts.get(row.state) ?? 0) > 0).map(
      (row) => ({
        ...row,
        count: counts.get(row.state) ?? 0,
        fraction: (counts.get(row.state) ?? 0) / max,
      }),
    );
  });

  /** Live caste breakdown: how many queens, workers, and drones are alive. */
  readonly castes = computed<CasteRow[]>(() => {
    const bees = this.snapshot()?.bees ?? [];
    const counts = new Map<BeeClass, number>();
    for (const bee of bees) {
      counts.set(bee.beeClass, (counts.get(bee.beeClass) ?? 0) + 1);
    }
    const rows: { caste: BeeClass; label: string }[] = [
      { caste: 'queen', label: 'Queen' },
      { caste: 'worker', label: 'Worker' },
      { caste: 'drone', label: 'Drone' },
    ];
    return rows.map((row) => ({ ...row, count: counts.get(row.caste) ?? 0 }));
  });

  /** Number of queens in the colony, for the population chip. */
  readonly queenCount = computed(
    () => this.castes().find((row) => row.caste === 'queen')?.count ?? 0,
  );

  /** Total wax scales secreted across the colony (workers only). */
  readonly waxScales = computed(() => {
    const bees = this.snapshot()?.bees ?? [];
    return bees.reduce((sum, bee) => sum + (bee.waxScales ?? 0), 0);
  });

  /** Comb wax the colony has produced, in grams (1000 scales = 1 gram). */
  readonly waxGrams = computed(() => this.snapshot()?.waxGrams ?? 0);

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
