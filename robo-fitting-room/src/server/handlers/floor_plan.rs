use axum::extract::State;
use axum::response::Json;

use crate::floor_plan::load_floor_plan;
use crate::server::dto::{FloorPlanSegments, PathQuery};
use crate::server::error::ApiError;
use crate::server::state::AppState;

/// POST /api/v1/floor-plan — body `{path}`. Parses the OBJ at the given
/// relative path through `floor_plan::load_floor_plan` and returns the
/// segment arrays as JSON. The editor's canvas renders these directly.
///
/// Offset and scale are deliberately set to identity here — the editor
/// renders in DXF/OBJ-local coordinates and applies the user's
/// `cell_origin_dxf` pick on its own. The compose pipeline applies
/// the offset later.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/floor-plan",
        tag = "floor-plan",
        request_body = PathQuery,
        responses(
            (status = 200, description = "Parsed floor-plan segments", body = FloorPlanSegments),
            (status = 400, description = "Path resolution or OBJ parse error", body = ErrorEnvelope),
        ),
    )
)]
pub async fn segments(
    State(state): State<AppState>,
    Json(q): Json<PathQuery>,
) -> Result<Json<FloorPlanSegments>, ApiError> {
    let abs = state
        .resolve_asset(&q.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let fp = load_floor_plan(&abs, [0.0; 3], 1.0)
        .map_err(|e| ApiError::bad_request("floor_plan_parse_failed", e.to_string()))?;
    Ok(Json(FloorPlanSegments {
        segments: fp.segments,
        footprint_segments: fp.footprint_segments,
    }))
}
