use axum::extract::State;
use axum::response::Json;

use robo_fitting_room::parse_scene;

use crate::config::{CsvFormat, RunConfig};
use crate::csv_intake::inspect_csv;
use crate::server::dto::{TopicsRequest, TopicsResponse};
use crate::server::error::ApiError;
use crate::server::state::AppState;
use crate::topics::resolve_topics;

// FR-701. Resolve the per-robot joint_command topics for a cell manifest.
#[utoipa::path(
    post,
    path = "/api/v1/topics",
    tag = "topics",
    request_body = TopicsRequest,
    responses(
        (status = 200, description = "Resolved topics", body = TopicsResponse),
        (status = 400, description = "Path / parse error", body = ErrorEnvelope),
    ),
)]
pub async fn resolve(
    State(state): State<AppState>,
    Json(req): Json<TopicsRequest>,
) -> Result<Json<TopicsResponse>, ApiError> {
    let cell = state
        .resolve_asset(&req.cell_path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let scene = parse_scene(&cell)
        .map_err(|e| ApiError::bad_request("scene_parse_failed", e.to_string()))?;

    let csv_robots = match &req.csv_path {
        Some(p) => {
            let csv = state
                .resolve_asset(p)
                .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
            inspect_csv(&csv, CsvFormat::Wide)
                .map_err(|e| ApiError::bad_request("csv_parse_failed", e.to_string()))?
                .robots
        }
        None => Vec::new(),
    };

    let cfg = RunConfig::from_paths(cell, req.csv_path.clone().unwrap_or_default().into());
    let topics = resolve_topics(&scene, &cfg, &csv_robots)
        .map_err(|e| ApiError::bad_request("topic_resolution_failed", e.to_string()))?;

    Ok(Json(TopicsResponse {
        cell_name: scene.name,
        topics,
    }))
}
