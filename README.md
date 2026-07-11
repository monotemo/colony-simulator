# colony-simulator

A web application that simulates a colony of bees. The simulation runs
server-side in **Rust** and is rendered in the browser by an **Angular PWA**,
which connects over a WebSocket and draws the world in real time.

This is the foundational vertical slice: bees wander inside a bounded world
with a few nectar sources, the engine advances time on a fixed timestep, and
the UI streams and renders the result. Foraging, the full bee lifecycle, and
collision detection are planned for later iterations — the module boundaries
are in place to accommodate them.

## Architecture

The Rust server is the **authority**: it owns the simulation and ticks it
~30×/second. Each tick it publishes the latest `WorldSnapshot` to connected
clients over a WebSocket; control actions (start / pause / reset) go the other
way over REST. The Angular app is a thin renderer plus controls.

```
backend/                      # Cargo workspace
  colony-core/                # pure simulation library (no async/networking)
    math, entity, bee, world, engine, snapshot
  colony-server/              # Axum binary: ticks the engine, serves WS + REST
frontend/                     # Angular PWA (renders snapshots on a canvas)
```

### Server endpoints

| Method | Path           | Purpose                                            |
| ------ | -------------- | -------------------------------------------------- |
| GET    | `/ws`          | WebSocket stream of `WorldSnapshot` JSON frames    |
| GET    | `/api/health`  | Liveness probe → `{"status":"ok"}`                 |
| POST   | `/api/control` | Body `{"command":"start"\|"pause"\|"reset"}`       |

## Prerequisites

- **Rust** (stable) with Cargo
- **Node.js** ≥ 20.19 / ≥ 22.12 and npm (the frontend targets Angular 20)

## Running it

### 1. Backend

```bash
cd backend
cargo test          # run the core simulation tests
cargo run -p colony-server
```

The server listens on `http://localhost:8080`. Quick checks:

```bash
curl localhost:8080/api/health
curl -X POST localhost:8080/api/control \
  -H 'content-type: application/json' -d '{"command":"start"}'
```

### 2. Frontend (development)

```bash
cd frontend
npm install
npm start            # ng serve, with proxy.conf.json routing /api and /ws to :8080
```

Open `http://localhost:4200` — you should see bees moving inside the world
bounds, with Start / Pause / Reset controls and a live tick/connection readout.
The dev server proxies `/api` and `/ws` to the Rust server (see
`frontend/proxy.conf.json`), so no CORS configuration is needed.

### Production (single origin)

Build the PWA and let the Rust server serve it as static files:

```bash
cd frontend && npm run build          # outputs to frontend/dist/colony-simulator/browser
cd ../backend && cargo run -p colony-server
```

The server serves the built bundle from `COLONY_STATIC_DIR`
(default `../frontend/dist/colony-simulator/browser`) and the whole app is
available at `http://localhost:8080`.

## Deployment (Cloudflare Workers)

The deployed simulation runs **in the browser via WebAssembly**
(`useWasm: true`), so the production app is entirely static: `wasm-pack`
compiles the `colony-wasm` engine, the Angular build bakes it into the bundle,
and an assets-only [Cloudflare Worker](https://developers.cloudflare.com/workers/static-assets/)
(`frontend/wrangler.jsonc`) serves the result. Static asset requests are free
and unmetered on every Cloudflare plan, which is all this PWA needs — each
visitor gets their own world, and no server runs behind the deployed origin.

The `.github/workflows/cloudflare-deploy.yml` workflow builds the wasm engine
and the Angular bundle, then runs `wrangler deploy`, on pushes to `main` that
touch the backend or frontend (and via manual dispatch). It needs
`CLOUDFLARE_API_TOKEN` (a token from the *Edit Cloudflare Workers* template)
and `CLOUDFLARE_ACCOUNT_ID` repository secrets.

To deploy by hand (requires `wasm-pack`, the `wasm32-unknown-unknown` target,
and a `wrangler login`):

```bash
rustup target add wasm32-unknown-unknown
cd frontend
npm run deploy          # build:static (wasm-pack + ng build), then wrangler deploy
```

The server-backed variant — a live WebSocket stream from `colony-server`, one
shared world — is no longer what's deployed, but the `Dockerfile` still builds
a self-contained image of it for anyone who wants to host one:

```bash
docker build -t colony . && docker run -p 8080:8080 colony
```

## Desktop app (Tauri)

The same static wasm bundle also ships as a native desktop app via a
[Tauri v2](https://v2.tauri.app) shell (`frontend/src-tauri`). The shell is
pure packaging — the engine still runs as WebAssembly inside the webview, no
Tauri APIs are exposed — built with the `desktop` Angular configuration, which
is production minus the service worker (pointless inside an installed app,
and unreliable under Tauri's custom protocol).

Prerequisites: Rust, `wasm-pack`, the `wasm32-unknown-unknown` target, and
[Tauri's platform dependencies](https://v2.tauri.app/start/prerequisites/)
(on Linux: `libwebkit2gtk-4.1-dev` and friends).

```bash
cd frontend
npm run tauri:dev       # dev shell over ng serve — WebSocket transport, so run
                        # `cargo run -p colony-server` first, like browser dev
npm run tauri:build     # wasm-pack + ng build --configuration desktop, then
                        # native installers → frontend/src-tauri/target/release/bundle/
```

Pushing a `v*` tag runs `.github/workflows/desktop-release.yml`, which builds
installers for Windows, macOS (Apple Silicon + Intel), and Linux and attaches
them to a draft GitHub release. The macOS artifacts are unsigned/unnotarized,
so Gatekeeper warns on first launch.
