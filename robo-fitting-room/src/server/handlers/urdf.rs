use axum::extract::State;
use axum::response::Json;

use urdf_robo_dresser::{parse_urdf, JointType};

use crate::server::dto::{JointInfo, PathQuery, UrdfJointsResponse};
use crate::server::error::ApiError;
use crate::server::state::AppState;

/// POST /api/v1/urdf/joints — body `{path}`. Parses the URDF at
/// `<asset_root>/<path>` and returns the actuated (non-Fixed) joints in
/// declaration order. Used by the editor to render one home-pose row
/// per joint. POST + JSON body avoids URL-encoding pain on relative
/// paths like `../foo`.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/urdf/joints",
        tag = "urdf",
        request_body = PathQuery,
        responses(
            (status = 200, description = "Actuated-joint listing", body = UrdfJointsResponse),
            (status = 400, description = "Path resolution or URDF parse error", body = ErrorEnvelope),
        ),
    )
)]
pub async fn joints(
    State(state): State<AppState>,
    Json(q): Json<PathQuery>,
) -> Result<Json<UrdfJointsResponse>, ApiError> {
    let abs = state
        .resolve_asset(&q.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let urdf = parse_urdf(&abs)
        .map_err(|e| ApiError::bad_request("urdf_parse_failed", e.to_string()))?;
    let joints = urdf
        .joints
        .iter()
        .filter(|j| !matches!(j.joint_type, JointType::Fixed))
        .map(|j| JointInfo {
            name: j.name.clone(),
            kind: match j.joint_type {
                JointType::Revolute => "revolute",
                JointType::Continuous => "continuous",
                JointType::Prismatic => "prismatic",
                JointType::Fixed => "fixed",
                JointType::Floating => "floating",
                JointType::Planar => "planar",
            }
            .to_string(),
            axis: j.axis,
            lower_deg: j.limit.as_ref().map(|l| l.lower.to_degrees()),
            upper_deg: j.limit.as_ref().map(|l| l.upper.to_degrees()),
        })
        .collect();
    Ok(Json(UrdfJointsResponse { joints }))
}
