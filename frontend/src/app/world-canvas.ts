import {
  Component,
  ElementRef,
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
import { BeeClass, BeeSnapshot, BeeState, Vec3, WorldSnapshot } from './models';

/**
 * Renders the world with three.js in the "Hearth" honey-and-hive palette: bees
 * as small striped, winged shapes tinted by behavior state and turned to face
 * their heading, nectar resources as terracotta discs, and a central hive with
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
 * opaque ground plane. The scene is redrawn whenever a new snapshot arrives
 * (~30 Hz), on zoom, and on resize — there is no continuous animation loop.
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
  private readonly canvas =
    viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  /** Current camera zoom as a whole percentage, for the dock readout. */
  readonly zoomPercent = signal(100);

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
  private readonly hiveGeometry = new THREE.CircleGeometry(1, 6);
  private readonly hiveMaterial = this.glowMaterial(0xd5901f, 0xf3ba4d, 0.55);
  private readonly queenGeometry = new THREE.SphereGeometry(1, 16, 16);
  private readonly queenMaterial = this.glowMaterial(0xe0a12b, 0xffe08a, 0.7);
  private hive?: THREE.Mesh;
  private queen?: THREE.Mesh;

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

  constructor() {
    // Build the three.js scene once the canvas element is in the DOM. This runs
    // only in the browser, so WebGL is never touched during SSR/prerender.
    afterNextRender(() => this.init());

    // Redraw on every new snapshot (no-op until the renderer is initialised).
    effect(() => {
      const snapshot = this.sim.snapshot();
      if (snapshot) {
        this.renderSnapshot(snapshot);
      }
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
    this.zoom = Math.min(
      WorldCanvas.MAX_ZOOM,
      Math.max(WorldCanvas.MIN_ZOOM, this.zoom * factor),
    );
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
    const snapshot = this.sim.snapshot();
    if (snapshot) {
      this.renderSnapshot(snapshot);
    }
  }

  private renderSnapshot(snapshot: WorldSnapshot): void {
    const { renderer, scene, camera } = this;
    if (!renderer || !scene || !camera) {
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

    this.reconcileEntities(
      this.beeObjects,
      snapshot.bees,
      (bee) => this.createBee(this.beeMaterialFor(bee), bee.beeClass),
      (object, bee) => this.updateBee(object as THREE.Group, bee),
    );
    this.reconcileEntities(
      this.flowerObjects,
      snapshot.resources,
      () => new THREE.Mesh(this.flowerGeometry, this.flowerMaterial),
      (object, flower) => this.placeAt(object, flower.position),
    );

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
   * Refresh a live bee: recolour the body by state, place it (world-`y` flip),
   * and turn it to face its velocity. On screen `y` is flipped, so the heading
   * angle negates `vy`; a near-stationary bee keeps its previous facing.
   */
  private updateBee(group: THREE.Group, bee: BeeSnapshot): void {
    (group.userData['body'] as THREE.Mesh).material = this.beeMaterialFor(bee);
    // Stamp the id so a click ray-hit on any body part resolves back to the bee.
    group.userData['beeId'] = bee.id;
    this.placeAt(group, bee.position);

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
   * Place the hive at the world centre and the queen just above it, scaled to
   * the world size. They have no wire representation yet, so they are derived
   * purely from the bounds — one hive, one queen, matching the rail's counts.
   */
  private rebuildLandmarks(): void {
    const cx = this.worldWidth / 2;
    const cy = this.worldHeight / 2;
    const unit = Math.min(this.worldWidth, this.worldHeight);
    const hiveRadius = unit * 0.06;
    const queenRadius = unit * 0.012;

    if (!this.hive) {
      this.hive = new THREE.Mesh(this.hiveGeometry, this.hiveMaterial);
      this.scene!.add(this.hive);
    }
    this.hive.position.set(cx, cy, 0.5);
    this.hive.scale.setScalar(hiveRadius);

    if (!this.queen) {
      this.queen = new THREE.Mesh(this.queenGeometry, this.queenMaterial);
      this.scene!.add(this.queen);
    }
    // Above the hive centre on screen (smaller screen-y ⇒ larger world-y).
    this.queen.position.set(cx, cy + hiveRadius * 0.6, 1);
    this.queen.scale.setScalar(queenRadius);
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
   * Resolve a click to a bee and follow it (or release on empty space). Casts a
   * ray through the orthographic frustum at the pointer and walks the nearest hit
   * up to the owning bee `Group` via its stamped `beeId`.
   */
  private handleClick(event: MouseEvent): void {
    const { camera, raycaster } = this;
    if (!camera || !raycaster) {
      return;
    }
    const canvas = this.canvas().nativeElement;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) {
      return;
    }
    const ndc = new THREE.Vector2(
      ((event.clientX - rect.left) / rect.width) * 2 - 1,
      -((event.clientY - rect.top) / rect.height) * 2 + 1,
    );
    raycaster.setFromCamera(ndc, camera);
    const hit = raycaster.intersectObjects([...this.beeObjects.values()], true)[0];
    // A hit follows that bee; a miss (empty space) releases the follow-cam.
    this.followedBeeId.set(hit ? this.beeIdFor(hit.object) : null);
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
    this.hiveGeometry.dispose();
    this.hiveMaterial.dispose();
    this.queenGeometry.dispose();
    this.queenMaterial.dispose();
    this.highlightGeometry.dispose();
    this.highlightMaterial.dispose();
    this.renderer?.dispose();
  }
}
