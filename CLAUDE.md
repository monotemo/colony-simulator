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
