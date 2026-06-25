import {
  Component,
  ElementRef,
  OnDestroy,
  afterNextRender,
  effect,
  inject,
  viewChild,
  ChangeDetectionStrategy,
} from '@angular/core';
import * as THREE from 'three';
import { SimulationService } from './simulation.service';
import { WorldSnapshot } from './models';

/**
 * Renders the world with three.js: the bounds rectangle as a ground plane,
 * nectar resources as flat discs, and every bee as a small sphere.
 *
 * The view is a top-down orthographic camera looking straight down the world's
 * `z` (flight) axis, so the screen shows the x/y plane just like the previous
 * 2D renderer. World `y` grows downward on screen (origin top-left), matching
 * the old canvas orientation. `z` raises an entity toward the camera; it is `0`
 * for every entity today but is mapped through so depth/flight rendering works
 * for free once the simulation populates it.
 *
 * The scene is redrawn whenever a new snapshot arrives (~30 Hz), on mouse-wheel
 * zoom, and on resize. There is no continuous animation loop.
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
        /* World is 800x600 (4:3); the camera letterboxes if this differs. */
        aspect-ratio: 4 / 3;
        height: auto;
        background: #1b1b1f;
        border: 1px solid #3a3a40;
        border-radius: 8px;
      }
    `,
  ],
})
export class WorldCanvas implements OnDestroy {
  private readonly sim = inject(SimulationService);
  private readonly canvas =
    viewChild.required<ElementRef<HTMLCanvasElement>>('canvas');

  private readonly background = new THREE.Color('#1b1b1f');

  // three.js handles, created in afterNextRender (browser only).
  private renderer?: THREE.WebGLRenderer;
  private scene?: THREE.Scene;
  private camera?: THREE.OrthographicCamera;
  private resizeObserver?: ResizeObserver;
  private readonly onWheel = (event: WheelEvent) => this.handleWheel(event);

  // Shared geometry/materials — one of each, reused across every entity.
  private readonly beeGeometry = new THREE.SphereGeometry(7, 16, 16);
  private readonly beeMaterial = new THREE.MeshStandardMaterial({
    color: 0xffcc33,
    roughness: 0.5,
  });
  private readonly flowerGeometry = new THREE.CircleGeometry(9, 24);
  private readonly flowerMaterial = new THREE.MeshStandardMaterial({
    color: 0xff5fa2,
    roughness: 0.7,
  });

  // Live meshes keyed by stable entity id, reconciled each snapshot.
  private readonly beeMeshes = new Map<number, THREE.Mesh>();
  private readonly flowerMeshes = new Map<number, THREE.Mesh>();
  private ground?: THREE.Mesh;

  /** World bounds the camera/ground are currently sized for. */
  private worldWidth = 0;
  private worldHeight = 0;

  /** Camera zoom factor driven by the mouse wheel. */
  private zoom = 1;
  private static readonly MIN_ZOOM = 0.2;
  private static readonly MAX_ZOOM = 8;

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

  private init(): void {
    const canvas = this.canvas().nativeElement;

    let renderer: THREE.WebGLRenderer;
    try {
      renderer = new THREE.WebGLRenderer({ canvas, antialias: true });
    } catch {
      // No WebGL context available (e.g. headless test env) — bail gracefully.
      return;
    }
    renderer.setClearColor(this.background);
    this.renderer = renderer;

    const scene = new THREE.Scene();
    scene.background = this.background;
    this.scene = scene;

    // Top-down orthographic camera looking straight down -z onto the x/y plane.
    const camera = new THREE.OrthographicCamera(-1, 1, 1, -1, 0.1, 4000);
    camera.position.z = 2000;
    this.camera = camera;

    scene.add(new THREE.AmbientLight(0xffffff, 0.75));
    const sun = new THREE.DirectionalLight(0xffffff, 0.6);
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

    // Resize the ground/camera if the world bounds changed (e.g. after reset).
    const { width, height } = snapshot.bounds;
    if (width !== this.worldWidth || height !== this.worldHeight) {
      this.worldWidth = width;
      this.worldHeight = height;
      this.rebuildGround();
      this.updateCamera();
    }

    this.reconcile(
      this.beeMeshes,
      this.beeGeometry,
      this.beeMaterial,
      snapshot.bees,
    );
    this.reconcile(
      this.flowerMeshes,
      this.flowerGeometry,
      this.flowerMaterial,
      snapshot.resources,
    );

    renderer.render(scene, camera);
  }

  /**
   * Create/update/remove meshes so the map mirrors `entities` exactly, keyed by
   * stable id. World `y` is flipped (`height - y`) so the origin sits top-left
   * on screen, matching the previous 2D renderer.
   */
  private reconcile(
    meshes: Map<number, THREE.Mesh>,
    geometry: THREE.BufferGeometry,
    material: THREE.Material,
    entities: ReadonlyArray<{ id: number; position: { x: number; y: number; z: number } }>,
  ): void {
    const seen = new Set<number>();
    for (const entity of entities) {
      seen.add(entity.id);
      let mesh = meshes.get(entity.id);
      if (!mesh) {
        mesh = new THREE.Mesh(geometry, material);
        meshes.set(entity.id, mesh);
        this.scene!.add(mesh);
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

  private rebuildGround(): void {
    if (this.ground) {
      this.scene!.remove(this.ground);
      this.ground.geometry.dispose();
    }
    const geometry = new THREE.PlaneGeometry(this.worldWidth, this.worldHeight);
    const material = new THREE.MeshStandardMaterial({
      color: 0x222228,
      roughness: 1,
    });
    const ground = new THREE.Mesh(geometry, material);
    // Centre the plane on the world; sit it just behind the entities (z < 0).
    ground.position.set(this.worldWidth / 2, this.worldHeight / 2, -1);
    this.ground = ground;
    this.scene!.add(ground);
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
    const factor = event.deltaY < 0 ? 1.1 : 1 / 1.1;
    this.zoom = Math.min(
      WorldCanvas.MAX_ZOOM,
      Math.max(WorldCanvas.MIN_ZOOM, this.zoom * factor),
    );
    this.updateCamera();
    this.renderCurrent();
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
    this.beeMaterial.dispose();
    this.flowerGeometry.dispose();
    this.flowerMaterial.dispose();
    this.ground?.geometry.dispose();
    (this.ground?.material as THREE.Material | undefined)?.dispose();
    this.renderer?.dispose();
  }
}
