//! In-binary HTTP server exposing the dresser's `dress` / `validate` /
//! `batch` / `list` operations as a versioned REST API. Gated on the
//! `server` Cargo feature so default builds stay CLI-only and pull in
//! zero axum/tokio dependencies.
//!
//! Both transports — CLI (`urdf-robo-dresser dress …`) and REST
//! (`POST /api/v1/dress`) — call the same library functions
//! (`merge`, `validate`, `parse_urdf`, `serialize_urdf`,
//! `catalogue::scan`). Single source of truth.
//!
//! Routes:
//!   * GET  /api/v1/catalogue                    catalogue listing
//!   * GET  /api/v1/catalogue/robots             robot listing only
//!   * GET  /api/v1/catalogue/tools              tool listing only
//!   * POST /api/v1/dress                        merge robot + tool
//!   * POST /api/v1/validate                     validate a URDF body
//!   * POST /api/v1/batch                        kick off a batch job
//!   * GET  /api/v1/jobs/:id                     poll a batch job
//!   * GET  /api/v1/jobs/:id/artifact            download the batch zip
//!   * GET  /assets/*path                        raw catalogue bytes (URDF + meshes)
//!   * GET  /healthz                             liveness
//!   * GET  /readyz                              readiness
//!   * GET  /openapi.json                        spec
//!   * GET  /docs                                Swagger UI
//!   * GET  /  (+ fallback)                      embedded SPA (3D URDF viewer)

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::CorsLayer;

pub mod dto;
pub mod error;
pub mod handlers;
pub mod openapi;
pub mod state;

pub use error::ApiError;
pub use state::{AppState, JobRegistry};

use crate::error::DresserError;

/// Build the axum router for tests + production. State carries the
/// catalogue root + an `Arc<RwLock<JobRegistry>>` for async batches.
pub fn router(state: AppState) -> Router {
    let api_v1 = Router::new()
        .route("/catalogue", get(handlers::catalogue::list_all))
        .route("/catalogue/robots", get(handlers::catalogue::list_robots))
        .route("/catalogue/tools", get(handlers::catalogue::list_tools))
        .route("/catalogue/dressed", get(handlers::catalogue::list_dressed))
        .route("/dress", post(handlers::dress::dress))
        .route("/validate", post(handlers::validate::validate))
        .route("/batch", post(handlers::batch::start))
        .route("/jobs/:id", get(handlers::jobs::status))
        .route("/jobs/:id/artifact", get(handlers::jobs::artifact));

    Router::new()
        .nest("/api/v1", api_v1)
        // FR-056: raw catalogue bytes for the browser 3D viewer.
        .route("/assets/*path", get(handlers::assets::serve_asset))
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        .merge(openapi::docs_router())
        // FR-052: the SPA owns "/" and every other unmatched path. axum tries
        // the explicit routes above first, so /api/v1, /assets, /docs,
        // /openapi.json, /healthz, /readyz all still win.
        .fallback(handlers::static_files::handler)
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Bind to `host:port` and run until the process is killed. Mirrors the
/// fitting-room banner so users see the URL on stdout on first launch.
pub async fn run(
    host: &str,
    port: u16,
    catalogue: PathBuf,
    web_root: Option<PathBuf>,
) -> Result<(), DresserError> {
    let canon = std::fs::canonicalize(&catalogue).map_err(DresserError::from)?;
    let state = AppState::new(canon.clone()).with_web_root(web_root.clone());
    let app = router(state);

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| DresserError::Io(std::io::Error::other(format!(
            "invalid bind address {}:{} — {}",
            host, port, e
        ))))?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(DresserError::from)?;

    let url_host = if host == "0.0.0.0" { "localhost" } else { host };
    let spa_src = match web_root.as_ref() {
        Some(dir) => format!("disk ({})", dir.display()),
        None => "embedded".to_string(),
    };
    eprintln!();
    eprintln!("  urdf-robo-dresser  ▸  3D viewer + REST API");
    eprintln!("  ─────────────────────────────────────────────");
    eprintln!("  Listening on   http://{}", addr);
    eprintln!("  Open in browser → http://{}:{}/", url_host, port);
    eprintln!("  API docs        http://{}:{}/docs", url_host, port);
    eprintln!("  Catalogue       {}", canon.display());
    eprintln!("  SPA source      {}", spa_src);
    eprintln!("  Press Ctrl-C to stop.");
    eprintln!();
    log::info!("urdf-robo-dresser serve listening on http://{}", addr);
    axum::serve(listener, app).await.map_err(|e| {
        DresserError::Io(std::io::Error::other(format!("axum::serve: {}", e)))
    })?;
    Ok(())
}
