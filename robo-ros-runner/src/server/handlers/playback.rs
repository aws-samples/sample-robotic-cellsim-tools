use axum::extract::{Path, State};
use axum::response::Json;

use crate::config::RunConfig;
use crate::playback::{build_plan, run_playback, CancelFlag, RealClock};
use crate::server::dto::{PlaybackRequest, PlaybackStartResponse, PlaybackStatusResponse};
use crate::server::error::ApiError;
use crate::server::state::{AppState, JobEntry, JobStatus};
use crate::transport::StdoutSink;

/// Build a RunConfig from the request, resolving paths under asset_root.
fn cfg_from_request(state: &AppState, req: &PlaybackRequest) -> Result<RunConfig, ApiError> {
    let cell = state
        .resolve_asset(&req.cell_path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let csv = state
        .resolve_asset(&req.csv_path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let mut cfg = RunConfig::from_paths(cell, csv);
    if let Some(v) = req.rate {
        cfg.rate = v;
    }
    if let Some(v) = req.loop_play {
        cfg.loop_play = v;
    }
    cfg.start_s = req.start_s;
    cfg.end_s = req.end_s;
    cfg.robots = req.robots.clone();
    if let Some(true) = req.clamp {
        cfg.mapping.clamp_to_limits = true;
    }
    Ok(cfg)
}

// FR-704. Start a playback job (StdoutSink in the server build — the DDS sink
// is a CLI/headless concern; see Backlog for a server-side DDS toggle).
#[utoipa::path(
    post,
    path = "/api/v1/playback",
    tag = "playback",
    request_body = PlaybackRequest,
    responses(
        (status = 200, description = "Job started", body = PlaybackStartResponse),
        (status = 400, description = "Path / parse error", body = ErrorEnvelope),
    ),
)]
pub async fn start(
    State(state): State<AppState>,
    Json(req): Json<PlaybackRequest>,
) -> Result<Json<PlaybackStartResponse>, ApiError> {
    let cfg = cfg_from_request(&state, &req)?;
    // Build the plan synchronously so request-time errors surface as 4xx.
    let plan =
        build_plan(&cfg).map_err(|e| ApiError::bad_request("plan_build_failed", e.to_string()))?;
    let topics = plan.topics.clone();

    let job_id = new_job_id();
    let cancel = CancelFlag::new();
    {
        let mut jobs = state.jobs.lock().unwrap();
        jobs.insert(
            job_id.clone(),
            JobEntry {
                status: JobStatus::Running,
                cancel: cancel.clone(),
                report: None,
                error: None,
            },
        );
    }

    // Run the (blocking) playback loop off the async runtime.
    let jobs = state.jobs.clone();
    let job_id_task = job_id.clone();
    tokio::task::spawn_blocking(move || {
        let mut sink = StdoutSink::new();
        let result = run_playback(&plan, &mut sink, &RealClock, &cancel);
        let mut jobs = jobs.lock().unwrap();
        if let Some(entry) = jobs.get_mut(&job_id_task) {
            match result {
                Ok(report) => {
                    entry.status = if cancel.is_cancelled() {
                        JobStatus::Cancelled
                    } else {
                        JobStatus::Done
                    };
                    entry.report = Some(report);
                }
                Err(e) => {
                    entry.status = JobStatus::Failed;
                    entry.error = Some(e.to_string());
                }
            }
        }
    });

    Ok(Json(PlaybackStartResponse {
        job_id,
        status: "running".to_string(),
        topics,
    }))
}

// FR-705. Cancel a running job.
#[utoipa::path(
    delete,
    path = "/api/v1/playback/{id}",
    tag = "playback",
    params(("id" = String, Path, description = "Job id")),
    responses(
        (status = 200, description = "Cancellation requested", body = PlaybackStatusResponse),
        (status = 404, description = "Unknown job", body = ErrorEnvelope),
    ),
)]
pub async fn stop(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PlaybackStatusResponse>, ApiError> {
    let jobs = state.jobs.lock().unwrap();
    let entry = jobs
        .get(&id)
        .ok_or_else(|| ApiError::not_found("unknown_job", format!("no job {}", id)))?;
    entry.cancel.cancel();
    Ok(Json(status_dto(&id, entry)))
}

// FR-706. Query job status.
#[utoipa::path(
    get,
    path = "/api/v1/playback/{id}",
    tag = "playback",
    params(("id" = String, Path, description = "Job id")),
    responses(
        (status = 200, description = "Job status", body = PlaybackStatusResponse),
        (status = 404, description = "Unknown job", body = ErrorEnvelope),
    ),
)]
pub async fn status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PlaybackStatusResponse>, ApiError> {
    let jobs = state.jobs.lock().unwrap();
    let entry = jobs
        .get(&id)
        .ok_or_else(|| ApiError::not_found("unknown_job", format!("no job {}", id)))?;
    Ok(Json(status_dto(&id, entry)))
}

fn status_dto(id: &str, entry: &JobEntry) -> PlaybackStatusResponse {
    let r = entry.report.clone().unwrap_or_default();
    PlaybackStatusResponse {
        job_id: id.to_string(),
        status: entry.status.as_str().to_string(),
        frames_published: r.frames_published,
        messages_published: r.messages_published,
        clamped_count: r.clamped_count,
        looped: r.looped,
        error: entry.error.clone(),
    }
}

fn new_job_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
