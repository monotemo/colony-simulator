import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { App } from './app';
import { SimulationService } from './simulation.service';

/** Minimal stand-in so components that inject the service can be created. */
class StubSimulationService implements Partial<SimulationService> {
  readonly snapshot = signal(null);
  readonly connected = signal(false);
  start(): void {}
  pause(): void {}
  reset(): void {}
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
});
