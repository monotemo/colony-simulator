//! Colony simulator server.
//!
//! Runs the simulation ([`sim`]) and exposes it:
//! - `GET /ws` — streams [`colony_core::WorldSnapshot`] frames over WebSocket.
//! - `GET /api/health` — liveness probe.
//! - `POST /api/control` — start / pause / reset the simulation.
//!
//! In production it can also serve the built Angular bundle as static files
//! (set `COLONY_STATIC_DIR`); during development the Angular dev server proxies
//! `/api` and `/ws` here instead.

mod sim;

use std::net::SocketAddr;
use std::path::Path;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;
use tower_http::{cors::CorsLayer, services::ServeDir, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use sim::{Command, SimHandle};

/// Shared application state handed to every request handler.
#[derive(Clone)]
struct AppState {
    sim: SimHandle,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "colony_server=info,tower_http=info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = AppState { sim: sim::spawn() };

    let mut app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/api/health", get(health))
        .route("/api/control", post(control))
        .with_state(state)
        // Permissive CORS so the Angular dev server (different origin) can call us.
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    // Optionally serve the built Angular PWA as static files.
    let static_dir = std::env::var("COLONY_STATIC_DIR")
        .unwrap_or_else(|_| "../frontend/dist/colony-simulator/browser".to_string());
    if Path::new(&static_dir).is_dir() {
        tracing::info!("serving static frontend from {static_dir}");
        app = app.fallback_service(ServeDir::new(static_dir));
    }

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!("listening on http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind listener");
    axum::serve(listener, app).await.expect("server error");
}

/// `GET /api/health`
async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok" }))
}

/// Body of `POST /api/control`.
#[derive(Debug, Deserialize)]
struct ControlRequest {
    command: Command,
}

/// `POST /api/control` — forward a control command to the simulation task.
async fn control(
    State(state): State<AppState>,
    Json(req): Json<ControlRequest>,
) -> impl IntoResponse {
    match state.sim.commands.send(req.command).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response(),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "error", "message": "simulation not running" })),
        )
            .into_response(),
    }
}

/// `GET /ws` — upgrade to a WebSocket and stream snapshots.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

/// Per-connection loop: push the latest snapshot whenever it changes, and
/// watch for the client closing the connection.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.sim.snapshots.clone();

    // Send the current frame right away so a fresh client renders immediately.
    let initial = rx.borrow_and_update().clone();
    if send_snapshot(&mut socket, &initial).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    break; // simulation task gone
                }
                let snapshot = rx.borrow_and_update().clone();
                if send_snapshot(&mut socket, &snapshot).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Err(_)) => break,
                    // Ignore any other client messages for now.
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

/// Serialize a snapshot to JSON and send it as a text frame.
async fn send_snapshot(
    socket: &mut WebSocket,
    snapshot: &colony_core::WorldSnapshot,
) -> Result<(), axum::Error> {
    let json = serde_json::to_string(snapshot).expect("snapshot serializes");
    socket.send(Message::Text(json)).await
}
