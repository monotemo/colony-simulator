import {
  Component,
  ChangeDetectionStrategy,
  computed,
  inject,
  signal,
  viewChild,
} from '@angular/core';
import { PointerTool, WorldCanvas } from './world-canvas';
import { SimulationService } from './simulation.service';
import { BeeClass, BeeSnapshot, BeeState } from './models';

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

/** State key → label, for one-off lookups (e.g. the follow-cam chip). */
const STATE_LABEL_BY_KEY = new Map<BeeState, string>(
  STATE_LABELS.map((row) => [row.state, row.label]),
);

/** Human-readable caste names, for the follow-cam chip. */
const CASTE_LABELS: Record<BeeClass, string> = {
  queen: 'Queen',
  worker: 'Worker',
  drone: 'Drone',
};

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

  // Every colony-wide readout below derives from `snapshot().stats` — the
  // engine-computed aggregates — never by tallying `snapshot().bees`. That
  // list may be culled to the reported viewport, so it only ever answers
  // "what is on screen", not "what does the colony look like".
  readonly population = computed(() => this.snapshot()?.stats.population ?? 0);

  /**
   * Live per-state counts, with bar widths scaled to the busiest state. Only
   * states actually present this tick get a row, so the breakdown grows to cover
   * caste-specific states (building comb, laying eggs, loafing, flying) as those
   * bees appear without ever showing empty rows.
   */
  readonly behavior = computed<BehaviorRow[]>(() => {
    const counts = this.snapshot()?.stats.stateCounts;
    if (!counts) {
      return [];
    }
    const max = Math.max(1, ...Object.values(counts));
    return STATE_LABELS.filter((row) => counts[row.state] > 0).map((row) => ({
      ...row,
      count: counts[row.state],
      fraction: counts[row.state] / max,
    }));
  });

  /** Live caste breakdown: how many queens, workers, and drones are alive. */
  readonly castes = computed<CasteRow[]>(() => {
    const counts = this.snapshot()?.stats.casteCounts;
    const rows: { caste: BeeClass; label: string }[] = [
      { caste: 'queen', label: 'Queen' },
      { caste: 'worker', label: 'Worker' },
      { caste: 'drone', label: 'Drone' },
    ];
    return rows.map((row) => ({ ...row, count: counts?.[row.caste] ?? 0 }));
  });

  /** Number of queens in the colony, for the population chip. */
  readonly queenCount = computed(
    () => this.castes().find((row) => row.caste === 'queen')?.count ?? 0,
  );

  /** Total wax scales secreted across the colony (workers only). */
  readonly waxScales = computed(() => this.snapshot()?.stats.waxScalesTotal ?? 0);

  /** Comb wax the colony has produced, in grams (1000 scales = 1 gram). */
  readonly waxGrams = computed(() => this.snapshot()?.waxGrams ?? 0);

  /** Average colony energy as a whole percentage (0% for an empty colony). */
  readonly colonyEnergy = computed(() => {
    const stats = this.snapshot()?.stats;
    if (!stats || stats.population === 0) {
      return 0;
    }
    return Math.round(stats.avgEnergy * 100);
  });

  readonly nectarCount = computed(
    () => this.snapshot()?.resources.filter((r) => r.kind === 'nectar').length ?? 0,
  );

  /** Honey in store as a whole percentage (0% until the engine models it). */
  readonly honeyStored = computed(() => Math.round((this.snapshot()?.honeyStored ?? 0) * 100));

  readonly zoomPercent = computed(() => this.world()?.zoomPercent() ?? 100);

  /** Active pointer tool, mirrored from the canvas for the dock's button state. */
  readonly tool = computed<PointerTool>(() => this.world()?.tool() ?? 'follow');

  /** Id of the bee the follow-cam is locked onto, or `null` when free. */
  readonly followedBeeId = computed(() => this.world()?.followedBeeId() ?? null);

  /**
   * The live snapshot of the followed bee, looked up fresh each tick so the dock
   * chip tracks its changing state. `null` when nothing is followed or the bee
   * has left the colony (the canvas releases the follow on the same tick).
   */
  readonly followedBee = computed<BeeSnapshot | null>(() => {
    const id = this.followedBeeId();
    if (id === null) {
      return null;
    }
    return this.snapshot()?.bees.find((bee) => bee.id === id) ?? null;
  });

  /** Friendly "Caste · State" summary of the followed bee for the dock chip. */
  readonly followLabel = computed(() => {
    const bee = this.followedBee();
    if (!bee) {
      return '';
    }
    const caste = CASTE_LABELS[bee.beeClass] ?? bee.beeClass;
    const state = STATE_LABEL_BY_KEY.get(bee.state) ?? bee.state;
    return `${caste} · ${state}`;
  });

  start(): void {
    this.sim.start();
  }

  pause(): void {
    this.sim.pause();
  }

  reset(): void {
    this.sim.reset();
  }

  /**
   * Toggle a placement tool on the canvas: clicking the active tool's button
   * again returns to the follow-cam. Clicks on the world then drop a bee or
   * nectar source instead of following bees.
   */
  toggleTool(tool: Exclude<PointerTool, 'follow'>): void {
    const world = this.world();
    if (!world) {
      return;
    }
    world.setTool(world.tool() === tool ? 'follow' : tool);
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

  /** Release the follow-cam, re-framing the whole world. */
  stopFollow(): void {
    this.world()?.clearFollow();
  }
}
