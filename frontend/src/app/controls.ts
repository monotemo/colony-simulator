import { Component, inject, ChangeDetectionStrategy } from '@angular/core';
import { SimulationService } from './simulation.service';

/** Start / Pause / Reset buttons plus a live connection + tick readout. */
@Component({
  selector: 'app-controls',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="controls">
      <button type="button" (click)="sim.start()">Start</button>
      <button type="button" (click)="sim.pause()">Pause</button>
      <button type="button" (click)="sim.reset()">Reset</button>

      <span class="status" [class.online]="sim.connected()">
        {{ sim.connected() ? 'connected' : 'disconnected' }}
      </span>
      <span class="tick">tick {{ sim.snapshot()?.tick ?? 0 }}</span>
      <span class="count">{{ sim.snapshot()?.bees?.length ?? 0 }} bees</span>
    </div>
  `,
  styles: [
    `
      .controls {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        margin-bottom: 0.75rem;
        flex-wrap: wrap;
      }
      button {
        padding: 0.4rem 0.9rem;
        border: 1px solid #3a3a40;
        border-radius: 6px;
        background: #2a2a30;
        color: #eee;
        cursor: pointer;
      }
      button:hover {
        background: #34343c;
      }
      .status {
        color: #c0392b;
        font-variant: small-caps;
      }
      .status.online {
        color: #27ae60;
      }
      .tick,
      .count {
        color: #aaa;
        font-variant-numeric: tabular-nums;
      }
    `,
  ],
})
export class Controls {
  readonly sim = inject(SimulationService);
}
