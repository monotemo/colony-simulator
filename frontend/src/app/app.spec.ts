import { WritableSignal, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { App } from './app';
import { SimulationService } from './simulation.service';
import { BeeClass, BeeSnapshot, BeeState, WorldSnapshot } from './models';

/** Minimal stand-in so components that inject the service can be created. */
class StubSimulationService implements Partial<SimulationService> {
  readonly snapshot: WritableSignal<WorldSnapshot | null> = signal(null);
  readonly connected = signal(false);
  readonly running = signal(false);
  start(): void {}
  pause(): void {}
  reset(): void {}
}

/** Build a bee snapshot, defaulting the fields a given test doesn't care about. */
function bee(over: Partial<BeeSnapshot> & { beeClass: BeeClass; state: BeeState }): BeeSnapshot {
  return {
    id: 0,
    position: { x: 0, y: 0, z: 0 },
    velocity: { x: 0, y: 0, z: 0 },
    sex: over.beeClass === 'drone' ? 'male' : 'female',
    energy: 1,
    waxScales: 0,
    ...over,
  };
}

function world(bees: BeeSnapshot[], waxGrams = 0): WorldSnapshot {
  return {
    tick: 1,
    bounds: { width: 100, height: 100, depth: 100 },
    bees,
    resources: [],
    honeyStored: 0,
    waxGrams,
  };
}

describe('App', () => {
  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [App],
      providers: [
        { provide: SimulationService, useClass: StubSimulationService },
      ],
    }).compileComponents();
  });

  it('should create the app', () => {
    const fixture = TestBed.createComponent(App);
    const app = fixture.componentInstance;
    expect(app).toBeTruthy();
  });

  it('should render the title heading', () => {
    const fixture = TestBed.createComponent(App);
    fixture.detectChanges();
    const compiled = fixture.nativeElement as HTMLElement;
    expect(compiled.querySelector('h1')?.textContent).toContain('Colony Simulator');
  });

  it('breaks the colony down by caste and totals its wax', () => {
    const fixture = TestBed.createComponent(App);
    const sim = TestBed.inject(SimulationService) as unknown as StubSimulationService;
    sim.snapshot.set(
      world(
        [
          bee({ beeClass: 'queen', state: 'laying_eggs' }),
          bee({ beeClass: 'worker', state: 'wandering' }),
          bee({ beeClass: 'worker', state: 'building_comb', waxScales: 2.5 }),
          bee({ beeClass: 'drone', state: 'loafing' }),
        ],
        0.0025,
      ),
    );

    const app = fixture.componentInstance;
    const casteCount = (caste: BeeClass) =>
      app.castes().find((row) => row.caste === caste)?.count ?? 0;
    expect(casteCount('queen')).toBe(1);
    expect(casteCount('worker')).toBe(2);
    expect(casteCount('drone')).toBe(1);
    expect(app.queenCount()).toBe(1);

    // Only states actually present show up in the behavior breakdown.
    const states = app.behavior().map((row) => row.state);
    expect(states).toContain('building_comb');
    expect(states).toContain('laying_eggs');
    expect(states).not.toContain('foraging');

    expect(app.waxScales()).toBe(2.5);
    expect(app.waxGrams()).toBe(0.0025);
  });
});
