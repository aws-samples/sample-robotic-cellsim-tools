//! In-binary HTTP server: a versioned REST API over the same playback pipeline
//! the CLI drives. Gated behind the `server` Cargo feature so default builds
//! stay lean. API routes live under `/api/v1/...`; liveness/readiness probes
//! sit at the root. There is no web UI SPA yet (tracked in REQUIREMENTS.md
//! Backlog); this is the REST scaffold the future UI will call.

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

pub mod dto;
pub mod error;
pub mod handlers;
pub mod openapi;
pub mod state;

pub use error::ApiError;
pub use state::AppState;

use crate::error::R3Error;

// FR-700. Build the axum router. `state` carries the asset root + job registry.
pub fn router(state: AppState) -> Router {
    let api_v1 = Router::new()
        .route("/topics", post(handlers::topics::resolve))
        .route("/csv/inspect", post(handlers::csv::inspect))
        .route("/playback", post(handlers::playback::start))
        .route("/playback/:id", get(handlers::playback::status))
        .route("/playback/:id", delete(handlers::playback::stop));

    Router::new()
        .nest("/api/v1", api_v1)
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        .merge(openapi::docs_router())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

// FR-700. Bind to `host:port` and run until killed.
pub async fn run(host: &str, port: u16, asset_root: PathBuf) -> Result<(), R3Error> {
    let asset_root_canon = std::fs::canonicalize(&asset_root).map_err(|e| {
        R3Error::Config(format!("asset_root {} not found: {}", asset_root.display(), e))
    })?;
    let state = AppState::new(asset_root_canon.clone());
    let app = router(state);
    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| {
            R3Error::Config(format!("invalid bind address {}:{} — {}", host, port, e))
        })?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| R3Error::Config(format!("bind {}: {}", addr, e)))?;

    let url_host = if host == "0.0.0.0" { "localhost" } else { host };
    eprintln!();
    eprintln!("  robo-ros-runner (R3)  ▸  REST API");
    eprintln!("  ───────────────────────────────────────────────");
    eprintln!("  Listening on   http://{}", addr);
    eprintln!("  Swagger UI   → http://{}:{}/docs", url_host, port);
    eprintln!("  Asset root     {}", asset_root_canon.display());
    eprintln!("  Press Ctrl-C to stop.");
    eprintln!();
    log::info!("robo-ros-runner serve listening on http://{}", addr);
    axum::serve(listener, app)
        .await
        .map_err(|e| R3Error::Config(format!("axum::serve: {}", e)))?;
    Ok(())
}
