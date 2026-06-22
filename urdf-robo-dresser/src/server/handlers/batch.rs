//! POST /api/v1/batch — kick off a permutation merge over the entire
//! catalogue (every robot × every tool). Returns 202 + a job id; clients
//! poll `GET /api/v1/jobs/{id}` and download via
//! `GET /api/v1/jobs/{id}/artifact`.
//!
//! v1 keeps the registry in process memory — jobs do not survive a pod
//! restart. Persistence to the FSx ONTAP volume is a v1.1 follow-up if
//! batch SLAs turn out to be multi-minute and pod restarts become a real
//! concern.

use std::path::PathBuf;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use uuid::Uuid;

use crate::collision::{generate_hulls, HullBuffer};
use crate::merger::merge as lib_merge;
use crate::parser::parse_urdf;
use crate::serializer::serialize_urdf;
// `ErrorEnvelope` is referenced by the `#[utoipa::path]` macro
// expansions (response body schemas) but not by hand-written code in
// this module — utoipa's expansion lookups happen lexically.
#[allow(unused_imports)]
use crate::server::dto::ErrorEnvelope;
use crate::server::dto::{BatchAccepted, BatchRequest};
use crate::server::error::ApiError;
use crate::server::state::{AppState, JobState};
use crate::{MergeOptions, Origin};

#[utoipa::path(
    post,
    path = "/api/v1/batch",
    tag = "batch",
    request_body = BatchRequest,
    responses(
        (status = 202, description = "Batch accepted; poll the returned job_id", body = BatchAccepted),
        (status = 400, description = "Invalid hull token", body = ErrorEnvelope),
        (status = 503, description = "Catalogue empty", body = ErrorEnvelope),
    ),
)]
pub async fn start(
    State(state): State<AppState>,
    Json(req): Json<BatchRequest>,
) -> Result<Response, ApiError> {
    let hull = HullBuffer::from_str(&req.hull)
        .map_err(|e| ApiError::bad_request("invalid_hull_token", e.to_string()))?;

    let robot_dir = state.catalogue_root.join("urdf-robot-collection");
    let tool_dir = state.catalogue_root.join("urdf-tool-collection");
    if !robot_dir.exists() || !tool_dir.exists() {
        return Err(ApiError::service_unavailable(
            "catalogue_empty",
            format!(
                "expected {} and {} to exist",
                robot_dir.display(),
                tool_dir.display()
            ),
        ));
    }

    let id = Uuid::new_v4();
    {
        let mut jobs = state.jobs.write().await;
        jobs.insert(id, JobState::Queued);
    }

    let state_clone = state.clone();
    let job_id = id;
    tokio::spawn(async move { run_batch(state_clone, job_id, hull).await });

    let resp = (
        StatusCode::ACCEPTED,
        Json(BatchAccepted {
            job_id: id.to_string(),
            status_url: format!("/api/v1/jobs/{}", id),
        }),
    );
    Ok(resp.into_response())
}

async fn run_batch(state: AppState, job_id: Uuid, hull: HullBuffer) {
    {
        let mut jobs = state.jobs.write().await;
        jobs.update(&job_id, JobState::Running { progress: 0.0 });
    }

    // Collect every robot/tool URDF path in the catalogue.
    let robot_dir = state.catalogue_root.join("urdf-robot-collection");
    let tool_dir = state.catalogue_root.join("urdf-tool-collection");
    let robots: Vec<PathBuf> = collect_urdf_dirs(&robot_dir);
    let tools: Vec<PathBuf> = collect_urdf_dirs(&tool_dir);

    if robots.is_empty() || tools.is_empty() {
        let mut jobs = state.jobs.write().await;
        jobs.update(
            &job_id,
            JobState::Failed {
                message: "catalogue empty".to_string(),
            },
        );
        return;
    }

    let total = robots.len() * tools.len();
    let mut done = 0usize;
    let mut success = 0usize;
    let mut failed = 0usize;

    // All output dirs go under one job-scoped temp directory; the artifact
    // we hand back is a zip of the whole tree.
    let job_root = std::env::temp_dir().join(format!("urdf-robo-dresser-batch-{}", job_id));
    if let Err(e) = std::fs::create_dir_all(&job_root) {
        let mut jobs = state.jobs.write().await;
        jobs.update(
            &job_id,
            JobState::Failed {
                message: format!("create job_root: {}", e),
            },
        );
        return;
    }

    for robot_path in &robots {
        let base = match parse_urdf(robot_path) {
            Ok(r) => r,
            Err(_) => {
                failed += 1;
                done += 1;
                continue;
            }
        };
        let robot_name = robot_path.file_stem().unwrap().to_string_lossy().to_string();
        let robot_urdf_dir = robot_path.parent().unwrap().to_path_buf();

        for tool_path in &tools {
            done += 1;
            let tool = match parse_urdf(tool_path) {
                Ok(r) => r,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };
            let tool_name = tool_path
                .parent()
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string();
            let combo_name = format!("{}__{}", robot_name, tool_name);
            let combo_dir = job_root.join(&combo_name);
            if std::fs::create_dir_all(&combo_dir).is_err() {
                failed += 1;
                continue;
            }

            let tool_urdf_dir = tool_path.parent().unwrap().to_path_buf();
            let mut tool_with_hulls = tool.clone();
            let _ = generate_hulls(&mut tool_with_hulls, &tool_urdf_dir, &combo_dir, "tool", hull);
            let mut base_with_hulls = base.clone();
            let _ = generate_hulls(&mut base_with_hulls, &robot_urdf_dir, &combo_dir, "robot", hull);

            let opts = MergeOptions {
                attach_to: None,
                orient: [0, 0, 0],
                offset: Origin::default(),
                tool_prefix: "tool__".to_string(),
                output_dir: combo_dir.clone(),
                robot_urdf_dir: robot_urdf_dir.clone(),
                tool_urdf_dir,
            };
            match lib_merge(&base_with_hulls, &tool_with_hulls, &opts) {
                Ok(dressed) => {
                    let xml = serialize_urdf(&dressed);
                    let out_file = combo_dir.join(format!("{}.urdf", combo_name));
                    if std::fs::write(&out_file, &xml).is_ok() {
                        success += 1;
                    } else {
                        failed += 1;
                    }
                }
                Err(_) => failed += 1,
            }

            let progress = done as f32 / total as f32;
            let mut jobs = state.jobs.write().await;
            jobs.update(&job_id, JobState::Running { progress });
        }
    }

    // Zip the whole tree.
    let zip_path = job_root.with_extension("zip");
    if let Err(e) = zip_dir(&job_root, &zip_path) {
        let mut jobs = state.jobs.write().await;
        jobs.update(
            &job_id,
            JobState::Failed {
                message: format!("zip failed: {}", e),
            },
        );
        return;
    }

    let mut jobs = state.jobs.write().await;
    jobs.update(
        &job_id,
        JobState::Completed {
            artifact_path: zip_path,
            success,
            failed,
        },
    );
}

fn collect_urdf_dirs(dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let urdf = e.path().join(format!("{}.urdf", name));
                    urdf.exists().then_some(urdf)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn zip_dir(src: &std::path::Path, out: &std::path::Path) -> std::io::Result<()> {
    let zip_file = std::fs::File::create(out)?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let opts: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    walk_dir(src, src, &mut zip, opts)?;
    zip.finish()
        .map_err(|e| std::io::Error::other(format!("zip finalize: {}", e)))?;
    Ok(())
}

fn walk_dir(
    base: &std::path::Path,
    cur: &std::path::Path,
    zip: &mut zip::ZipWriter<std::fs::File>,
    opts: zip::write::SimpleFileOptions,
) -> std::io::Result<()> {
    use std::io::Write;
    for entry in std::fs::read_dir(cur)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path.strip_prefix(base).unwrap_or(&path);
        if path.is_dir() {
            walk_dir(base, &path, zip, opts)?;
        } else if path.is_file() {
            zip.start_file(rel.to_string_lossy(), opts)
                .map_err(|e| std::io::Error::other(format!("zip start_file: {}", e)))?;
            let bytes = std::fs::read(&path)?;
            zip.write_all(&bytes)?;
        }
    }
    Ok(())
}
