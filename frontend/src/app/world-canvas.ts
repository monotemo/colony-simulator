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
import { BeeState, WorldSnapshot } from './models';

/**
 * Renders the world with three.js in the "Hearth" honey-and-hive palette: bees
 * as small glowing dots tinted by behavior state, nectar resources as
 * terracotta discs, and a central hive with its queen just above it.
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
      .world {
        display: block;
        width: 100%;
        height: 100%;
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

  // three.js handles, created in afterNextRender (browser only).
  private renderer?: THREE.WebGLRenderer;
  private scene?: THREE.Scene;
  private camera?: THREE.OrthographicCamera;
  private resizeObserver?: ResizeObserver;
  private readonly onWheel = (event: WheelEvent) => this.handleWheel(event);

  // Shared geometry/materials — one of each, reused across every entity.
  private readonly beeGeometry = new THREE.SphereGeometry(7, 16, 16);
  // Bee body tinted by behavior state (wandering gold / foraging sage / resting
  // dim). The emissive lift gives the "glow" the design calls for.
  private readonly beeMaterials: Record<BeeState, THREE.MeshStandardMaterial> = {
    wandering: this.glowMaterial(0xe2a12b, 0xf3b84a, 0.45),
    foraging: this.glowMaterial(0x7c8b5a, 0x9aae6e, 0.35),
    resting: this.glowMaterial(0xc99a38, 0xc99a38, 0.12),
  };
  private readonly flowerGeometry = new THREE.CircleGeometry(9, 24);
  private readonly flowerMaterial = this.glowMaterial(0xc9663c, 0xe68a5e, 0.25);

  // Central landmarks, derived from the world bounds (no wire data yet).
  private readonly hiveGeometry = new THREE.CircleGeometry(1, 6);
  private readonly hiveMaterial = this.glowMaterial(0xd5901f, 0xf3ba4d, 0.55);
  private readonly queenGeometry = new THREE.SphereGeometry(1, 16, 16);
  private readonly queenMaterial = this.glowMaterial(0xe0a12b, 0xffe08a, 0.7);
  private hive?: THREE.Mesh;
  private queen?: THREE.Mesh;

  // Live meshes keyed by stable entity id, reconciled each snapshot.
  private readonly beeMeshes = new Map<number, THREE.Mesh>();
  private readonly flowerMeshes = new Map<number, THREE.Mesh>();

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

    this.resizeObserver = new ResizeObserver(() => this.handleResize());
    this.resizeObserver.observe(canvas);
    canvas.addEventListener('wheel', this.onWheel, { passive: false });

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
    const { width, height } = snapshot.bounds;
    if (width !== this.worldWidth || height !== this.worldHeight) {
      this.worldWidth = width;
      this.worldHeight = height;
      // Match the element's box to the world so the camera never letterboxes.
      this.canvas().nativeElement.style.aspectRatio = `${width} / ${height}`;
      this.rebuildLandmarks();
      this.updateCamera();
    }

    this.reconcile(this.beeMeshes, this.beeGeometry, snapshot.bees, (bee) =>
      this.beeMaterials[bee.state] ?? this.beeMaterials.wandering,
    );
    this.reconcile(
      this.flowerMeshes,
      this.flowerGeometry,
      snapshot.resources,
      () => this.flowerMaterial,
    );

    renderer.render(scene, camera);
  }

  /**
   * Create/update/remove meshes so the map mirrors `entities` exactly, keyed by
   * stable id. World `y` is flipped (`height - y`) so the origin sits top-left
   * on screen. `material` resolves per entity so bees can be tinted by state.
   */
  private reconcile<T extends { id: number; position: { x: number; y: number; z: number } }>(
    meshes: Map<number, THREE.Mesh>,
    geometry: THREE.BufferGeometry,
    entities: ReadonlyArray<T>,
    material: (entity: T) => THREE.Material,
  ): void {
    const seen = new Set<number>();
    for (const entity of entities) {
      seen.add(entity.id);
      let mesh = meshes.get(entity.id);
      if (!mesh) {
        mesh = new THREE.Mesh(geometry, material(entity));
        meshes.set(entity.id, mesh);
        this.scene!.add(mesh);
      } else {
        mesh.material = material(entity);
      }
      const { x, y, z } = entity.position;
      mesh.position.set(x, this.worldHeight - y, z);
    }

    // Drop meshes whose entity disappeared from the snapshot.
    for (const [id, mesh] of meshes) {
      if (!seen.has(id)) {
        this.scene!.remove(mesh);
        meshes.delete(id);
      }
    }
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
    // Centre on the world; symmetric frustum keeps the world centred.
    camera.position.x = this.worldWidth / 2;
    camera.position.y = this.worldHeight / 2;
    camera.updateProjectionMatrix();
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
    this.resizeObserver?.disconnect();

    this.beeGeometry.dispose();
    for (const material of Object.values(this.beeMaterials)) {
      material.dispose();
    }
    this.flowerGeometry.dispose();
    this.flowerMaterial.dispose();
    this.hiveGeometry.dispose();
    this.hiveMaterial.dispose();
    this.queenGeometry.dispose();
    this.queenMaterial.dispose();
    this.renderer?.dispose();
  }
}
