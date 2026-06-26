# Colony Simulator

A live bee-colony simulation: a deterministic engine written in **Rust**,
rendered by an **Angular + three.js** frontend. The same engine runs two ways —
streamed from a server over WebSocket (local dev) or compiled to WebAssembly and
run in-browser (static GitHub Pages build).

## Layout

```
backend/                 Rust workspace (Cargo)
  colony-core/           Pure simulation: World, Bee, Engine, Vec3, snapshots.
                         No I/O, no async — the deterministic heart, fully unit-tested.
  colony-server/         Axum server: runs the engine in a Tokio task, streams
                         WorldSnapshot over /ws, accepts /api/control (start/pause/reset).
  colony-wasm/           wasm-bindgen wrapper exposing the engine to JS (WasmEngine).
frontend/                Angular 20 app (standalone components, signals)
  src/app/
    models.ts            TS wire types — MUST mirror colony-core/src/snapshot.rs.
    simulation.service.ts Abstract DI token + contract for a simulation source.
    websocket-simulation.ts / wasm-simulation.ts  The two implementations.
    app.*                The "Hearth" dashboard (header, world, stats rail).
    world-canvas.ts      three.js renderer for the world.
```

## Architecture notes

- **One engine, two transports.** Components depend only on the abstract
  `SimulationService` (snapshot / connected / running signals; start / pause /
  reset / setSpeed). `app.config.ts` picks the implementation at build time via
  `environment.useWasm`: WebSocket in dev, WASM in production. When you add a
  capability, add it to the abstract class and implement it in **both**
  services. A transport may legitimately no-op a capability it can't express
  (the abstract `setSpeed` defaults to a no-op for that reason), though both
  transports do honour speed today — wasm re-arms its stepping loop and the
  server forwards a `set_speed` control command to its tick loop.
- **The wire format is a contract.** `frontend/src/app/models.ts` is a
  hand-maintained mirror of `backend/colony-core/src/snapshot.rs` (serde
  `snake_case`). Change one, change the other. Fields the engine doesn't emit
  yet (bee `energy`, `honeyStored`, the `foraging`/`resting` states) are typed
  as optional/forward-looking on the TS side so the UI lights up automatically
  once the backend reports them — don't fake values for them in the UI.
- **Rendering is snapshot-driven, not loop-driven.** `world-canvas` redraws only
  when a new snapshot arrives, on zoom, or on resize — there is no
  `requestAnimationFrame` loop. Meshes are reconciled by stable entity `id`;
  geometry/materials are shared singletons created once and disposed in
  `ngOnDestroy`. Keep it that way (don't allocate per frame).
- **`running` is service-owned.** Both transports start already running, so the
  UI binds to `sim.running()` for Start/Pause state rather than tracking its own
  guess. `reset` does not change running.
- **Cross-entity systems must stay deterministic.** Collision avoidance lives in
  `World::step` (not `Bee::step`, which stays a pure single-entity integrator and
  remains the *sole* authority that confines a bee to the bounds — steering only
  nudges velocity, so it can never eject a bee through a wall). It runs in two
  strict passes — compute every bee's separation force from immutable positions,
  *then* apply — so the result is independent of iteration order. Anything that
  sums floats across entities must pin its order (we walk pairs `i < j` and the
  grid sorts candidates ascending to match) or results drift. Two guards enforce
  this: `stepping_from_the_seed_is_deterministic` (same seed → bit-identical
  trajectory) and `grid_matches_naive` (the spatial grid must equal the naive
  all-pairs oracle bit-for-bit). Keep the naive reference around as the oracle
  whenever you optimize a broad phase. The engine has **no RNG**; if you ever add
  one, seed it explicitly and thread it through so determinism survives.
- All three.js setup runs in `afterNextRender` and is wrapped so it bails
  gracefully when there is no WebGL context (headless/SSR).

## Commands

Run frontend commands from `frontend/`, Rust commands from `backend/`.

```bash
# Backend
cargo test                       # core simulation tests
cargo run -p colony-server       # serve on http://localhost:8080

# Frontend (dev: talks to the server above via proxy.conf.json)
npm install
npm start                        # ng serve on http://localhost:4200
npm run build                    # production (wasm) build → dist/colony-simulator/browser
npm test                         # Karma + Jasmine unit tests
npm run build:pages              # wasm-pack + ng build for GitHub Pages
```

## Conventions

- **TypeScript / Angular:** standalone components only; `ChangeDetectionStrategy.OnPush`;
  prefer signals (`signal` / `computed` / `viewChild`) and `inject()` over
  constructor DI and decorators. Formatting is Prettier (single quotes, 100
  cols — see `frontend/package.json`). Derive rail/stat values with `computed`,
  not stored duplicate state.
- **Rust:** keep `colony-core` free of I/O and async so it stays deterministic
  and unit-testable; it's the shared dependency of both the server and the wasm
  crate.
- Match the surrounding code's comment density and naming; the existing files
  are heavily doc-commented — explain *why*, not *what*.

## Benchmarking & performance (colony-core)

`colony-core` has a criterion bench at `backend/colony-core/benches/step.rs`
timing `Engine::step` across colony sizes. Lessons worth keeping:

- **Measure before optimizing a hot path, and bracket behavior changes with a
  saved baseline.** `cargo bench -p colony-core --bench step -- --save-baseline
  <name>` before, `--baseline <name>` after. **Scope to `--bench step`** — a bare
  `cargo bench` also runs the lib's libtest harness, which rejects criterion's
  flags (`Unrecognized option: 'save-baseline'`).
- **Benchmark the scenario that actually runs.** The runtimes step one engine
  continuously, so the *warm* bench (`engine_step_warm`, steps in place) is the
  representative cost; the *cold* bench (`iter_batched` cloning a fresh engine)
  measures only the first step and, because each iteration starts from pristine
  state, never exercises buffer reuse. Optimizing against the wrong one misleads.
- **Profile, don't guess, what's expensive.** Per-tick allocation looked like the
  cost; it wasn't. At scale the broad phase was dominated by **SipHash** over the
  grid's integer cell keys (~`n × 27` lookups/tick). A tiny no-seed multiply-rotate
  `CellHasher` (cell keys are trusted internal integers — DoS resistance is moot,
  determinism is not) cut the step 60–76%. A custom `Hasher` is the lean,
  no-dependency way to escape SipHash here.
- **Density, not population, is the scaling limit.** The grid is O(n · local
  density). In the fixed-size world (bees still flat at `z = 0`), density rises
  with `n`, so it degrades toward O(n²). True large-swarm scaling means bounding
  density first — grow the world with population and/or use the live `z` axis —
  before chasing further constant factors (flat array grid, dropping the per-bee
  sort behind a relaxed determinism check, rayon over the read-only first pass).

## Testing gotcha (containers / CI)

Karma launches Chrome. Inside a root container the default `ChromeHeadless`
fails with a sandbox error and three.js logs a harmless "could not create a
WebGL context" (the renderer catches it and bails — tests still pass). Run with
a Chromium that has `--no-sandbox`, e.g.:

```bash
CHROME_BIN=/path/to/chromium-wrapper npx ng test --watch=false --browsers=ChromeHeadless
```

where the wrapper exec's the real binary with
`--no-sandbox --disable-gpu --disable-dev-shm-usage`. Don't commit a custom
`karma.conf` that overrides the `@angular/build:karma` builder's defaults — it
drops the Jasmine framework wiring ("describe is not defined").
