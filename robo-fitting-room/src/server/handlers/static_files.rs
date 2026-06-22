//! Serves the SPA either from disk (`--web-root` set) or from the
//! `include_dir!` bundle baked into the binary.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};

use include_dir::{include_dir, Dir};

use crate::server::state::AppState;

/// SPA bundle baked at compile time. CARGO_MANIFEST_DIR points at the
/// `robo-fitting-room` crate root; the `web/` subdir is the SPA source.
static EMBEDDED_WEB: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/web");

/// Catch-all GET for non-API paths. Resolves against `--web-root` when set,
/// otherwise from the embedded bundle. Falls back to `index.html` for
/// any unknown path so the SPA can do client-side routing later if it
/// ever needs to.
pub async fn handler(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    if let Some(root) = state.web_root.as_ref() {
        let target = root.join(path);
        if let Ok(bytes) = tokio::fs::read(&target).await {
            let mime = mime_guess::from_path(&target).first_or_octet_stream();
            return ([(header::CONTENT_TYPE, mime.as_ref().to_string())], bytes)
                .into_response();
        }
        // Fall back to index.html so the SPA loads even if the user
        // navigated to a deep path.
        if let Ok(bytes) = tokio::fs::read(root.join("index.html")).await {
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
                bytes,
            )
                .into_response();
        }
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }

    // Embedded path
    if let Some(file) = EMBEDDED_WEB.get_file(path) {
        let mime = mime_guess::from_path(file.path()).first_or_octet_stream();
        return (
            [(header::CONTENT_TYPE, mime.as_ref().to_string())],
            file.contents().to_vec(),
        )
            .into_response();
    }
    if let Some(idx) = EMBEDDED_WEB.get_file("index.html") {
        return (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
            idx.contents().to_vec(),
        )
            .into_response();
    }

    let _ = Body::empty;
    (StatusCode::NOT_FOUND, "not found").into_response()
}
