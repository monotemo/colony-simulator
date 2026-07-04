import {
  Component,
  ElementRef,
  NgZone,
  OnDestroy,
  afterNextRender,
  effect,
  inject,
  signal,
  viewChild,
  ChangeDetectionStrategy,
} from '@angular/core';
import * as THREE from 'three';
import { SimulationService } from './simulation.service';
import { BeeClass, BeeSnapshot, BeeState, Vec3, ViewportRect, WorldSnapshot } from './models';

/**
 * What a click on the world does: follow a bee (the default), or place a new
 * entity at the clicked point. The placement tools drive the interactive
 * world-perturbation controls.
 */
export type PointerTool = 'follow' | 'spawnBee' | 'addNectar';

/**
 * Renders the world with three.js in the "Hearth" honey-and-hive palette: bees
 * as small striped, winged shapes tinted by behavior state and turned to face
 * their heading, nectar resources as terracotta discs, and a central beehive —
 * a tiered skep silhouette that domes outward as the colony banks wax — with
 * its queen just above it.
 *
 * The view is a top-down orthographic camera looking straight down the world's
 * `z` (flight) axis, so the screen shows the x/y plane. World `y` grows downward
 * on screen (origin top-left). `z` raises an entity toward the camera; it is `0`
 * for every entity today but is mapped through so depth/flight rendering works
 * for free once the simulation populates it.
 *
 * The canvas is transparent: the warm radial-gradient "stage" behind it (styled
 * in the dashboard SCSS) shows through as the colony floor, so there is no
 * opaque ground plane.
 *
 * Snapshots arrive at ~10 Hz (the transports publish every third engine tick),
 * so the canvas *interpolates*: each new snapshot becomes the target and a
 * requestAnimationFrame loop glides every bee from where the previous snapshot
 * left it. The loop is bounded — it stops the moment it has caught up with the
 * latest snapshot — so a paused or disconnected simulation costs no frames.
 * Zoom and resize still redraw immediately. The canvas also reports the world
 * rect the camera can see to the simulation source (see {@link reportViewport})
 * so the server can cull off-screen bees from the stream.
 */
@Component({
  selector: 'app-world-canvas',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<canvas #canvas class="world"></canvas>`,
  styles: [
    `
      :host {
        display: block;
        width: 100%;
        height: 100%;
      }
      .world {
        display: block;
        width: 100%;
        height: 100%;
        /* The canvas is interactive: click a bee to lock the follow-cam on it. */
        cursor: pointer;
      }
    `,
  ],
})
export class WorldCanvas implements OnDestroy {
  private readonly sim = inject(SimulationService);
  private readonly zone = inject(NgZone);
  private readonly canvas = viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  /** Current camera zoom as a whole percentage, for the dock readout. */
  readonly zoomPercent = signal(100);

  /**
   * The active pointer tool. In `follow` mode a click locks the follow-cam onto
   * a bee (or releases it on empty space); in a placement mode a click drops a
   * new bee or nectar source at the clicked world point instead. The dashboard
   * dock toggles this (see {@link setTool}) and reads it to light the active
   * button.
   */
  readonly tool = signal<PointerTool>('follow');

  /**
   * Stable id of the bee the follow-cam is locked onto, or `null` when the
   * camera is free and framing the whole world. Clicking a bee sets it; clicking
   * empty space (or the dock's clear button) releases it. It is a signal so the
   * dashboard can show a "following" chip and so the snapshot render effect
   * re-centres the moment the selection changes, even while paused.
   */
  readonly followedBeeId = signal<number | null>(null);

  // three.js handles, created in afterNextRender (browser only).
  private renderer?: THREE.WebGLRenderer;
  private scene?: THREE.Scene;
  private camera?: THREE.OrthographicCamera;
  private resizeObserver?: ResizeObserver;
  private raycaster?: THREE.Raycaster;
  // The world floor (`z = 0`) as a math plane, for mapping a click back to a
  // world-space point when a placement tool is active. Reused across clicks.
  private readonly groundPlane = new THREE.Plane(new THREE.Vector3(0, 0, 1), 0);
  private readonly onWheel = (event: WheelEvent) => this.handleWheel(event);
  private readonly onClick = (event: MouseEvent) => this.handleClick(event);

  // Shared geometry/materials — one of each, reused across every entity. A bee
  // is composed from a handful of flat shapes laid out in the x/y plane: an
  // elongated body pointing along +x (its heading), a dark head at the tip,
  // dark abdomen stripes, and a pair of translucent wings. Every bee `Group`
  // reuses these singletons; only the lightweight `Group`/child meshes are
  // per-entity (see {@link createBee}).
  private readonly beeBodyGeometry = new THREE.ShapeGeometry(this.ellipse(8, 5), 24);
  private readonly beeStripeGeometry = new THREE.ShapeGeometry(this.ellipse(1.1, 4.6), 16);
  private readonly beeWingGeometry = new THREE.ShapeGeometry(this.ellipse(3.6, 6), 20);
  private readonly beeHeadGeometry = new THREE.CircleGeometry(3, 16);
  // Body tinted by behavior state; the emissive lift gives the soft "glow" the
  // design calls for. Each caste mostly occupies its own states, so the state
  // tints double as caste cues: amber wax for a building worker, royal gold for
  // the laying queen, a muted brown for a loafing drone. Combined with the
  // per-caste scale (see {@link classScale}), castes read at a glance.
  private readonly beeMaterials: Record<BeeState, THREE.MeshStandardMaterial> = {
    wandering: this.glowMaterial(0xe2a12b, 0xf3b84a, 0.45),
    foraging: this.glowMaterial(0x7c8b5a, 0x9aae6e, 0.35),
    resting: this.glowMaterial(0xc99a38, 0xc99a38, 0.12),
    building_comb: this.glowMaterial(0xe6c34a, 0xf2d873, 0.4),
    laying_eggs: this.glowMaterial(0xe0a12b, 0xffe08a, 0.7),
    loafing: this.glowMaterial(0x9a8050, 0xb39863, 0.2),
    flying: this.glowMaterial(0xcdb06a, 0xe7cf8a, 0.4),
  };
  // Per-caste body scale, so a queen looms over her workers and drones sit a
  // notch larger — size carries the caste even when two share a state colour.
  private readonly classScale: Record<BeeClass, number> = {
    queen: 1.7,
    drone: 1.25,
    worker: 1.0,
  };
  // Near-black bands for the head and abdomen stripes — the bee's contrast.
  private readonly beeMarkingMaterial = this.glowMaterial(0x2a1c08, 0x3a2a10, 0.1);
  // Gauzy wings: translucent and depth-write off so they blend over the body
  // (and each other) without occluding it.
  private readonly beeWingMaterial = new THREE.MeshStandardMaterial({
    color: 0xffffff,
    emissive: 0xfff4dd,
    emissiveIntensity: 0.25,
    transparent: true,
    opacity: 0.4,
    roughness: 0.3,
    depthWrite: false,
    side: THREE.DoubleSide,
  });
  private readonly flowerGeometry = new THREE.CircleGeometry(9, 24);
  private readonly flowerMaterial = this.glowMaterial(0xc9663c, 0xe68a5e, 0.25);

  // Central landmarks, derived from the world bounds (no wire data yet).
  //
  // The hive is a small "skep" silhouette: horizontal bands tapering from a
  // wide base to a cap knob, built as a flat sprite-like Group (like a bee or
  // flower) from stacked ellipses rather than the plain hexagon disc it used
  // to be. Coordinates below are in an unscaled unit space; the whole group
  // is scaled by a world-derived, wax-grown radius (see updateHiveScale), so
  // this layout only needs to look like a hive at scale 1.
  private static readonly HIVE_TIERS: ReadonlyArray<{ rx: number; ry: number; y: number }> = [
    { rx: 1.0, ry: 0.34, y: -0.62 },
    { rx: 0.84, ry: 0.32, y: -0.34 },
    { rx: 0.66, ry: 0.3, y: -0.08 },
    { rx: 0.48, ry: 0.26, y: 0.16 },
    { rx: 0.28, ry: 0.22, y: 0.36 },
  ];
  private readonly hiveTierGeometries = WorldCanvas.HIVE_TIERS.map(
    ({ rx, ry }) => new THREE.ShapeGeometry(this.ellipse(rx, ry), 24),
  );
  private readonly hiveKnobGeometry = new THREE.CircleGeometry(0.14, 16);
  private readonly hiveEntranceGeometry = new THREE.ShapeGeometry(this.ellipse(0.16, 0.09), 16);
  // Two amber tones alternated band-to-band for the woven-straw texture.
  private readonly hiveBandMaterials = [
    this.glowMaterial(0xf0b955, 0xffce7a, 0.5),
    this.glowMaterial(0xc07f22, 0xe0a13f, 0.4),
  ];
  private readonly hiveKnobMaterial = this.glowMaterial(0xd5901f, 0xf3ba4d, 0.6);

  private readonly queenGeometry = new THREE.SphereGeometry(1, 16, 16);
  private readonly queenMaterial = this.glowMaterial(0xe0a12b, 0xffe08a, 0.7);
  private hive?: THREE.Group;
  private queen?: THREE.Mesh;
  // World-space centre and unscaled (wax = 0) radius the hive was last built
  // for; wax growth multiplies this radius fresh every snapshot (see
  // updateHiveScale) rather than being baked into the mesh once.
  private hiveCenter = { x: 0, y: 0 };
  private hiveBaseRadius = 0;

  // Wax-driven hive growth: an easing approach toward HIVE_MAX_WAX_GROWTH as
  // waxGrams climbs, not a linear scale, so a freshly-founded colony (0 g)
  // still reads as a recognisable hive and the mound doesn't balloon without
  // bound over a long-running session. The half-life is small because a
  // colony only ever banks gram fractions in a session (workers secrete
  // scales at WAX_SCALES_PER_SECOND — see world.rs) — a linear or
  // large-denominator curve would look static.
  private static readonly HIVE_MAX_WAX_GROWTH = 0.6;
  private static readonly HIVE_WAX_GROWTH_HALF_LIFE = 0.02;

  // A luminous ring laid over the followed bee so the target reads at a glance.
  // Unlit (MeshBasicMaterial) so it stays bright regardless of scene lighting,
  // and depth-write off so it haloes the bee without occluding it. One shared
  // mesh, repositioned/scaled onto the target each render (see updateFollow).
  private readonly highlightGeometry = new THREE.RingGeometry(10, 12.5, 36);
  private readonly highlightMaterial = new THREE.MeshBasicMaterial({
    color: 0xfff1c0,
    transparent: true,
    opacity: 0.92,
    side: THREE.DoubleSide,
    depthWrite: false,
  });
  private highlight?: THREE.Mesh;

  // Live scene objects keyed by stable entity id, reconciled each snapshot via
  // the shared {@link reconcileEntities} skeleton. Bees are multi-part `Group`s
  // (body + markings + wings); flowers are single meshes — both are just
  // `Object3D`s to the reconciler, so new entity kinds drop in the same way.
  private readonly beeObjects = new Map<number, THREE.Object3D>();
  private readonly flowerObjects = new Map<number, THREE.Object3D>();

  /** World bounds the camera/landmarks are currently sized for. */
  private worldWidth = 0;
  private worldHeight = 0;

  /** Camera zoom factor driven by the wheel and dock zoom stepper. */
  private zoom = 1;
  private static readonly MIN_ZOOM = 0.2;
  private static readonly MAX_ZOOM = 8;
  private static readonly ZOOM_STEP = 1.1;

  // Interpolation state: the latest snapshot is the *target*; bees glide from
  // where the previous snapshot placed them (prevBees, by id) toward it over
  // the measured publish interval. See the class doc and {@link renderScene}.
  private currSnapshot: WorldSnapshot | null = null;
  private readonly prevBees = new Map<number, BeeSnapshot>();
  /** `performance.now()` when {@link currSnapshot} arrived. */
  private currArrivedAt = 0;
  /** Smoothed gap between snapshot arrivals — the interpolation window. */
  private snapshotIntervalMs = 100;
  private animationFrame?: number;
  /** Reused output of the per-bee lerp, so interpolation allocates nothing. */
  private readonly lerpedPosition: Vec3 = { x: 0, y: 0, z: 0 };
  /**
   * An arrival gap beyond this is a stalled stream — a reconnect after an
   * outage, a long-hidden tab — not a publish interval. The tick counter alone
   * can't catch it (the sim kept running, so the tick *grew*), but gliding
   * bees from positions that old is the same unrelated-worlds sweep the
   * tick-regression guard exists to prevent. Comfortably above the 1000 ms
   * gap clamp, so a merely slow frame never trips it.
   */
  private static readonly STALE_GAP_MS = 2000;
  /**
   * Set when a new snapshot may have changed entity *membership*; cleared once
   * {@link renderScene} has reconciled objects against it. Between arrivals
   * the interpolation frames skip the reconcile diff entirely — membership
   * can't change mid-window, and the diff allocates (see reconcileEntities).
   */
  private membershipDirty = true;

  // Viewport reporting (see {@link reportViewport}).
  private static readonly VIEWPORT_MARGIN = 0.25;
  private static readonly VIEWPORT_HYSTERESIS = 0.1;
  private lastViewport?: ViewportRect;

  constructor() {
    // Build the three.js scene once the canvas element is in the DOM. This runs
    // only in the browser, so WebGL is never touched during SSR/prerender.
    afterNextRender(() => this.init());

    // Adopt every new snapshot as the interpolation target (rendering is a
    // no-op until the renderer is initialised, but the state still tracks).
    effect(() => {
      const snapshot = this.sim.snapshot();
      if (snapshot) {
        this.onSnapshot(snapshot);
      }
    });
  }

  /**
   * Adopt a freshly arrived snapshot: the previous one's bees become the lerp
   * origins, the arrival gap feeds the smoothed interpolation window, and the
   * animation loop is (re)armed. Two kinds of discontinuity skip interpolation
   * for the frame rather than sweeping unrelated worlds into each other: a
   * tick that went *backwards* (reshuffle, reconnect to a reset sim) and a
   * previous frame that is simply too *old* ({@link STALE_GAP_MS} — reconnect
   * after an outage, a tab hidden long enough for the stream to lapse).
   */
  private onSnapshot(snapshot: WorldSnapshot): void {
    const now = performance.now();
    const prev = this.currSnapshot;
    const gap = now - this.currArrivedAt;
    this.prevBees.clear();
    if (prev && snapshot.tick >= prev.tick && gap <= WorldCanvas.STALE_GAP_MS) {
      // Clamp the gap: an off-cadence publish (a click landing mid-interval)
      // shouldn't whipsaw the window.
      const clamped = Math.min(1000, Math.max(1, gap));
      this.snapshotIntervalMs = 0.7 * this.snapshotIntervalMs + 0.3 * clamped;
      for (const bee of prev.bees) {
        this.prevBees.set(bee.id, bee);
      }
    }
    this.currSnapshot = snapshot;
    this.currArrivedAt = now;
    this.membershipDirty = true;
    this.renderScene(0);
    this.startInterpolation();
  }

  /**
   * Run the interpolation loop until it catches up with the latest snapshot
   * (alpha 1), then stop — a paused simulation costs no frames; the next
   * arrival re-arms it. Outside the Angular zone: the loop only writes to the
   * three.js scene, so change detection has nothing to see 60× a second.
   */
  private startInterpolation(): void {
    if (this.animationFrame !== undefined || !this.renderer) {
      return;
    }
    this.zone.runOutsideAngular(() => {
      const step = () => {
        const alpha = (performance.now() - this.currArrivedAt) / this.snapshotIntervalMs;
        if (alpha < 1) {
          this.renderScene(alpha);
          this.animationFrame = requestAnimationFrame(step);
        } else {
          this.renderScene(1);
          this.animationFrame = undefined;
        }
      };
      this.animationFrame = requestAnimationFrame(step);
    });
  }

  /** A flat ellipse `Shape` centred at the origin, for the bee's body parts. */
  private ellipse(rx: number, ry: number): THREE.Shape {
    const shape = new THREE.Shape();
    shape.absellipse(0, 0, rx, ry, 0, Math.PI * 2, false, 0);
    return shape;
  }

  /** A warm body colour with an emissive lift, for the soft dot "glow". */
  private glowMaterial(
    color: number,
    emissive: number,
    intensity: number,
  ): THREE.MeshStandardMaterial {
    return new THREE.MeshStandardMaterial({
      color,
      emissive,
      emissiveIntensity: intensity,
      roughness: 0.55,
    });
  }

  /** Step the camera zoom in (closer). Called by the dock `+` button. */
  zoomIn(): void {
    this.applyZoom(WorldCanvas.ZOOM_STEP);
  }

  /** Step the camera zoom out (further). Called by the dock `−` button. */
  zoomOut(): void {
    this.applyZoom(1 / WorldCanvas.ZOOM_STEP);
  }

  private applyZoom(factor: number): void {
    this.zoom = Math.min(WorldCanvas.MAX_ZOOM, Math.max(WorldCanvas.MIN_ZOOM, this.zoom * factor));
    this.zoomPercent.set(Math.round(this.zoom * 100));
    this.updateCamera();
    this.renderCurrent();
  }

  private init(): void {
    const canvas = this.canvas().nativeElement;

    let renderer: THREE.WebGLRenderer;
    try {
      // Transparent so the CSS gradient stage reads as the colony floor.
      renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true });
    } catch {
      // No WebGL context available (e.g. headless test env) — bail gracefully.
      return;
    }
    renderer.setClearColor(0x000000, 0);
    this.renderer = renderer;

    const scene = new THREE.Scene();
    this.scene = scene;

    // Top-down orthographic camera looking straight down -z onto the x/y plane.
    const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 4000);
    camera.position.z = 2000;
    this.camera = camera;

    scene.add(new THREE.AmbientLight(0xfff4dd, 0.95));
    const sun = new THREE.DirectionalLight(0xffffff, 0.5);
    sun.position.set(0.4, 0.6, 1);
    scene.add(sun);

    // Pick bees on click via a ray cast through the orthographic frustum.
    this.raycaster = new THREE.Raycaster();
    this.highlight = new THREE.Mesh(this.highlightGeometry, this.highlightMaterial);
    this.highlight.visible = false;
    scene.add(this.highlight);

    this.resizeObserver = new ResizeObserver(() => this.handleResize());
    this.resizeObserver.observe(canvas);
    canvas.addEventListener('wheel', this.onWheel, { passive: false });
    canvas.addEventListener('click', this.onClick);

    // Size the renderer to the canvas before the first draw, then render the
    // latest snapshot immediately if one already arrived.
    this.handleResize();
    if (this.currSnapshot) {
      this.renderScene(1);
    }
  }

  /**
   * Draw the world at interpolation fraction `alpha` between the previous and
   * the latest snapshot: `0` is where the previous frame left every bee, `1`
   * is the latest snapshot's exact positions. Membership, states, resources,
   * and landmarks always come from the latest snapshot — only positions glide.
   */
  private renderScene(alpha: number): void {
    const { renderer, scene, camera } = this;
    const snapshot = this.currSnapshot;
    if (!renderer || !scene || !camera || !snapshot) {
      return;
    }

    // Resize landmarks/camera if the world bounds changed (e.g. after reset).
    // The canvas fills its stage; `updateCamera` letterboxes the world within
    // it to preserve the world's aspect ratio.
    const { width, height } = snapshot.bounds;
    if (width !== this.worldWidth || height !== this.worldHeight) {
      this.worldWidth = width;
      this.worldHeight = height;
      this.rebuildLandmarks();
      this.updateCamera();
    }
    // Every snapshot, not just on a bounds change: wax accrues tick by tick.
    this.updateHiveScale(snapshot.waxGrams);

    // The membership diff (create/remove + its Set allocation) runs only when
    // a snapshot could actually have changed membership — once per arrival,
    // not on every interpolation frame. In-between frames touch nothing but
    // the positions of already-reconciled objects, keeping the ~60 Hz loop
    // allocation-free per the class convention.
    if (this.membershipDirty) {
      this.reconcileEntities(
        this.beeObjects,
        snapshot.bees,
        (bee) => this.createBee(this.beeMaterialFor(bee), bee.beeClass),
        (object, bee) => this.updateBee(object as THREE.Group, bee, alpha),
      );
      this.reconcileEntities(
        this.flowerObjects,
        snapshot.resources,
        () => new THREE.Mesh(this.flowerGeometry, this.flowerMaterial),
        (object, flower) => this.placeAt(object, flower.position),
      );
      this.membershipDirty = false;
    } else {
      // Flowers are static within a snapshot, so only bees need refreshing.
      for (const bee of snapshot.bees) {
        const object = this.beeObjects.get(bee.id);
        if (object) {
          this.updateBee(object as THREE.Group, bee, alpha);
        }
      }
    }

    // Lock the camera/highlight onto the followed bee at its fresh position.
    this.updateFollow();

    renderer.render(scene, camera);
  }

  /**
   * The shared reconcile skeleton: make `objects` mirror `entities` exactly,
   * keyed by stable id. New entities are built with `create` and added to the
   * scene; every surviving entity is refreshed with `update`; objects whose
   * entity vanished from the snapshot are removed. Each entity kind supplies its
   * own `create`/`update` (a single mesh, a composite `Group`, …), so adding a
   * kind is one more call — no new bespoke loop.
   */
  private reconcileEntities<T extends { id: number }>(
    objects: Map<number, THREE.Object3D>,
    entities: ReadonlyArray<T>,
    create: (entity: T) => THREE.Object3D,
    update: (object: THREE.Object3D, entity: T) => void,
  ): void {
    const seen = new Set<number>();
    for (const entity of entities) {
      seen.add(entity.id);
      let object = objects.get(entity.id);
      if (!object) {
        object = create(entity);
        objects.set(entity.id, object);
        this.scene!.add(object);
      }
      update(object, entity);
    }

    // Drop objects whose entity disappeared from the snapshot.
    for (const [id, object] of objects) {
      if (!seen.has(id)) {
        this.scene!.remove(object);
        objects.delete(id);
      }
    }
  }

  /**
   * Position an object on the x/y plane, flipping world `y` (`height - y`) so
   * the world origin sits top-left on screen. Shared by every entity kind.
   */
  private placeAt(object: THREE.Object3D, position: Vec3): void {
    object.position.set(position.x, this.worldHeight - position.y, position.z);
  }

  /** The body material for a bee's current behavior state (gold/sage/dim). */
  private beeMaterialFor(bee: BeeSnapshot): THREE.Material {
    return this.beeMaterials[bee.state] ?? this.beeMaterials.wandering;
  }

  /**
   * Refresh a live bee: recolour the body by state, place it (world-`y` flip)
   * at the interpolated position — `alpha` of the way from where the previous
   * snapshot had it to where the latest one does — and turn it to face its
   * velocity. A bee absent from the previous snapshot (fresh spawn, or newly
   * scrolled into the viewport) has no origin to glide from and sits at its
   * latest position outright. On screen `y` is flipped, so the heading angle
   * negates `vy`; a near-stationary bee keeps its previous facing.
   */
  private updateBee(group: THREE.Group, bee: BeeSnapshot, alpha: number): void {
    (group.userData['body'] as THREE.Mesh).material = this.beeMaterialFor(bee);
    // Stamp the id so a click ray-hit on any body part resolves back to the bee.
    group.userData['beeId'] = bee.id;

    const prev = alpha < 1 ? this.prevBees.get(bee.id) : undefined;
    if (prev) {
      // Into the reused scratch vector — no allocation per bee per frame.
      const lerped = this.lerpedPosition;
      lerped.x = prev.position.x + (bee.position.x - prev.position.x) * alpha;
      lerped.y = prev.position.y + (bee.position.y - prev.position.y) * alpha;
      lerped.z = prev.position.z + (bee.position.z - prev.position.z) * alpha;
      this.placeAt(group, lerped);
    } else {
      this.placeAt(group, bee.position);
    }

    const vx = bee.velocity?.x ?? 0;
    const vy = bee.velocity?.y ?? 0;
    if (vx * vx + vy * vy > 1e-6) {
      group.rotation.z = Math.atan2(-vy, vx);
    }
  }

  /**
   * Assemble one bee from the shared part geometries: an elongated body (the
   * state-tinted glow), a dark head at the heading tip (`+x`), two abdomen
   * stripes tapering toward the tail, and a translucent wing on each side. The
   * `body` mesh is stashed in `userData` so {@link updateBee} can recolour it by
   * state without rebuilding the group. The bee is modelled pointing along `+x`
   * and rotated about `z` to face its velocity (see {@link updateBee}).
   */
  private createBee(material: THREE.Material, beeClass: BeeClass): THREE.Group {
    const bee = new THREE.Group();
    // Caste never changes, so scale once here rather than every snapshot.
    bee.scale.setScalar(this.classScale[beeClass] ?? 1);

    const body = new THREE.Mesh(this.beeBodyGeometry, material);
    bee.add(body);
    bee.userData['body'] = body;

    // Dark head poking out past the front of the body.
    const head = new THREE.Mesh(this.beeHeadGeometry, this.beeMarkingMaterial);
    head.position.set(7.5, 0, 0.05);
    bee.add(head);

    // Two abdomen stripes on the rear half, narrowed to follow the body taper
    // so they stay inside the silhouette. Lifted slightly in z to sit on top.
    for (const [x, widthScale] of [
      [-1, 1],
      [-4.5, 0.78],
    ] as const) {
      const stripe = new THREE.Mesh(this.beeStripeGeometry, this.beeMarkingMaterial);
      stripe.position.set(x, 0, 0.05);
      stripe.scale.set(1, widthScale, 1);
      bee.add(stripe);
    }

    // A wing on each flank, fanned slightly back and floated above the body.
    for (const side of [1, -1]) {
      const wing = new THREE.Mesh(this.beeWingGeometry, this.beeWingMaterial);
      wing.position.set(2, side * 4.5, 0.3);
      wing.rotation.z = side * 0.35;
      bee.add(wing);
    }

    return bee;
  }

  /**
   * Place the hive at the world centre and the queen just above it, sized to
   * the world bounds. They have no wire representation yet, so their position
   * and base size are derived purely from the bounds — one hive, one queen,
   * matching the rail's counts. The hive's actual on-screen radius is
   * re-derived from this base every snapshot (see {@link updateHiveScale}),
   * since it also grows with banked wax.
   */
  private rebuildLandmarks(): void {
    const cx = this.worldWidth / 2;
    const cy = this.worldHeight / 2;
    const unit = Math.min(this.worldWidth, this.worldHeight);
    this.hiveCenter = { x: cx, y: cy };
    this.hiveBaseRadius = unit * 0.06;
    const queenRadius = unit * 0.012;

    if (!this.hive) {
      this.hive = this.createHive();
      this.scene!.add(this.hive);
    }
    this.hive.position.set(cx, cy, 0.5);

    if (!this.queen) {
      this.queen = new THREE.Mesh(this.queenGeometry, this.queenMaterial);
      this.scene!.add(this.queen);
    }
    this.queen.scale.setScalar(queenRadius);
  }

  /**
   * Assemble the hive landmark from its tier bands, cap knob, and entrance
   * notch (see {@link HIVE_TIERS}). Built once and reused — only its overall
   * scale and the queen's resting height change afterward.
   */
  private createHive(): THREE.Group {
    const hive = new THREE.Group();

    WorldCanvas.HIVE_TIERS.forEach(({ y }, i) => {
      const band = new THREE.Mesh(
        this.hiveTierGeometries[i],
        this.hiveBandMaterials[i % this.hiveBandMaterials.length],
      );
      // Tiny z steps so each band draws cleanly over the one beneath it.
      band.position.set(0, y, i * 0.01);
      hive.add(band);
    });

    const topZ = WorldCanvas.HIVE_TIERS.length * 0.01;
    const knob = new THREE.Mesh(this.hiveKnobGeometry, this.hiveKnobMaterial);
    knob.position.set(0, 0.54, topZ);
    hive.add(knob);

    // Dark entrance notch near the base, reusing the bee markings' near-black
    // tone rather than a bespoke material.
    const entrance = new THREE.Mesh(this.hiveEntranceGeometry, this.beeMarkingMaterial);
    entrance.position.set(0, -0.8, topZ + 0.01);
    hive.add(entrance);

    return hive;
  }

  /**
   * Rescale the hive by banked wax (see {@link HIVE_MAX_WAX_GROWTH}) and keep
   * the queen resting just above its now-grown crown.
   */
  private updateHiveScale(waxGrams: number): void {
    if (!this.hive || !this.queen) {
      return;
    }
    const wax = Math.max(0, waxGrams);
    const growth =
      1 + WorldCanvas.HIVE_MAX_WAX_GROWTH * (wax / (wax + WorldCanvas.HIVE_WAX_GROWTH_HALF_LIFE));
    const radius = this.hiveBaseRadius * growth;
    this.hive.scale.setScalar(radius);
    // Above the hive centre on screen (smaller screen-y ⇒ larger world-y).
    this.queen.position.set(this.hiveCenter.x, this.hiveCenter.y + radius * 0.6, 1);
  }

  /** Frame the whole world, letterboxing to preserve aspect ratio. */
  private updateCamera(): void {
    const { camera, renderer } = this;
    if (!camera || !renderer || !this.worldWidth || !this.worldHeight) {
      return;
    }
    const size = renderer.getSize(new THREE.Vector2());
    const viewAspect = size.height > 0 ? size.width / size.height : 1;
    const worldAspect = this.worldWidth / this.worldHeight;

    // Half-extents that just contain the world for the current viewport aspect.
    let halfW: number;
    let halfH: number;
    if (viewAspect > worldAspect) {
      halfH = this.worldHeight / 2;
      halfW = halfH * viewAspect;
    } else {
      halfW = this.worldWidth / 2;
      halfH = halfW / viewAspect;
    }

    camera.left = -halfW;
    camera.right = halfW;
    camera.top = halfH;
    camera.bottom = -halfH;
    camera.zoom = this.zoom;
    // Centre on the focus point — the followed bee, or the world centre when the
    // follow-cam is free. With a symmetric frustum the focus point sits dead
    // centre-frame, which is exactly what the follow-cam wants.
    const focus = this.focusPoint();
    camera.position.x = focus.x;
    camera.position.y = focus.y;
    camera.updateProjectionMatrix();
    this.reportViewport();
  }

  /**
   * Tell the simulation source what world rect the camera can see, so the
   * server can cull off-screen bees from this client's stream (the wasm source
   * no-ops it). Runs on every camera change — zoom, resize, follow-cam glide —
   * so it is deliberately cheap and quiet: the rect is padded by
   * {@link VIEWPORT_MARGIN} (bees enter the screen before they enter the
   * stream, and small drifts stay inside the last report), and a re-send only
   * happens when an edge moves by more than {@link VIEWPORT_HYSTERESIS} of the
   * view. The world-`y` flip {@link placeAt} bakes into scene coordinates is
   * inverted here, so the rect is in true world space like the wire expects.
   */
  private reportViewport(): void {
    const camera = this.camera;
    if (!camera || !this.worldWidth || !this.worldHeight) {
      return;
    }
    const pad = 1 + WorldCanvas.VIEWPORT_MARGIN;
    const halfW = (((camera.right - camera.left) / 2) * pad) / camera.zoom;
    const halfH = (((camera.top - camera.bottom) / 2) * pad) / camera.zoom;
    const cx = camera.position.x;
    const worldCy = this.worldHeight - camera.position.y;
    const rect: ViewportRect = {
      minX: cx - halfW,
      maxX: cx + halfW,
      minY: worldCy - halfH,
      maxY: worldCy + halfH,
    };

    const last = this.lastViewport;
    if (last) {
      const tolX = (rect.maxX - rect.minX) * WorldCanvas.VIEWPORT_HYSTERESIS;
      const tolY = (rect.maxY - rect.minY) * WorldCanvas.VIEWPORT_HYSTERESIS;
      if (
        Math.abs(rect.minX - last.minX) < tolX &&
        Math.abs(rect.maxX - last.maxX) < tolX &&
        Math.abs(rect.minY - last.minY) < tolY &&
        Math.abs(rect.maxY - last.maxY) < tolY
      ) {
        return;
      }
    }
    this.lastViewport = rect;
    this.sim.setViewport(rect);
  }

  /**
   * Screen-space point to keep centre-frame: the followed bee's current
   * position, or the world centre when nothing is followed (or the target has
   * left the snapshot). Bee object positions are already in screen space (the
   * world-`y` flip is baked in by {@link placeAt}), so they map straight through.
   */
  private focusPoint(): { x: number; y: number } {
    const id = this.followedBeeId();
    if (id !== null) {
      const target = this.beeObjects.get(id);
      if (target) {
        return { x: target.position.x, y: target.position.y };
      }
    }
    return { x: this.worldWidth / 2, y: this.worldHeight / 2 };
  }

  /**
   * Keep the follow-cam glued to its target each render: move the highlight ring
   * onto the bee and re-centre the camera on it. If the followed bee has left the
   * snapshot (died, or reset away), release the follow so the camera frees up.
   */
  private updateFollow(): void {
    const id = this.followedBeeId();
    const target = id === null ? undefined : this.beeObjects.get(id);

    if (id !== null && !target) {
      this.clearFollow();
      return;
    }

    if (this.highlight) {
      if (target) {
        this.highlight.visible = true;
        // Match the bee's per-caste scale so the ring hugs queen and worker alike.
        this.highlight.scale.setScalar(target.scale.x);
        this.highlight.position.set(target.position.x, target.position.y, 0.4);
      } else {
        this.highlight.visible = false;
      }
    }

    if (target) {
      this.updateCamera();
    }
  }

  /**
   * Release the follow-cam: free the camera, hide the highlight, and re-frame the
   * whole world. Called by the dock's clear button and when a click lands on
   * empty space. No-op when already free so it stays idempotent.
   */
  clearFollow(): void {
    if (this.followedBeeId() === null) {
      return;
    }
    this.followedBeeId.set(null);
    if (this.highlight) {
      this.highlight.visible = false;
    }
    this.updateCamera();
    this.renderCurrent();
  }

  /**
   * Select the active pointer tool. The dashboard dock calls this to switch
   * between following bees and placing new entities; switching away from
   * `follow` keeps any current follow lock until the next click resolves it.
   */
  setTool(tool: PointerTool): void {
    this.tool.set(tool);
    // A crosshair while placing signals the click will drop something.
    this.canvas().nativeElement.style.cursor = tool === 'follow' ? 'pointer' : 'crosshair';
  }

  /**
   * Handle a click. In a placement tool, drop a bee/nectar at the clicked world
   * point via the simulation service. Otherwise resolve to a bee and follow it
   * (or release on empty space) by casting a ray through the orthographic
   * frustum and walking the nearest hit up to its owning bee `Group`.
   */
  private handleClick(event: MouseEvent): void {
    const { camera, raycaster } = this;
    if (!camera || !raycaster) {
      return;
    }
    const ndc = this.pointerNdc(event);
    if (!ndc) {
      return;
    }
    raycaster.setFromCamera(ndc, camera);

    const tool = this.tool();
    if (tool !== 'follow') {
      const point = this.worldPointFromRay();
      if (point) {
        if (tool === 'spawnBee') {
          this.sim.spawnBee(point.x, point.y);
        } else {
          this.sim.addNectar(point.x, point.y);
        }
      }
      return;
    }

    const hit = raycaster.intersectObjects([...this.beeObjects.values()], true)[0];
    // A hit follows that bee; a miss (empty space) releases the follow-cam.
    this.followedBeeId.set(hit ? this.beeIdFor(hit.object) : null);
  }

  /**
   * Normalised device coordinates for a pointer event, or `null` if the canvas
   * has no size yet. The screen→NDC mapping the raycaster expects.
   */
  private pointerNdc(event: MouseEvent): THREE.Vector2 | null {
    const rect = this.canvas().nativeElement.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) {
      return null;
    }
    return new THREE.Vector2(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1,
    );
  }

  /**
   * The world-space point under the current ray, found by intersecting the
   * `z = 0` floor plane and inverting the screen-`y` flip {@link placeAt} bakes
   * in (scene-y = `worldHeight − world-y`). Clamped into bounds so a click on
   * the letterbox margin still lands on the floor. `null` if the ray misses.
   */
  private worldPointFromRay(): { x: number; y: number } | null {
    const point = new THREE.Vector3();
    if (!this.raycaster!.ray.intersectPlane(this.groundPlane, point)) {
      return null;
    }
    return {
      x: Math.min(this.worldWidth, Math.max(0, point.x)),
      y: Math.min(this.worldHeight, Math.max(0, this.worldHeight - point.y)),
    };
  }

  /** Walk an object up its ancestry to the owning bee `Group`'s stamped id. */
  private beeIdFor(object: THREE.Object3D): number | null {
    let node: THREE.Object3D | null = object;
    while (node) {
      const id = node.userData['beeId'];
      if (typeof id === 'number') {
        return id;
      }
      node = node.parent;
    }
    return null;
  }

  private handleResize(): void {
    const { renderer } = this;
    if (!renderer) {
      return;
    }
    const canvas = this.canvas().nativeElement;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    if (width === 0 || height === 0) {
      return;
    }
    renderer.setPixelRatio(window.devicePixelRatio);
    renderer.setSize(width, height, false);
    this.updateCamera();
    this.renderCurrent();
  }

  private handleWheel(event: WheelEvent): void {
    event.preventDefault();
    this.applyZoom(event.deltaY < 0 ? WorldCanvas.ZOOM_STEP : 1 / WorldCanvas.ZOOM_STEP);
  }

  /** Re-render the latest snapshot after a camera-only change (zoom/resize). */
  private renderCurrent(): void {
    const { renderer, scene, camera } = this;
    if (renderer && scene && camera) {
      renderer.render(scene, camera);
    }
  }

  ngOnDestroy(): void {
    if (this.animationFrame !== undefined) {
      cancelAnimationFrame(this.animationFrame);
    }
    const canvas = this.canvas().nativeElement;
    canvas.removeEventListener('wheel', this.onWheel);
    canvas.removeEventListener('click', this.onClick);
    this.resizeObserver?.disconnect();

    this.beeBodyGeometry.dispose();
    this.beeStripeGeometry.dispose();
    this.beeWingGeometry.dispose();
    this.beeHeadGeometry.dispose();
    for (const material of Object.values(this.beeMaterials)) {
      material.dispose();
    }
    this.beeMarkingMaterial.dispose();
    this.beeWingMaterial.dispose();
    this.flowerGeometry.dispose();
    this.flowerMaterial.dispose();
    for (const geometry of this.hiveTierGeometries) {
      geometry.dispose();
    }
    this.hiveKnobGeometry.dispose();
    this.hiveEntranceGeometry.dispose();
    for (const material of this.hiveBandMaterials) {
      material.dispose();
    }
    this.hiveKnobMaterial.dispose();
    this.queenGeometry.dispose();
    this.queenMaterial.dispose();
    this.highlightGeometry.dispose();
    this.highlightMaterial.dispose();
    this.renderer?.dispose();
  }
}
