//! Wire types shared between handlers and external clients. Every type
//! carries `utoipa::ToSchema` so it's reflected in the published OpenAPI
//! spec — the same schema-as-contract discipline the fitting-room uses.
//!
//! These are *the* DTOs; handlers must never return inline JSON or bare
//! strings. Schema changes are enforced via the
//! `tests/openapi_snapshot.rs` test against the checked-in `openapi.json`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---- Catalogue listings ----

#[derive(Debug, Serialize, ToSchema)]
pub struct CatalogueListing {
    pub robots: Vec<RobotEntry>,
    pub tools: Vec<ToolEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RobotEntry {
    pub name: String,
    pub flange_link: String,
    pub joint_count: usize,
    pub link_count: usize,
    pub status: AssetStatusDto,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ToolEntry {
    pub name: String,
    pub visual_count: usize,
    pub status: AssetStatusDto,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssetStatusDto {
    Valid,
    Incomplete { issues: Vec<String> },
    Error { message: String },
}

// ---- Dress / merge ----

#[derive(Debug, Deserialize, ToSchema)]
pub struct DressRequest {
    /// Catalogue ID for the robot — corresponds to `<catalogue>/urdf-robot-collection/<robot_id>/<robot_id>.urdf`.
    pub robot_id: String,
    /// Catalogue ID for the tool — corresponds to `<catalogue>/urdf-tool-collection/<tool_id>/<tool_id>.urdf`.
    pub tool_id: String,
    /// Override the auto-detected flange link.
    #[serde(default)]
    pub attach_to: Option<String>,
    /// Tool orientation in 90° steps as `[x, y, z]` degrees. Each value
    /// must be 0, 90, 180, or 270.
    #[serde(default = "default_orient")]
    pub orient: [u16; 3],
    /// Collision hull buffer size: `tight` / `medium` (default) / `max`.
    #[serde(default = "default_hull")]
    pub hull: String,
}

fn default_orient() -> [u16; 3] {
    [0, 0, 0]
}
fn default_hull() -> String {
    "medium".into()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DressResponse {
    /// True iff the dressed URDF + meshes are written to disk under
    /// `<catalogue>/urdf-dressed-robots/<robot_id>__<tool_id>/`.
    pub ok: bool,
    /// Catalogue-relative path of the resulting dressed URDF.
    pub dressed_urdf_rel: String,
    /// Inline-listed validation diagnostics from `validator::validate`
    /// against both inputs. Always present so clients can surface
    /// non-blocking warnings.
    pub diagnostics: Vec<DiagnosticDto>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DiagnosticDto {
    pub severity: String, // "error" | "warning"
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

// ---- Validate ----

#[derive(Debug, Deserialize, ToSchema)]
pub struct ValidateRequest {
    /// Catalogue-relative path to the URDF to validate.
    pub path: String,
    /// True to validate as a tool (extra origin alignment checks);
    /// false for a robot (default).
    #[serde(default)]
    pub tool: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ValidateResponse {
    pub ok: bool,
    pub diagnostics: Vec<DiagnosticDto>,
}

// ---- Batch ----

#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRequest {
    /// Collision hull buffer to use for every permutation.
    #[serde(default = "default_hull")]
    pub hull: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BatchAccepted {
    pub job_id: String,
    /// Convenience pointer for the polling endpoint
    /// (`GET /api/v1/jobs/{id}`). Clients are free to construct this
    /// themselves.
    pub status_url: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct JobStatus {
    pub job_id: String,
    /// `queued` | `running` | `completed` | `failed`.
    pub state: String,
    /// 0.0..1.0 best-effort progress while `state == "running"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<f32>,
    /// Number of successful permutations (only set when `completed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<usize>,
    /// Number of failed permutations (only set when `completed`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<usize>,
    /// Failure message if the job state is `failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Artifact download URL (`GET /api/v1/jobs/{id}/artifact`) once
    /// the job is `completed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifact_url: Option<String>,
}

// ---- System ----

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthDto {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadinessDto {
    pub ready: bool,
    pub catalogue_root: String,
    pub robot_count: usize,
    pub tool_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// Uniform error body. Same shape as fitting-room's so cross-service
/// clients see a consistent contract.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}
