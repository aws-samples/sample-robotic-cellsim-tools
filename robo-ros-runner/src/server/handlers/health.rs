//! Liveness + readiness probes, routed at the root to match k8s conventions.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

use crate::server::dto::{HealthDto, ReadinessDto};
use crate::server::state::AppState;

// FR-707.
#[utoipa::path(
    get,
    path = "/healthz",
    tag = "system",
    responses((status = 200, description = "Service is alive", body = HealthDto)),
)]
pub async fn healthz() -> Json<HealthDto> {
    Json(HealthDto {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

// FR-707. Ready when the asset root canonicalises.
#[utoipa::path(
    get,
    path = "/readyz",
    tag = "system",
    responses(
        (status = 200, description = "Service is ready", body = ReadinessDto),
        (status = 503, description = "Service not ready", body = ReadinessDto),
    ),
)]
pub async fn readyz(State(state): State<AppState>) -> Response {
    let asset_root = state.asset_root.as_path().to_path_buf();
    let asset_root_str = asset_root.display().to_string();
    match std::fs::canonicalize(&asset_root) {
        Ok(_) => Json(ReadinessDto {
            ready: true,
            asset_root: asset_root_str,
            errors: vec![],
        })
        .into_response(),
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessDto {
                ready: false,
                asset_root: asset_root_str,
                errors: vec![format!("asset_root canonicalize: {}", e)],
            }),
        )
            .into_response(),
    }
}
