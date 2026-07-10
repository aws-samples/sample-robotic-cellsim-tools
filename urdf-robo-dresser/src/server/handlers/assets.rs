//! GET /assets/*path — stream raw catalogue bytes (URDF + meshes) to the
//! browser 3D viewer. Sandboxed through AppState::resolve_asset (rejects
//! absolute paths and any '..' segment). Feeds the WebGL viewer: urdf-loader
//! fetches a dressed URDF and resolves its relative meshes against this route.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::server::error::ApiError;
use crate::server::state::AppState;

// FR-056
pub async fn serve_asset(
    State(state): State<AppState>,
    Path(rel): Path<String>,
) -> Result<Response, ApiError> {
    // resolve_asset already rejects absolute + ".." (axum percent-decodes
    // the wildcard first, so "%2e%2e" arrives as ".." and is rejected too).
    let abs = state
        .resolve_asset(&rel)
        .map_err(|e| ApiError::bad_request("invalid_asset_path", e.to_string()))?;

    let bytes = tokio::fs::read(&abs)
        .await
        .map_err(|e| ApiError::not_found("asset_not_found", format!("{}: {}", rel, e)))?;

    let ctype = content_type_for(&abs);
    Ok(([(header::CONTENT_TYPE, ctype)], Bytes::from(bytes)).into_response())
}

/// Explicit map for the mesh/URDF types urdf-loader fetches; octet-stream
/// fallback via mime_guess. Content-Type is advisory for three.js loaders
/// (they parse by loader, not MIME) but correct types keep DevTools sane.
fn content_type_for(path: &std::path::Path) -> String {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("urdf") | Some("xml") => "application/xml; charset=utf-8".into(),
        Some("obj") => "text/plain; charset=utf-8".into(),
        Some("mtl") => "text/plain; charset=utf-8".into(),
        Some("stl") => "model/stl".into(),
        Some("dae") => "model/vnd.collada+xml".into(),
        _ => mime_guess::from_path(path)
            .first_or_octet_stream()
            .as_ref()
            .to_string(),
    }
}
