import {
  Component,
  ElementRef,
  effect,
  inject,
  viewChild,
  ChangeDetectionStrategy,
} from '@angular/core';
import { SimulationService } from './simulation.service';
import { WorldSnapshot } from './models';

/**
 * Renders the world onto a `<canvas>`: the bounds rectangle, nectar resources,
 * and every bee as a dot. Redraws whenever a new snapshot arrives.
 *
 * This is a flat projection onto the x/y plane — positions carry a `z` (flight)
 * axis that is intentionally ignored here until depth rendering lands. Entities
 * are all at `z = 0` today, so the projection is currently lossless.
 */
@Component({
  selector: 'app-world-canvas',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<canvas #canvas class="world"></canvas>`,
  styles: [
    `
      .world {
        display: block;
        width: 100%;
        height: auto;
        background: #1b1b1f;
        border: 1px solid #3a3a40;
        border-radius: 8px;
      }
    `,
  ],
})
export class WorldCanvas {
  private readonly sim = inject(SimulationService);
  private readonly canvas =
    viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  constructor() {
    // Redraw on every new snapshot once the canvas element exists.
    effect(() => {
      const snapshot = this.sim.snapshot();
      const canvasEl = this.canvas().nativeElement;
      if (snapshot) {
        this.draw(canvasEl, snapshot);
      }
    });
  }

  private draw(canvas: HTMLCanvasElement, snapshot: WorldSnapshot): void {
    const { width, height } = snapshot.bounds;
    // Size the backing store to the world so we can draw in world coordinates.
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }

    const ctx = canvas.getContext('2d');
    if (!ctx) {
      return;
    }

    ctx.clearRect(0, 0, width, height);

    // Resources (nectar) — amber diamonds.
    ctx.fillStyle = '#f4b942';
    for (const resource of snapshot.resources) {
      const { x, y } = resource.position;
      ctx.beginPath();
      ctx.moveTo(x, y - 7);
      ctx.lineTo(x + 7, y);
      ctx.lineTo(x, y + 7);
      ctx.lineTo(x - 7, y);
      ctx.closePath();
      ctx.fill();
    }

    // Bees — small dots.
    ctx.fillStyle = '#ffd23f';
    for (const bee of snapshot.bees) {
      ctx.beginPath();
      ctx.arc(bee.position.x, bee.position.y, 4, 0, Math.PI * 2);
      ctx.fill();
    }
  }
}
