//! Colony simulator server.
//!
//! Runs the simulation ([`sim`]) and exposes it:
//! - `GET /ws` — streams binary wire frames (see `colony_core::wire`) over
//!   WebSocket: the roster message when a client hasn't seen the current
//!   membership yet, then a motion message per tick. Frames are encoded once
//!   by the sim task and shared; this handler only forwards bytes.
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

/// Per-connection loop: forward the latest encoded frame whenever it changes,
/// and watch for the client closing the connection.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.sim.frames.clone();
    // Roster version this client has been sent; `None` until the first frame,
    // so a fresh connection always receives the roster before any motion.
    let mut sent_roster: Option<u32> = None;

    // Send the current frame right away so a fresh client renders immediately.
    let initial = rx.borrow_and_update().clone();
    if send_frame(&mut socket, &initial, &mut sent_roster).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    break; // simulation task gone
                }
                let frame = rx.borrow_and_update().clone();
                if send_frame(&mut socket, &frame, &mut sent_roster).await.is_err() {
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

/// Send one encoded tick as binary frames: the roster first if this client
/// hasn't seen the current version (the watch channel may have coalesced past
/// the frame that introduced it), then the motion. The bytes were encoded once
/// by the sim task; the `to_vec` here is a plain memcpy into the socket's
/// message, not a re-serialization.
async fn send_frame(
    socket: &mut WebSocket,
    frame: &sim::WireFrame,
    sent_roster: &mut Option<u32>,
) -> Result<(), axum::Error> {
    if *sent_roster != Some(frame.roster_version) {
        socket.send(Message::Binary(frame.roster.to_vec())).await?;
        *sent_roster = Some(frame.roster_version);
    }
    socket.send(Message::Binary(frame.motion.to_vec())).await
}
