use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::server::dto::{PathQuery, SaveResponse, ScenePathBody};
use crate::server::error::ApiError;
use crate::server::state::AppState;

/// POST /api/v1/scenes/load — body `{path}`. Returns the raw YAML at
/// `<asset_root>/<path>`. POST + JSON body keeps relative paths
/// (e.g. with `..` segments) intact without URL-encoding pain.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/scenes/load",
        tag = "scene",
        request_body = PathQuery,
        responses(
            (status = 200, description = "Manifest YAML body", content_type = "application/x-yaml"),
            (status = 400, description = "Path resolution failed", body = ErrorEnvelope),
            (status = 404, description = "Manifest not found", body = ErrorEnvelope),
        ),
    )
)]
pub async fn load(
    State(state): State<AppState>,
    Json(q): Json<PathQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let abs = state
        .resolve_asset(&q.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let body = tokio::fs::read_to_string(&abs)
        .await
        .map_err(|e| ApiError::not_found("manifest_not_found", e.to_string()))?;
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/x-yaml")],
        body,
    ))
}

/// POST /api/v1/scenes/save — body is `{path, yaml}`. Path is resolved
/// against `asset_root`; the YAML is written verbatim.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/scenes/save",
        tag = "scene",
        request_body = ScenePathBody,
        responses(
            (status = 200, description = "Manifest persisted", body = SaveResponse),
            (status = 400, description = "Path resolution failed", body = ErrorEnvelope),
            (status = 500, description = "Filesystem write failed", body = ErrorEnvelope),
        ),
    )
)]
pub async fn save(
    State(state): State<AppState>,
    Json(body): Json<ScenePathBody>,
) -> Result<Json<SaveResponse>, ApiError> {
    let abs = state
        .resolve_asset(&body.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ApiError::internal("mkdir_failed", e.to_string()))?;
    }
    tokio::fs::write(&abs, &body.yaml)
        .await
        .map_err(|e| ApiError::internal("write_failed", e.to_string()))?;
    Ok(Json(SaveResponse {
        ok: true,
        bytes: body.yaml.len() as u64,
    }))
}
