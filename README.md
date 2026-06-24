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
