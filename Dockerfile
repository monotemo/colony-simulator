# Build and run the colony-server (Axum) backend for Fly.io.
#
# API/WebSocket only: the Angular frontend ships separately to GitHub Pages
# (compiled to wasm), so the image carries just the Rust server binary. The
# server's optional static-file serving stays dormant — COLONY_STATIC_DIR is
# unset, so the `is_dir()` check in main.rs skips it and only /ws + /api answer.

# ---- build stage -------------------------------------------------------------
FROM rust:1-bookworm AS builder

WORKDIR /app
# The whole Cargo workspace lives under backend/. Copy it in and build only the
# server crate in release mode; colony-wasm (cdylib) is never compiled here.
COPY backend/ ./backend/
WORKDIR /app/backend
RUN cargo build --release -p colony-server

# ---- runtime stage -----------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# ca-certificates is the only runtime dependency worth carrying (TLS roots);
# the server itself makes no outbound calls today, but it's cheap insurance.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Run as a non-root user rather than the image default.
RUN useradd --create-home --uid 10001 colony
USER colony

COPY --from=builder /app/backend/target/release/colony-server /usr/local/bin/colony-server

# Matches the address main.rs binds (0.0.0.0:8080) and fly.toml's internal_port.
EXPOSE 8080
CMD ["colony-server"]
