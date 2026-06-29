# Build and run the full colony-simulator on Fly.io: the Axum backend plus the
# Angular frontend it serves as static files.
#
# The image carries one process — the Rust `colony-server` — which answers
# /ws + /api and, when COLONY_STATIC_DIR points at a real directory, falls back
# to serving the compiled Angular bundle (see main.rs). The frontend runs the
# simulation in-browser via WebAssembly (environment.production.useWasm = true),
# so the server's own tick loop is unused by visitors; it's still available for
# anyone pointing the WebSocket transport at /ws.
#
# Three stages: build the server binary and the wasm engine with Cargo, build
# the Angular app with Node (consuming that wasm), then assemble a slim runtime.

# ---- rust stage: server binary + wasm engine ---------------------------------
FROM rust:1-bookworm AS rust-builder

# wasm-pack + the wasm target compile colony-wasm to a browser ES module.
RUN rustup target add wasm32-unknown-unknown \
    && curl https://rustwasm.github.io/wasm-pack/installer/init.sh -sSf | sh

WORKDIR /app
# The whole Cargo workspace lives under backend/. Copy it in, build the server
# in release mode, then build the wasm engine into a standalone package dir.
# --manifest-path points cargo at the workspace root (backend/Cargo.toml); the
# build target lands at /app/backend/target.
COPY backend/ ./backend/
RUN cargo build --release --manifest-path backend/Cargo.toml -p colony-server
RUN wasm-pack build backend/colony-wasm --target web --release --out-dir /wasm-pkg

# ---- node stage: Angular production build ------------------------------------
FROM node:24-bookworm-slim AS frontend-builder

WORKDIR /app/frontend
# Install deps first so the layer caches across source-only changes.
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci

# Bring in the frontend sources and the wasm package the loader expects under
# public/wasm/ (assets glob copies public/** to the site root → /wasm/...).
COPY frontend/ ./
COPY --from=rust-builder /wasm-pkg ./public/wasm

# Default base-href (/) because the server serves the app at the origin root,
# unlike the former /colony-simulator/ GitHub Pages subpath.
RUN npx ng build

# ---- runtime stage -----------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# ca-certificates is the only runtime dependency worth carrying (TLS roots);
# the server itself makes no outbound calls today, but it's cheap insurance.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as a non-root user rather than the image default.
RUN useradd --create-home --uid 10001 colony

COPY --from=rust-builder /app/backend/target/release/colony-server /usr/local/bin/colony-server
# The built Angular bundle; COLONY_STATIC_DIR tells the server to serve it.
COPY --from=frontend-builder /app/frontend/dist/colony-simulator/browser /srv/frontend
ENV COLONY_STATIC_DIR=/srv/frontend

USER colony

# Matches the address main.rs binds (0.0.0.0:8080) and fly.toml's internal_port.
EXPOSE 8080
CMD ["colony-server"]
