use axum::extract::State;
use axum::response::Json;

use crate::config::CsvFormat;
use crate::csv_intake::inspect_csv;
use crate::server::dto::{CsvInspectRequest, CsvInspectResponse};
use crate::server::error::ApiError;
use crate::server::state::AppState;

// FR-702. Inspect a capture: robots, axes, row count, time span.
#[utoipa::path(
    post,
    path = "/api/v1/csv/inspect",
    tag = "csv",
    request_body = CsvInspectRequest,
    responses(
        (status = 200, description = "Capture schema", body = CsvInspectResponse),
        (status = 400, description = "Path / parse error", body = ErrorEnvelope),
    ),
)]
pub async fn inspect(
    State(state): State<AppState>,
    Json(req): Json<CsvInspectRequest>,
) -> Result<Json<CsvInspectResponse>, ApiError> {
    let csv = state
        .resolve_asset(&req.csv_path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let schema = inspect_csv(&csv, CsvFormat::Wide)
        .map_err(|e| ApiError::bad_request("csv_parse_failed", e.to_string()))?;
    Ok(Json(CsvInspectResponse {
        robots: schema.robots,
        axes_per_robot: schema.axes_per_robot,
        row_count: schema.row_count,
        t_min: schema.t_min,
        t_max: schema.t_max,
    }))
}
