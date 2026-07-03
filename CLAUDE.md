# Colony Simulator

A live, interactive bee-colony simulation: an engine written in **Rust**,
rendered by an **Angular + three.js** frontend. The same engine runs two ways —
streamed from a server over WebSocket (local dev) or compiled to WebAssembly and
run in-browser (the deployed Fly.io build, where `colony-server` serves the
static bundle and the engine runs in the browser).

The engine is **seeded, not deterministic across runs.** It once replayed one
fixed script bit-for-bit; that constraint was a scaffold for early profiling and
has been dropped. Randomness (a seeded RNG — see `colony-core/src/rng.rs`) now
varies the starting colony and adds per-tick wander, and the host seeds it from
entropy so every launch differs. What survives is reproducibility **given a
seed**: the same seed replays exactly, which is what debugging and the test
suite lean on. If you need a known run, build with `*_seed`/`from_seed` and the
fixed `DEFAULT_SEED`.

## Layout

```
backend/                 Rust workspace (Cargo)
  colony-core/           Pure simulation: World, Bee, Engine, Vec3, snapshots,
                         RNG, and the binary wire codec (wire.rs). No I/O, no
                         async — the seeded, reproducible-per-seed heart, fully
                         unit-tested. Entropy enters only at the host.
  colony-server/         Axum server: runs the engine in a Tokio task, streams
                         binary wire frames over /ws (encoded once per tick,
                         shared by every client), accepts /api/control
                         (start/pause/reset/set_speed/spawn_bee/add_nectar).
  colony-wasm/           wasm-bindgen wrapper exposing the engine to JS (WasmEngine).
frontend/                Angular 20 app (standalone components, signals)
  src/app/
    models.ts            Parsed snapshot types — MUST mirror colony-core/src/snapshot.rs.
    snapshot-codec.ts    Binary wire decoder — MUST mirror colony-core/src/wire.rs.
    simulation.service.ts Abstract DI token + contract for a simulation source.
    websocket-simulation.ts / wasm-simulation.ts  The two implementations.
    app.*                The "Hearth" dashboard (header, world, stats rail).
    world-canvas.ts      three.js renderer for the world.
```

## Architecture notes

- **One engine, two transports.** Components depend only on the abstract
  `SimulationService` (snapshot / connected / running signals; start / pause /
  reset / setSpeed / spawnBee / addNectar). `app.config.ts` picks the
  implementation at build time via `environment.useWasm`: WebSocket in dev, WASM
  in production. When you add a capability, add it to the abstract class and
  implement it in **both** services. A transport may legitimately no-op a
  capability it can't express (the abstract `setSpeed` defaults to a no-op for
  that reason), though every capability is honoured by both today — the wasm
  engine calls straight through while the server forwards a control command to
  its tick loop.
- **Interactivity is world perturbation over the control channel.** The user can
  reshuffle (reset → a *new* entropy-seeded colony, not a replay), drop a bee, or
  drop a nectar source by clicking the world. The canvas carries a pointer
  *tool* (`follow` | `spawnBee` | `addNectar`); a placement click maps screen →
  world via a `z = 0` ground-plane raycast and calls `sim.spawnBee/addNectar`,
  which the engine applies through `Engine::spawn_worker_at` / `add_nectar_at`.
  New world-perturbation commands follow this path end to end: a `Command`
  variant + handler in `colony-server/src/sim.rs`, a `WasmEngine` method, a
  `ControlCommand` shape in `models.ts`, and both transport implementations.
- **The wire format is a contract — two mirrored pairs now.**
  `frontend/src/app/models.ts` mirrors `backend/colony-core/src/snapshot.rs`
  (the *parsed* shapes the app consumes), and `frontend/src/app/snapshot-codec.ts`
  mirrors `backend/colony-core/src/wire.rs` (the binary encoding those shapes
  travel as). Change one side, change the other. The binary pair is pinned by a
  shared byte-for-byte fixture — `fixture_encodes_to_pinned_bytes` in `wire.rs`
  and the same hex strings in `snapshot-codec.spec.ts` — update both together.
  On the wire, immutable identity (ids, castes, resources, bounds) rides a rare
  versioned *roster* message; per-tick dynamics ride a compact f32 *motion*
  message keyed by roster index, so don't add an immutable field to motion or a
  per-tick field to roster. The server encodes each tick **once** and fans the
  shared bytes out to every socket; snapshot JSON survives only as a debug view.
  Fields the engine doesn't emit yet are typed as optional/forward-looking on
  the TS side so the UI lights up automatically once the backend reports them —
  don't fake values for them in the UI.
- **Rendering is snapshot-driven, not loop-driven.** `world-canvas` redraws only
  when a new snapshot arrives, on zoom, or on resize — there is no
  `requestAnimationFrame` loop. Meshes are reconciled by stable entity `id`;
  geometry/materials are shared singletons created once and disposed in
  `ngOnDestroy`. Keep it that way (don't allocate per frame).
- **`running` is service-owned.** Both transports start already running, so the
  UI binds to `sim.running()` for Start/Pause state rather than tracking its own
  guess. `reset` does not change running.
- **Reproducible per seed, and how that's guarded.** Collision avoidance lives in
  `World::step` (not `Bee::step`, which stays a pure single-entity integrator and
  remains the *sole* authority that confines a bee to the bounds — steering only
  nudges velocity, so neither avoidance, the foraging seek, nor the wander jitter
  can eject a bee through a wall). Separation runs in two strict passes — compute
  every bee's force from immutable positions, *then* apply — so it is independent
  of iteration order. Randomness flows through **one** seeded `Rng` on the
  `World`, consumed in a fixed (bee-index) order, so a given seed still replays
  bit-for-bit even though runs now differ. Anything that sums floats across
  entities, or draws from the RNG, must keep its order pinned (we walk pairs
  `i < j` and the grid sorts candidates ascending to match) or reproducibility
  drifts. Guards, now that bit-exact *cross-run* comparison is gone:
  - `same_seed_replays_bit_identically` — the seeded reproducibility contract;
    re-run after any change to `World::step`.
  - `different_seeds_produce_different_colonies` — proves the seed actually
    matters (catches a seed silently ignored, i.e. determinism creeping back).
  - `grid_matches_naive` — the spatial grid must equal the naive all-pairs oracle
    bit-for-bit (RNG-independent; keep the oracle around when optimizing the
    broad phase).
  - `invariants_hold_across_many_seeds` + `colony_spreads_across_states_on_most_seeds`
    — the **property/statistical** net that replaces bit-exact regression
    checking: over a sample of seeds the colony must never violate its physical
    constraints (in-bounds, energy/honey ∈ `[0,1]`) and must stay behaviorally
    lively. Prefer adding *invariants over many seeds* to pinning a magic
    trajectory when you test new behavior.
  If you add another source of randomness, route it through the world `Rng` and
  seed it explicitly — do **not** reach for `SystemTime`, `getrandom`, or a
  thread RNG inside `colony-core`; entropy belongs at the host (server/wasm).
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
npm run build:static             # wasm-pack + ng build (the bundle the Fly server serves)
```

## Conventions

- **TypeScript / Angular:** standalone components only; `ChangeDetectionStrategy.OnPush`;
  prefer signals (`signal` / `computed` / `viewChild`) and `inject()` over
  constructor DI and decorators. Formatting is Prettier (single quotes, 100
  cols — see `frontend/package.json`). Derive rail/stat values with `computed`,
  not stored duplicate state.
- **Rust:** keep `colony-core` free of I/O, async, and entropy so it stays
  reproducible-per-seed and unit-testable; it's the shared dependency of both the
  server and the wasm crate. Randomness goes through the seeded `Rng`; the seed
  itself comes from the host.
- Match the surrounding code's comment density and naming; the existing files
  are heavily doc-commented — explain *why*, not *what*.

## Benchmarking & performance (colony-core)

`colony-core` has a criterion bench at `backend/colony-core/benches/step.rs`
timing `Engine::step` across colony sizes. Lessons worth keeping:

- **Bench against the fixed seed so measurements stay comparable.** The engine is
  non-deterministic in production, but the benches build with
  `World::seeded_with_count` (the fixed `DEFAULT_SEED`), so the swarm layout —
  and therefore local density, the thing that drives the broad-phase cost — is
  identical run to run. Keep it that way: a bench seeded from entropy would make
  baselines incomparable. The wander jitter adds a couple of cheap RNG draws per
  in-flight bee per tick; it's in the warm path now, so a baseline captured
  before a step change already includes it.
- **Behavior regressions are caught statistically, not by a golden trajectory.**
  Since runs are no longer bit-identical, don't reach for a recorded snapshot to
  detect drift — assert invariants over many seeds (see
  `invariants_hold_across_many_seeds`) and re-run the seeded guard
  (`same_seed_replays_bit_identically`) to confirm a change didn't perturb RNG
  ordering. Performance is the criterion bench; correctness is the invariant net.
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
  before chasing further constant factors (flat array grid, rayon over the
  read-only first pass). Note the per-bee candidate sort can't simply be dropped
  for speed: it's what keeps the grid bit-identical to the naive oracle
  (`grid_matches_naive`) and the seeded replay stable (`same_seed_replays_bit_identically`).
  Relaxing it means relaxing those guards too — a deliberate trade, not a freebie
  now that cross-run determinism is gone.

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
