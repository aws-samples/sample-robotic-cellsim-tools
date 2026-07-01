use std::path::PathBuf;

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};

use crate::compose::{compose_stage_with_options, compose_urdf_bundle, ComposeOptions};
use crate::scene::{parse_scene, parse_scene_from_yaml, validate_scene};
use crate::server::dto::ValidateResponse;
use crate::server::error::ApiError;
use crate::server::state::AppState;

/// Query parameters for compose: matches the CLI flags one-for-one.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct ComposeQuery {
    /// Path under asset_root pointing at an existing manifest YAML.
    pub manifest_path: String,
    #[serde(default)]
    pub full_robots: bool,
    #[serde(default = "yes")]
    pub full_tools: bool,
}

fn yes() -> bool {
    true
}

/// POST /api/v1/validate — body is the raw manifest YAML
/// (`application/x-yaml` or `text/plain`). Returns `{ok, errors[]}`. The
/// 200 response is used both for valid and invalid manifests; the
/// `errors` array is the success/fail signal. Path resolution failures
/// (i.e. URDFs referenced by the manifest can't be found under
/// `<asset_root>/fitted-robots/`) populate `errors` rather than 4xx.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/validate",
        tag = "compose",
        request_body(content = String, content_type = "application/x-yaml"),
        responses(
            (status = 200, description = "Validation result", body = ValidateResponse),
        ),
    )
)]
pub async fn validate(
    State(state): State<AppState>,
    body: String,
) -> Result<Json<ValidateResponse>, ApiError> {
    // Parse the YAML in memory. Resolve relative paths against
    // `<asset_root>/fitted-robots/` because that's the canonical
    // location for scene manifests in this project — paths like
    // `../floor-plans/foo.obj` reach back into asset_root from there.
    // Same convention compose's on-disk parse_scene uses.
    let scene_dir = state.asset_root.join("fitted-robots");
    let parse_res = parse_scene_from_yaml(&body, &scene_dir);
    let scene = match parse_res {
        Ok(s) => s,
        Err(e) => {
            return Ok(Json(ValidateResponse {
                ok: false,
                errors: vec![e.to_string()],
            }));
        }
    };
    let errors = validate_scene(&scene)
        .into_iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    Ok(Json(ValidateResponse {
        ok: errors.is_empty(),
        errors,
    }))
}

/// POST /api/v1/compose?manifest_path=...&full_robots=&full_tools= —
/// run the existing compose pipeline against a manifest already on disk
/// under `asset_root`. The composed `.zip` is streamed back as the
/// response body with Content-Type `application/zip`.
///
/// The editor's "Generate USDA" button:
///   1. POSTs the in-memory YAML to `/api/v1/scenes/save?path=…`
///   2. POSTs to `/api/v1/compose?manifest_path=…` and downloads the response
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/compose",
        tag = "compose",
        params(ComposeQuery),
        responses(
            (status = 200, description = "Composed scene zip", content_type = "application/zip"),
            (status = 400, description = "Manifest path / validation failed", body = ErrorEnvelope),
            (status = 500, description = "Compose pipeline failure", body = ErrorEnvelope),
        ),
    )
)]
pub async fn compose_zip(
    State(state): State<AppState>,
    Query(q): Query<ComposeQuery>,
) -> Result<Response, ApiError> {
    let manifest_abs = state
        .resolve_asset(&q.manifest_path)
        .map_err(|e| ApiError::bad_request("invalid_manifest_path", e.to_string()))?;
    let scene = parse_scene(&manifest_abs)
        .map_err(|e| ApiError::bad_request("manifest_parse_failed", e.to_string()))?;
    let errs = validate_scene(&scene);
    if !errs.is_empty() {
        let joined = errs
            .into_iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(
            ApiError::bad_request("manifest_invalid", joined)
                .with_hint("POST the manifest body to /api/v1/validate to inspect each error"),
        );
    }

    // Write the zip to a temp file adjacent to the manifest, then read it
    // back into memory for the response. Adjacent placement keeps relative
    // asset references working at compose time.
    let stem = manifest_abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scene");
    let zip_path: PathBuf = manifest_abs
        .with_file_name(format!(".__compose_{}.zip", stem));
    let opts = ComposeOptions {
        full_robots: q.full_robots,
        full_tools: q.full_tools,
    };
    compose_stage_with_options(&scene, &manifest_abs, &zip_path, &opts)
        .map_err(|e| ApiError::internal("compose_failed", e.to_string()))?;
    let bytes = tokio::fs::read(&zip_path)
        .await
        .map_err(|e| ApiError::internal("zip_read_failed", e.to_string()))?;
    let _ = tokio::fs::remove_file(&zip_path).await;

    let download_name = format!("{}.zip", stem);
    let body = Bytes::from(bytes);
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", download_name),
        )
        .body(body.into_response().into_body())
        .map_err(|e| ApiError::internal("response_build_failed", e.to_string()))?;
    Ok(resp)
}

/// Query parameters for the URDF-source bundle. The compose fidelity flags
/// (`full_robots`/`full_tools`) are USDA-only knobs and don't apply here, so
/// this endpoint takes just the manifest path.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema, utoipa::IntoParams)]
pub struct UrdfBundleQuery {
    /// Path under asset_root pointing at an existing manifest YAML.
    pub manifest_path: String,
}

/// POST /api/v1/compose-urdf-bundle?manifest_path=... — build the
/// self-contained URDF-source bundle for a manifest already on disk under
/// `asset_root` and stream it back as `application/zip`. The bundle holds the
/// scene YAML (rewritten to reference the URDFs by relative path) plus every
/// source URDF copied verbatim (each in its own folder with its `meshes/`
/// subtree) and the floor-plan OBJ when present. This is a SEPARATE artifact
/// from `/compose`; the editor calls both when the user opts in.
#[cfg_attr(
    feature = "server",
    utoipa::path(
        post,
        path = "/api/v1/compose-urdf-bundle",
        tag = "compose",
        params(UrdfBundleQuery),
        responses(
            (status = 200, description = "URDF-source bundle zip", content_type = "application/zip"),
            (status = 400, description = "Manifest path / validation failed", body = ErrorEnvelope),
            (status = 500, description = "Bundle build failure", body = ErrorEnvelope),
        ),
    )
)]
pub async fn compose_urdf_bundle_zip(
    State(state): State<AppState>,
    Query(q): Query<UrdfBundleQuery>,
) -> Result<Response, ApiError> {
    let manifest_abs = state
        .resolve_asset(&q.manifest_path)
        .map_err(|e| ApiError::bad_request("invalid_manifest_path", e.to_string()))?;
    let scene = parse_scene(&manifest_abs)
        .map_err(|e| ApiError::bad_request("manifest_parse_failed", e.to_string()))?;
    let errs = validate_scene(&scene);
    if !errs.is_empty() {
        let joined = errs
            .into_iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(
            ApiError::bad_request("manifest_invalid", joined)
                .with_hint("POST the manifest body to /api/v1/validate to inspect each error"),
        );
    }

    // Temp file adjacent to the manifest so relative asset references resolve.
    let stem = manifest_abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scene");
    let zip_path: PathBuf = manifest_abs.with_file_name(format!(".__urdf_bundle_{}.zip", stem));
    compose_urdf_bundle(&scene, &manifest_abs, &zip_path)
        .map_err(|e| ApiError::internal("bundle_failed", e.to_string()))?;
    let bytes = tokio::fs::read(&zip_path)
        .await
        .map_err(|e| ApiError::internal("zip_read_failed", e.to_string()))?;
    let _ = tokio::fs::remove_file(&zip_path).await;

    let download_name = format!("{}-urdf.zip", stem);
    let body = Bytes::from(bytes);
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/zip")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", download_name),
        )
        .body(body.into_response().into_body())
        .map_err(|e| ApiError::internal("response_build_failed", e.to_string()))?;
    Ok(resp)
}
