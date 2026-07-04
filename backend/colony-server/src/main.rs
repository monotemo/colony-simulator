//! Colony simulator server.
//!
//! Runs the simulation ([`sim`]) and exposes it:
//! - `GET /ws` — streams binary wire frames (see `colony_core::wire`) over
//!   WebSocket: the roster message when a client hasn't seen the current
//!   membership yet, then a motion message per published tick. Dense frames
//!   are encoded once by the sim task and shared; a client may send a JSON
//!   `{"viewport": {...}}` text message describing the world rect its camera
//!   can see, after which this handler culls its motion to that rect (sparse
//!   frames) so per-client bandwidth scales with what's on screen.
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

/// The world-space rectangle a client's camera can see, as reported over the
/// WebSocket. Mirrored by the `ViewportRect` shape in `models.ts` (sent by
/// `websocket-simulation.ts`); the canvas pads it with a margin before
/// reporting so bees don't pop in at the screen edge.
#[derive(Debug, Clone, Copy, Deserialize)]
struct Viewport {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Viewport {
    /// A usable rect: finite edges, properly ordered. Degenerate reports (NaN,
    /// `min > max`) must be rejected at receipt — a stored one would cull every
    /// bee on every tick, silently starving the client of motion with nothing
    /// to self-correct it, which is worse than keeping the previous delivery.
    fn is_valid(self) -> bool {
        self.min_x.is_finite()
            && self.min_y.is_finite()
            && self.max_x.is_finite()
            && self.max_y.is_finite()
            && self.min_x <= self.max_x
            && self.min_y <= self.max_y
    }
}

/// Messages a client may send on the WebSocket. Externally tagged like the
/// control commands, so a viewport update is `{"viewport": {...}}` — new
/// per-connection message kinds slot in as further variants.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClientMessage {
    Viewport(Viewport),
}

/// Per-connection loop: forward the latest encoded frame whenever it changes,
/// track the client's reported viewport, and watch for the connection closing.
async fn handle_socket(mut socket: WebSocket, state: AppState) {
    let mut rx = state.sim.frames.clone();
    // Roster version this client has been sent; `None` until the first frame,
    // so a fresh connection always receives the roster before any motion.
    let mut sent_roster: Option<u32> = None;
    // The client's camera rect, once it reports one. Until then it gets the
    // shared dense broadcast.
    let mut viewport: Option<Viewport> = None;

    // Send the current frame right away so a fresh client renders immediately.
    let initial = rx.borrow_and_update().clone();
    if send_frame(&mut socket, &initial, &mut sent_roster, viewport).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    break; // simulation task gone
                }
                let frame = rx.borrow_and_update().clone();
                if send_frame(&mut socket, &frame, &mut sent_roster, viewport).await.is_err() {
                    break;
                }
            }
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        // A malformed or degenerate message is dropped rather
                        // than fatal: the client keeps its previous (or dense)
                        // delivery.
                        if let Ok(ClientMessage::Viewport(vp)) = serde_json::from_str(&text) {
                            if vp.is_valid() {
                                viewport = Some(vp);
                            }
                        }
                    }
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
/// the frame that introduced it), then the motion — dense (the sim task's
/// shared bytes; `to_vec` is a plain memcpy, not a re-serialization) when the
/// client has no viewport or can see everything, sparse (culled and encoded
/// here, O(visible)) when a viewport hides part of the colony.
async fn send_frame(
    socket: &mut WebSocket,
    frame: &sim::WireFrame,
    sent_roster: &mut Option<u32>,
    viewport: Option<Viewport>,
) -> Result<(), axum::Error> {
    if *sent_roster != Some(frame.roster_version) {
        socket.send(Message::Binary(frame.roster.to_vec())).await?;
        *sent_roster = Some(frame.roster_version);
    }
    let motion = viewport
        .and_then(|vp| cull_motion(frame, vp))
        .unwrap_or_else(|| frame.motion.to_vec());
    socket.send(Message::Binary(motion)).await
}

/// Encode a sparse motion frame for the bees inside `viewport`, or `None` when
/// every bee is visible — the shared dense bytes are cheaper then (no index
/// overhead, no re-encode).
fn cull_motion(frame: &sim::WireFrame, viewport: Viewport) -> Option<Vec<u8>> {
    // Cheap pre-check: a viewport covering the whole world (the common
    // zoomed-out case — bees are confined to bounds) can't cull anything, so
    // skip the per-bee filter entirely rather than building an index list
    // just to discard it below.
    let bounds = frame.snapshot.bounds;
    if viewport.min_x <= 0.0
        && viewport.min_y <= 0.0
        && viewport.max_x >= bounds.width
        && viewport.max_y >= bounds.height
    {
        return None;
    }

    let bees = &frame.snapshot.bees;
    let indices: Vec<u32> = bees
        .iter()
        .enumerate()
        .filter(|(_, bee)| {
            let p = bee.position;
            p.x >= viewport.min_x && p.x <= viewport.max_x
                && p.y >= viewport.min_y && p.y <= viewport.max_y
        })
        .map(|(i, _)| i as u32)
        .collect();
    if indices.len() == bees.len() {
        return None;
    }
    Some(colony_core::wire::encode_motion_sparse(
        &frame.snapshot,
        frame.roster_version,
        &indices,
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use colony_core::{Engine, WireEncoder};

    use super::*;

    fn frame() -> sim::WireFrame {
        let engine = Engine::seeded();
        let snapshot = Arc::new(engine.snapshot());
        let frames = WireEncoder::new().encode(&snapshot);
        sim::WireFrame {
            roster_version: frames.roster_version,
            roster: frames.roster,
            motion: frames.motion.into(),
            snapshot,
        }
    }

    #[test]
    fn degenerate_viewports_are_rejected() {
        let valid = Viewport { min_x: 0.0, min_y: 0.0, max_x: 10.0, max_y: 10.0 };
        assert!(valid.is_valid());
        // Inverted on either axis, or any non-finite edge, must be refused —
        // storing one would cull every bee forever (see `Viewport::is_valid`).
        assert!(!Viewport { min_x: 10.0, max_x: 0.0, ..valid }.is_valid());
        assert!(!Viewport { min_y: 10.0, max_y: 0.0, ..valid }.is_valid());
        assert!(!Viewport { min_x: f64::NAN, ..valid }.is_valid());
        assert!(!Viewport { max_y: f64::INFINITY, ..valid }.is_valid());
    }

    #[test]
    fn world_covering_viewport_falls_back_to_dense_without_filtering() {
        let frame = frame();
        let bounds = frame.snapshot.bounds;
        let whole_world = Viewport {
            min_x: -1.0,
            min_y: -1.0,
            max_x: bounds.width + 1.0,
            max_y: bounds.height + 1.0,
        };
        assert!(cull_motion(&frame, whole_world).is_none());
    }

    #[test]
    fn partial_viewport_produces_a_sparse_frame() {
        let frame = frame();
        let bounds = frame.snapshot.bounds;
        // A quarter of the world: some bees must be culled on the seeded layout.
        let quarter = Viewport {
            min_x: 0.0,
            min_y: 0.0,
            max_x: bounds.width / 2.0,
            max_y: bounds.height / 2.0,
        };
        let sparse = cull_motion(&frame, quarter).expect("some bees off-screen");
        assert_eq!(sparse[0], colony_core::wire::SPARSE_MOTION_MESSAGE);
    }
}
