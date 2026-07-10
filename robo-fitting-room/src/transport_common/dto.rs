//! Wire types shared between transports and the SPA.
//!
//! These structs derive plain `serde` `Serialize`/`Deserialize`
//! unconditionally; only the `utoipa::ToSchema` derive is `server`-feature
//! gated (the `stdio` transport never generates an OpenAPI document). They
//! live here in `transport_common` — compiled whenever either the
//! `server` or the `stdio` feature is on — so both the HTTP handlers and
//! the stdio dispatch table reuse the exact same shapes with zero
//! duplication. `server::dto` re-exports them unchanged, so existing
//! `crate::server::dto::*` call sites keep working without modification.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct AssetEntry {
    pub id: String,
    pub urdf_rel: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct FloorPlanEntry {
    pub name: String,
    pub ext: String,
    pub rel: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct JointInfo {
    pub name: String,
    /// "revolute" / "continuous" / "prismatic" — non-Fixed only.
    pub kind: String,
    pub axis: [f64; 3],
    pub lower_deg: Option<f64>,
    pub upper_deg: Option<f64>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct UrdfJointsResponse {
    pub joints: Vec<JointInfo>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct FloorPlanSegments {
    /// Each segment is [[x, y, z], [x, y, z]].
    pub segments: Vec<[[f64; 3]; 2]>,
    pub footprint_segments: Vec<[[f64; 3]; 2]>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct ValidateResponse {
    pub ok: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct PathQuery {
    pub path: String,
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct ScenePathBody {
    pub path: String,
    pub yaml: String,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct UploadResponse {
    pub stored_path: String,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct SaveResponse {
    pub ok: bool,
    pub bytes: u64,
}

/// Liveness probe response. `GET /healthz` always returns 200 with this.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct HealthDto {
    pub status: String,
    pub version: String,
}

/// Readiness probe response. `GET /readyz` returns 200 with `ready: true`
/// when the asset_root canonicalises and at least one URDF is discoverable;
/// otherwise 503 with `ready: false` and a populated `errors` list.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "server", derive(utoipa::ToSchema))]
pub struct ReadinessDto {
    pub ready: bool,
    pub asset_root: String,
    pub urdf_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}
