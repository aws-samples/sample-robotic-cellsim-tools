use std::path::Path;

use axum::extract::{Multipart, State};
use axum::response::Json;

use crate::server::dto::{AssetEntry, FloorPlanEntry, UploadResponse};
use crate::server::error::ApiError;
use crate::server::state::AppState;

/// GET /api/v1/assets/robots — list every dressed-robot URDF under
/// `<asset_root>/urdf-dressed-robots/<id>/<id>.urdf`.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        get,
        path = "/api/v1/assets/robots",
        tag = "assets",
        responses(
            (status = 200, description = "Dressed-robot catalogue listing", body = [AssetEntry]),
        ),
    )
)]
pub async fn list_robots(State(state): State<AppState>) -> Json<Vec<AssetEntry>> {
    let dir = state.asset_root.join("urdf-dressed-robots");
    Json(list_urdf_subdirs(&dir))
}

/// GET /api/v1/assets/fixtures — same shape, under `urdf-fixture-collection/`.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        get,
        path = "/api/v1/assets/fixtures",
        tag = "assets",
        responses(
            (status = 200, description = "Fixture catalogue listing", body = [AssetEntry]),
        ),
    )
)]
pub async fn list_fixtures(State(state): State<AppState>) -> Json<Vec<AssetEntry>> {
    let dir = state.asset_root.join("urdf-fixture-collection");
    Json(list_urdf_subdirs(&dir))
}

/// GET /api/v1/assets/floor-plans — flat list of `*.obj` / `*.dxf` under
/// `<asset_root>/floor-plans/`.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        get,
        path = "/api/v1/assets/floor-plans",
        tag = "assets",
        responses(
            (status = 200, description = "Floor-plan catalogue listing", body = [FloorPlanEntry]),
        ),
    )
)]
pub async fn list_floor_plans(State(state): State<AppState>) -> Json<Vec<FloorPlanEntry>> {
    let dir = state.asset_root.join("floor-plans");
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for ent in rd.flatten() {
            let path = ent.path();
            if !path.is_file() {
                continue;
            }
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if !matches!(ext.as_str(), "obj" | "dxf") {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                out.push(FloorPlanEntry {
                    name: name.to_string(),
                    ext,
                    rel: format!("floor-plans/{}", name),
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Json(out)
}

/// POST /api/v1/assets/floor-plans/upload-obj — multipart upload, stores
/// the file under `<asset_root>/floor-plans/<filename>`. Returns the
/// relative path the manifest can reference.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/assets/floor-plans/upload-obj",
        tag = "assets",
        request_body(
            content = String,
            content_type = "multipart/form-data",
            description = "Form fields: `filename` (text) and `file` (the OBJ payload).",
        ),
        responses(
            (status = 200, description = "Upload accepted", body = UploadResponse),
            (status = 400, description = "Missing filename / non-OBJ extension", body = ErrorEnvelope),
            (status = 500, description = "Filesystem write failed", body = ErrorEnvelope),
        ),
    )
)]
pub async fn upload_obj(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, ApiError> {
    let dir = state.asset_root.join("floor-plans");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError::internal("floor_plans_mkdir_failed", e.to_string()))?;

    let mut filename: Option<String> = None;
    let mut bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request("multipart_invalid", e.to_string()))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "filename" {
            let v = field
                .text()
                .await
                .map_err(|e| ApiError::bad_request("multipart_invalid", e.to_string()))?;
            filename = Some(sanitize_filename(&v));
        } else if name == "file" {
            if filename.is_none() {
                if let Some(fname) = field.file_name() {
                    filename = Some(sanitize_filename(fname));
                }
            }
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request("multipart_invalid", e.to_string()))?;
            bytes = Some(data.to_vec());
        }
    }

    let filename = filename
        .ok_or_else(|| ApiError::bad_request("missing_filename", "form field `filename` is required"))?;
    let bytes = bytes
        .ok_or_else(|| ApiError::bad_request("missing_file_body", "form field `file` is required"))?;

    if !filename.to_ascii_lowercase().ends_with(".obj") {
        return Err(ApiError::bad_request(
            "unsupported_extension",
            "uploaded file must have a .obj extension",
        ));
    }

    let target = dir.join(&filename);
    tokio::fs::write(&target, &bytes)
        .await
        .map_err(|e| ApiError::internal("write_failed", e.to_string()))?;
    Ok(Json(UploadResponse {
        stored_path: format!("floor-plans/{}", filename),
        bytes: bytes.len() as u64,
    }))
}

fn list_urdf_subdirs(root: &Path) -> Vec<AssetEntry> {
    let mut out = Vec::new();
    let rd = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    for ent in rd.flatten() {
        let p = ent.path();
        if !p.is_dir() {
            continue;
        }
        let id = match p.file_name().and_then(|n| n.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let urdf = p.join(format!("{}.urdf", id));
        if !urdf.exists() {
            continue;
        }
        // The root segment determines the prefix used by the manifest.
        // We re-derive it from the parent dir's name for symmetry with how
        // the editor refers back to these entries.
        let parent_label = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("urdf-dressed-robots");
        out.push(AssetEntry {
            id: id.clone(),
            urdf_rel: format!("{}/{}/{}.urdf", parent_label, id, id),
        });
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Strip any path components from a user-supplied filename. Only allow
/// alphanumeric / dot / underscore / dash; reject everything else.
fn sanitize_filename(name: &str) -> String {
    let last = std::path::Path::new(name)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(name);
    last.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}
