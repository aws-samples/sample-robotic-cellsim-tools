//! Liveness + readiness probes. Routed at the *root* (`/healthz`,
//! `/readyz`) — outside `/api/v1` — to match the convention every k8s
//! probe expects out of the box.

use std::path::Path;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

use crate::server::dto::{HealthDto, ReadinessDto};
use crate::server::state::AppState;

/// Recursively count `*.urdf` files under `dir` up to `depth` levels deep.
/// Bounded so a misconfigured asset_root pointing at a giant filesystem
/// doesn't stall the readiness probe.
fn count_urdfs(dir: &Path, depth: u32) -> usize {
    if depth == 0 {
        return 0;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.eq_ignore_ascii_case("urdf"))
                .unwrap_or(false)
        {
            count += 1;
        } else if path.is_dir() {
            count += count_urdfs(&path, depth - 1);
        }
    }
    count
}

/// `GET /healthz` — always 200 once axum is accepting connections. The
/// `version` field is sourced from `CARGO_PKG_VERSION` at compile time.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        get,
        path = "/healthz",
        tag = "system",
        responses(
            (status = 200, description = "Service is alive", body = HealthDto),
        ),
    )
)]
pub async fn healthz() -> Json<HealthDto> {
    Json(HealthDto {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

/// `GET /readyz` — 200 when the asset root canonicalises and at least one
/// `*.urdf` is discoverable; 503 otherwise. The walk is shallow (depth 4)
/// so a misconfigured asset_root with a giant filesystem doesn't stall
/// the probe.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        get,
        path = "/readyz",
        tag = "system",
        responses(
            (status = 200, description = "Service is ready", body = ReadinessDto),
            (status = 503, description = "Service not ready", body = ReadinessDto),
        ),
    )
)]
pub async fn readyz(State(state): State<AppState>) -> Response {
    let asset_root = state.asset_root.as_path().to_path_buf();
    let asset_root_str = asset_root.display().to_string();

    let canon = match std::fs::canonicalize(&asset_root) {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadinessDto {
                    ready: false,
                    asset_root: asset_root_str,
                    urdf_count: 0,
                    errors: vec![format!("asset_root canonicalize: {}", e)],
                }),
            )
                .into_response();
        }
    };

    let urdf_count = count_urdfs(&canon, 5);

    if urdf_count == 0 {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ReadinessDto {
                ready: false,
                asset_root: asset_root_str,
                urdf_count: 0,
                errors: vec!["no *.urdf found under asset_root (depth 4)".to_string()],
            }),
        )
            .into_response();
    }

    Json(ReadinessDto {
        ready: true,
        asset_root: asset_root_str,
        urdf_count,
        errors: vec![],
    })
    .into_response()
}
