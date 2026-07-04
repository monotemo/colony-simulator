import { Injectable, signal, NgZone, inject, DestroyRef } from '@angular/core';
import { SimulationService } from './simulation.service';
import { ControlCommand, ViewportRect, WorldSnapshot } from './models';
import { SnapshotDecoder } from './snapshot-codec';
import { environment } from '../environments/environment';

/** First reconnect delay; doubles per failed attempt up to the cap below. */
const INITIAL_RECONNECT_DELAY_MS = 1000;
/**
 * Ceiling on the reconnect backoff. Long enough not to hammer a downed backend,
 * short enough that recovery (including a scale-to-zero cold start on a
 * self-hosted server) is picked up promptly once the server is back.
 */
const MAX_RECONNECT_DELAY_MS = 15000;

/**
 * Streams the latest {@link WorldSnapshot} from the Rust server over a
 * WebSocket and sends control commands over REST.
 *
 * Frames arrive as the binary wire format (see {@link SnapshotDecoder}): a
 * roster message whenever this connection hasn't seen the current colony
 * membership, then one compact motion message per tick — not JSON.
 *
 * URLs come from `environment.backendUrl`: when empty (dev) they resolve to the
 * page origin, so the same build works behind the Angular dev-server proxy and
 * when served as static files by the Rust server; when set they point at a
 * colony-server hosted elsewhere (the deployed Cloudflare origin is static-only,
 * so a live-stream build must name its server), which the server allows via
 * its permissive CORS layer.
 */
@Injectable({ providedIn: 'root' })
export class WebSocketSimulationService extends SimulationService {
  private readonly zone = inject(NgZone);

  readonly snapshot = signal<WorldSnapshot | null>(null);
  readonly connected = signal(false);
  // The server spawns its simulation already running; we track command intent
  // optimistically since snapshots don't carry a running flag.
  readonly running = signal(true);

  /**
   * `environment.backendUrl` with any trailing slash trimmed, so path
   * concatenation can't produce `//ws` / `//api/control` (which some servers
   * reject). Empty in dev, meaning same-origin URLs.
   */
  private readonly backend = environment.backendUrl.replace(/\/+$/, '');

  private socket?: WebSocket;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  /** Current reconnect backoff; reset on a successful open. */
  private reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
  private closed = false;
  /**
   * The last viewport the canvas reported, so it can be replayed to the server
   * whenever a (re)connection opens — culling state is per-connection there.
   */
  private viewport?: ViewportRect;

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

  override setViewport(rect: ViewportRect): void {
    this.viewport = rect;
    this.sendViewport();
  }

  /**
   * Tell the server what this client can see, as the `{"viewport": ...}` text
   * message `colony_server`'s socket handler expects (snake_case keys, matching
   * its serde shape). The server then culls per-tick motion to this rect.
   */
  private sendViewport(): void {
    const rect = this.viewport;
    if (!rect || this.socket?.readyState !== WebSocket.OPEN) {
      return;
    }
    this.socket.send(
      JSON.stringify({
        viewport: { min_x: rect.minX, min_y: rect.minY, max_x: rect.maxX, max_y: rect.maxY },
      }),
    );
  }

  /**
   * The `/ws` endpoint URL. Derived from `environment.backendUrl` when set
   * (swapping the http(s) scheme for ws(s)), otherwise from the page origin so
   * dev keeps working through the proxy.
   */
  private socketUrl(): string {
    if (this.backend) {
      return `${this.backend.replace(/^http/, 'ws')}/ws`;
    }
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    return `${proto}://${window.location.host}/ws`;
  }

  private async control(command: ControlCommand): Promise<void> {
    // Fire-and-forget from the caller's view, but don't swallow failures
    // silently: a rejected command or an unreachable backend is logged so it's
    // diagnosable rather than vanishing.
    try {
      const response = await fetch(`${this.backend}/api/control`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ command }),
      });
      if (!response.ok) {
        console.warn(
          `control command rejected (${response.status} ${response.statusText})`,
          command,
        );
      }
    } catch (error) {
      console.warn('control command failed to reach the backend', command, error);
    }
  }

  private connect(): void {
    const socket = new WebSocket(this.socketUrl());
    socket.binaryType = 'arraybuffer';
    this.socket = socket;
    // A fresh decoder per connection: the server sends the roster before any
    // motion a new connection sees, so no state may carry across reconnects.
    const decoder = new SnapshotDecoder();

    socket.onopen = () =>
      this.zone.run(() => {
        this.connected.set(true);
        // Good connection — start the backoff over for the next drop.
        this.reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
        // Culling state is per-connection on the server, so replay the canvas's
        // viewport (if it reported one) to the fresh socket.
        this.sendViewport();
      });

    socket.onmessage = (event) => {
      if (!(event.data instanceof ArrayBuffer)) {
        return;
      }
      const snapshot = decoder.decode(event.data);
      if (!snapshot) {
        return; // roster-only frame; the next motion completes the picture
      }
      // Snapshots arrive ~30x/sec; run inside the zone so the signal update
      // drives change detection.
      this.zone.run(() => this.snapshot.set(snapshot));
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
    const delay = this.reconnectDelay;
    // Capped exponential backoff so a sustained outage isn't hammered, while a
    // brief drop (or a cold-starting backend) still reconnects quickly.
    this.reconnectDelay = Math.min(this.reconnectDelay * 2, MAX_RECONNECT_DELAY_MS);
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.connect();
    }, delay);
  }

  private disconnect(): void {
    this.closed = true;
    clearTimeout(this.reconnectTimer);
    this.socket?.close();
  }
}
