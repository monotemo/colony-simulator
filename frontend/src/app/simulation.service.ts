import { Injectable, signal, NgZone, inject, DestroyRef } from '@angular/core';
import { ControlCommand, WorldSnapshot } from './models';

/**
 * Connects to the Rust simulation server: streams the latest
 * {@link WorldSnapshot} over a WebSocket and sends control commands over REST.
 *
 * The WebSocket and REST URLs are derived from the page origin, so the same
 * build works both behind the Angular dev-server proxy and when served as
 * static files directly by the Rust server.
 */
@Injectable({ providedIn: 'root' })
export class SimulationService {
  private readonly zone = inject(NgZone);

  /** The most recent snapshot received, or `null` before the first frame. */
  readonly snapshot = signal<WorldSnapshot | null>(null);
  /** Whether the WebSocket is currently open. */
  readonly connected = signal(false);

  private socket?: WebSocket;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private closed = false;

  constructor() {
    this.connect();
    inject(DestroyRef).onDestroy(() => this.disconnect());
  }

  /** Resume stepping the simulation. */
  start(): Promise<void> {
    return this.control('start');
  }

  /** Pause the simulation in place. */
  pause(): Promise<void> {
    return this.control('pause');
  }

  /** Reset to a fresh seeded world. */
  reset(): Promise<void> {
    return this.control('reset');
  }

  private async control(command: ControlCommand): Promise<void> {
    await fetch('/api/control', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ command }),
    });
  }

  private connect(): void {
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const url = `${proto}://${window.location.host}/ws`;
    const socket = new WebSocket(url);
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
