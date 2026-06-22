//! Wire types shared between handlers and clients. Every 4xx/5xx body is the
//! `ErrorEnvelope`; every 2xx body is one of the typed structs below.

use serde::{Deserialize, Serialize};

use crate::topics::ResolvedTopic;

/// Request body naming a cell manifest + optional capture, both relative to
/// `asset_root`.
#[derive(Debug, Deserialize)]
#[derive(utoipa::ToSchema)]
pub struct TopicsRequest {
    pub cell_path: String,
    #[serde(default)]
    pub csv_path: Option<String>,
}

#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct TopicsResponse {
    pub cell_name: String,
    pub topics: Vec<ResolvedTopic>,
}

/// Request body for CSV inspection.
#[derive(Debug, Deserialize)]
#[derive(utoipa::ToSchema)]
pub struct CsvInspectRequest {
    pub csv_path: String,
}

#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct CsvInspectResponse {
    pub robots: Vec<String>,
    pub axes_per_robot: usize,
    pub row_count: Option<usize>,
    pub t_min: Option<f64>,
    pub t_max: Option<f64>,
}

/// Request body to start a playback job. Paths are relative to `asset_root`.
#[derive(Debug, Deserialize)]
#[derive(utoipa::ToSchema)]
pub struct PlaybackRequest {
    pub cell_path: String,
    pub csv_path: String,
    #[serde(default)]
    pub rate: Option<f64>,
    #[serde(default)]
    pub loop_play: Option<bool>,
    #[serde(default)]
    pub start_s: Option<f64>,
    #[serde(default)]
    pub end_s: Option<f64>,
    #[serde(default)]
    pub robots: Option<Vec<String>>,
    #[serde(default)]
    pub clamp: Option<bool>,
}

#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct PlaybackStartResponse {
    pub job_id: String,
    pub status: String,
    pub topics: Vec<ResolvedTopic>,
}

#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct PlaybackStatusResponse {
    pub job_id: String,
    pub status: String,
    pub frames_published: u64,
    pub messages_published: u64,
    pub clamped_count: u64,
    pub looped: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Uniform error body for every 4xx/5xx response.
#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct ErrorEnvelope {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct HealthDto {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct ReadinessDto {
    pub ready: bool,
    pub asset_root: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}
