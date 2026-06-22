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
//!   * GET  /healthz                             liveness
//!   * GET  /readyz                              readiness
//!   * GET  /openapi.json                        spec
//!   * GET  /docs                                Swagger UI

use std::net::SocketAddr;
use std::path::PathBuf;

use axum::http::header;
use axum::response::IntoResponse;
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
        .route("/dress", post(handlers::dress::dress))
        .route("/validate", post(handlers::validate::validate))
        .route("/batch", post(handlers::batch::start))
        .route("/jobs/:id", get(handlers::jobs::status))
        .route("/jobs/:id/artifact", get(handlers::jobs::artifact));

    Router::new()
        .nest("/api/v1", api_v1)
        .route("/", get(landing_page))
        .route("/healthz", get(handlers::health::healthz))
        .route("/readyz", get(handlers::health::readyz))
        .merge(openapi::docs_router())
        .layer(CorsLayer::permissive())
        .with_state(state)
}

/// Tiny landing page for `GET /`. The dresser is headless — no SPA, no
/// static editor — but a bare-host visit otherwise shows nothing, which
/// reads as "is it broken?". This page links to the actual surfaces.
async fn landing_page() -> impl IntoResponse {
    let html = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>urdf-robo-dresser</title>
  <style>
    body { font: 14px -apple-system, system-ui, sans-serif; max-width: 42em;
           margin: 4em auto; padding: 0 1em; color: #222; line-height: 1.5; }
    h1 { font-size: 1.4em; margin-bottom: 0.2em; }
    .sub { color: #666; margin-bottom: 2em; }
    ul { padding-left: 1.2em; }
    code { background: #f0f0f0; padding: 0.1em 0.35em; border-radius: 3px; }
    a { color: #0451a5; }
  </style>
</head>
<body>
  <h1>urdf-robo-dresser</h1>
  <p class="sub">Headless URDF merge service. There is no web UI — the
  surfaces below are everything the server exposes.</p>

  <h2>Browser-friendly</h2>
  <ul>
    <li><a href="/docs">/docs</a> — Swagger UI (interactive API explorer)</li>
    <li><a href="/openapi.json">/openapi.json</a> — raw OpenAPI 3.x spec</li>
    <li><a href="/healthz">/healthz</a> — liveness probe</li>
    <li><a href="/readyz">/readyz</a> — readiness probe (catalogue scan)</li>
  </ul>

  <h2>REST surface (versioned)</h2>
  <ul>
    <li><code>GET  /api/v1/catalogue</code> — list robots + tools</li>
    <li><code>GET  /api/v1/catalogue/robots</code></li>
    <li><code>GET  /api/v1/catalogue/tools</code></li>
    <li><code>POST /api/v1/dress</code> — merge robot + tool by catalogue id</li>
    <li><code>POST /api/v1/validate</code> — validate a URDF by path</li>
    <li><code>POST /api/v1/batch</code> — kick off a permutation merge job</li>
    <li><code>GET  /api/v1/jobs/:id</code> — poll a batch job</li>
    <li><code>GET  /api/v1/jobs/:id/artifact</code> — download the batch zip</li>
  </ul>

  <p>For the 2D scene-layout editor, see <code>robo-fitting-room serve</code>
  (a separate service on a different port).</p>
</body>
</html>"#;
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

/// Bind to `host:port` and run until the process is killed. Mirrors the
/// fitting-room banner so users see the URL on stdout on first launch.
pub async fn run(host: &str, port: u16, catalogue: PathBuf) -> Result<(), DresserError> {
    let canon = std::fs::canonicalize(&catalogue).map_err(DresserError::from)?;
    let state = AppState::new(canon.clone());
    let app = router(state);

    let addr: SocketAddr = format!("{}:{}", host, port)
        .parse()
        .map_err(|e: std::net::AddrParseError| DresserError::Io(std::io::Error::other(format!(
            "invalid bind address {}:{} — {}",
            host, port, e
        ))))?;
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(DresserError::from)?;

    let url_host = if host == "0.0.0.0" { "localhost" } else { host };
    eprintln!();
    eprintln!("  urdf-robo-dresser  ▸  catalogue + REST API");
    eprintln!("  ─────────────────────────────────────────────");
    eprintln!("  Listening on   http://{}", addr);
    eprintln!("  Open in browser → http://{}:{}/docs", url_host, port);
    eprintln!("  Catalogue       {}", canon.display());
    eprintln!("  Press Ctrl-C to stop.");
    eprintln!();
    log::info!("urdf-robo-dresser serve listening on http://{}", addr);
    axum::serve(listener, app).await.map_err(|e| {
        DresserError::Io(std::io::Error::other(format!("axum::serve: {}", e)))
    })?;
    Ok(())
}
