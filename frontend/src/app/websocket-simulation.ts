import { Injectable, signal, NgZone, inject, DestroyRef } from '@angular/core';
import { SimulationService } from './simulation.service';
import { ControlCommand, WorldSnapshot } from './models';
import { environment } from '../environments/environment';

/**
 * Streams the latest {@link WorldSnapshot} from the Rust server over a
 * WebSocket and sends control commands over REST.
 *
 * URLs come from `environment.backendUrl`: when empty (dev) they resolve to the
 * page origin, so the same build works behind the Angular dev-server proxy and
 * when served as static files by the Rust server; when set (production on
 * GitHub Pages) they point at the colony-server deployed on Fly.io, which the
 * server allows via its permissive CORS layer.
 */
@Injectable({ providedIn: 'root' })
export class WebSocketSimulationService extends SimulationService {
  private readonly zone = inject(NgZone);

  readonly snapshot = signal<WorldSnapshot | null>(null);
  readonly connected = signal(false);
  // The server spawns its simulation already running; we track command intent
  // optimistically since snapshots don't carry a running flag.
  readonly running = signal(true);

  private socket?: WebSocket;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private closed = false;

  constructor() {
    super();
    this.connect();
    inject(DestroyRef).onDestroy(() => this.disconnect());
  }

  start(): void {
    this.running.set(true);
    void this.control('start');
  }

  pause(): void {
    this.running.set(false);
    void this.control('pause');
  }

  reset(): void {
    void this.control('reset');
  }

  override setSpeed(multiplier: number): void {
    // The server owns the tick rate, so forward the multiplier and let the
    // simulation task re-arm its interval (see `colony_server::sim`).
    if (multiplier > 0) {
      void this.control({ set_speed: multiplier });
    }
  }

  spawnBee(x: number, y: number): void {
    void this.control({ spawn_bee: { x, y } });
  }

  addNectar(x: number, y: number): void {
    void this.control({ add_nectar: { x, y } });
  }

  /**
   * The `/ws` endpoint URL. Derived from `environment.backendUrl` when set
   * (swapping the http(s) scheme for ws(s)), otherwise from the page origin so
   * dev keeps working through the proxy.
   */
  private socketUrl(): string {
    if (environment.backendUrl) {
      return `${environment.backendUrl.replace(/^http/, 'ws')}/ws`;
    }
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    return `${proto}://${window.location.host}/ws`;
  }

  private async control(command: ControlCommand): Promise<void> {
    await fetch(`${environment.backendUrl}/api/control`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ command }),
    });
  }

  private connect(): void {
    const socket = new WebSocket(this.socketUrl());
    this.socket = socket;

    socket.onopen = () => this.zone.run(() => this.connected.set(true));

    socket.onmessage = (event) => {
      let parsed: WorldSnapshot;
      try {
        parsed = JSON.parse(event.data as string);
      } catch {
        return;
      }
      // Snapshots arrive ~30x/sec; run inside the zone so the signal update
      // drives change detection.
      this.zone.run(() => this.snapshot.set(parsed));
    };

    socket.onclose = () => {
      this.zone.run(() => this.connected.set(false));
      this.scheduleReconnect();
    };

    socket.onerror = () => socket.close();
  }

  private scheduleReconnect(): void {
    if (this.closed || this.reconnectTimer) {
      return;
    }
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.connect();
    }, 1000);
  }

  private disconnect(): void {
    this.closed = true;
    clearTimeout(this.reconnectTimer);
    this.socket?.close();
  }
}
