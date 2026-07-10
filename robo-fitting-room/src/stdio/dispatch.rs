//! Method dispatch for the `stdio` transport.
//!
//! [`dispatch`] is a total function: every method name recognized by the
//! API surface maps to exactly one arm; anything else is a protocol-level
//! "method not found" error, not a panic. Each handler below is a direct,
//! mechanical translation of the equivalent axum handler in
//! `src/server/handlers/*.rs` — same sandboxing (`state.resolve_asset`),
//! same library calls, same validation order — with `Json<T>` extraction
//! replaced by `from_params::<T>(params)` (deserialize-or-`invalid_params`)
//! and `Result<Json<T>, ApiError>` replaced by `Result<T, ApiError>`.
//!
//! The synchronous `std::fs` calls here stand in for the axum handlers'
//! `tokio::fs` awaits: the stdio loop is single-threaded with no async
//! runtime, and every underlying library call (`parse_urdf`,
//! `parse_scene_from_yaml`, `validate_scene`, `load_floor_plan`) is already
//! synchronous.

use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use urdf_robo_dresser::{parse_urdf, JointType};

use crate::compose::{compose_stage_with_options, compose_urdf_bundle, ComposeOptions};
use crate::floor_plan::load_floor_plan;
use crate::scene::{parse_scene, parse_scene_from_yaml, validate_scene};
use crate::stdio::models::{ComposeArtifact, SceneLoadResult};
use crate::stdio::protocol::{RpcError, RpcErrorData};
use crate::transport_common::dto::{
    AssetEntry, FloorPlanEntry, FloorPlanSegments, HealthDto, JointInfo, PathQuery, ReadinessDto,
    SaveResponse, ScenePathBody, UploadResponse, UrdfJointsResponse, ValidateResponse,
};
use crate::transport_common::{ApiError, AppState};

/// Process-local counter ensuring temp-file names for composed artifacts are
/// unique across every call within a single running binary. Combined with
/// `process::id()` in the filename, this also rules out collisions between
/// concurrently running binary invocations.
static COMPOSE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Total dispatch: map a method name + params to a JSON result or an
/// `RpcError`. Never panics; an unknown method is a `method_not_found`
/// error, not a crash.
pub fn dispatch(state: &AppState, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "health.check" => ok(health_check()),
        "health.ready" => ok(health_ready(state)),
        "assets.listRobots" => ok(assets_list_robots(state)),
        "assets.listFixtures" => ok(assets_list_fixtures(state)),
        "assets.listFloorPlans" => ok(assets_list_floor_plans(state)),
        "assets.uploadFloorPlanObj" => to_rpc(assets_upload(state, from_params(params)?)),
        "urdf.joints" => to_rpc(urdf_joints(state, from_params(params)?)),
        "floorPlan.segments" => to_rpc(floor_plan_segments(state, from_params(params)?)),
        "compose.validate" => ok(compose_validate(state, from_params(params)?)),
        "compose.run" => to_rpc(compose_run(state, from_params(params)?)),
        "compose.urdfBundle" => to_rpc(compose_urdf_bundle_run(state, from_params(params)?)),
        "scene.load" => to_rpc(scene_load(state, from_params(params)?)),
        "scene.save" => to_rpc(scene_save(state, from_params(params)?)),
        other => Err(RpcError::method_not_found(other)),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Deserialize a method's `params` into the expected shape, mapping any
/// deserialization failure to an `invalid_params` protocol error
/// (code -32602, data code `"invalid_params"`).
pub(crate) fn from_params<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    serde_json::from_value(params).map_err(|e| RpcError {
        code: RpcError::CODE_INVALID_PARAMS,
        message: format!("Invalid params: {e}"),
        data: Some(RpcErrorData {
            code: "invalid_params".to_string(),
            hint: None,
        }),
    })
}

/// Serialize a handler's success value into a JSON `Value`. Serialization
/// of these plain data structs can't realistically fail, but the error is
/// mapped rather than unwrapped so `dispatch` stays panic-free.
pub(crate) fn ok<T: Serialize>(value: T) -> Result<Value, RpcError> {
    serde_json::to_value(value).map_err(|e| RpcError {
        code: 500,
        message: format!("Failed to serialize response: {e}"),
        data: Some(RpcErrorData {
            code: "serialization_failed".to_string(),
            hint: None,
        }),
    })
}

/// Bridge an `ApiError`-returning handler onto the RPC result type: serialize
/// the success payload, or project the `ApiError` onto an `RpcError` via the
/// existing `From<ApiError>` mapping.
pub(crate) fn to_rpc<T: Serialize>(result: Result<T, ApiError>) -> Result<Value, RpcError> {
    match result {
        Ok(value) => ok(value),
        Err(err) => Err(RpcError::from(err)),
    }
}

// ---------------------------------------------------------------------------
// health.* — mirror src/server/handlers/health.rs
// ---------------------------------------------------------------------------

/// `health.check` — always-ok liveness, mirroring `GET /healthz`. The
/// `version` field is sourced from `CARGO_PKG_VERSION` at compile time.
fn health_check() -> HealthDto {
    HealthDto {
        status: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

/// `health.ready` — readiness, mirroring `GET /readyz`. The HTTP handler
/// distinguishes ready/not-ready via 200 vs 503; over stdio the not-ready
/// case is folded into the normal result (`ready: false` + populated
/// `errors`), so this returns a `ReadinessDto` directly rather than a
/// `Result`.
fn health_ready(state: &AppState) -> ReadinessDto {
    let asset_root = state.asset_root.as_path().to_path_buf();
    let asset_root_str = asset_root.display().to_string();

    let canon = match std::fs::canonicalize(&asset_root) {
        Ok(c) => c,
        Err(e) => {
            return ReadinessDto {
                ready: false,
                asset_root: asset_root_str,
                urdf_count: 0,
                errors: vec![format!("asset_root canonicalize: {}", e)],
            };
        }
    };

    let urdf_count = count_urdfs(&canon, 5);

    if urdf_count == 0 {
        return ReadinessDto {
            ready: false,
            asset_root: asset_root_str,
            urdf_count: 0,
            errors: vec!["no *.urdf found under asset_root (depth 4)".to_string()],
        };
    }

    ReadinessDto {
        ready: true,
        asset_root: asset_root_str,
        urdf_count,
        errors: vec![],
    }
}

/// Recursively count `*.urdf` files under `dir` up to `depth` levels deep.
/// Bounded so a misconfigured asset_root pointing at a giant filesystem
/// doesn't stall the readiness probe. Verbatim from the HTTP handler.
fn count_urdfs(dir: &Path, depth: u32) -> usize {
    if depth == 0 {
        return 0;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.eq_ignore_ascii_case("urdf"))
                .unwrap_or(false)
        {
            count += 1;
        } else if path.is_dir() {
            count += count_urdfs(&path, depth - 1);
        }
    }
    count
}

// ---------------------------------------------------------------------------
// assets.* — mirror src/server/handlers/assets.rs
// ---------------------------------------------------------------------------

/// `assets.listRobots` — list dressed-robot URDFs under
/// `<asset_root>/urdf-dressed-robots/<id>/<id>.urdf`.
fn assets_list_robots(state: &AppState) -> Vec<AssetEntry> {
    let dir = state.asset_root.join("urdf-dressed-robots");
    list_urdf_subdirs(&dir)
}

/// `assets.listFixtures` — same shape, under `urdf-fixture-collection/`.
fn assets_list_fixtures(state: &AppState) -> Vec<AssetEntry> {
    let dir = state.asset_root.join("urdf-fixture-collection");
    list_urdf_subdirs(&dir)
}

/// `assets.listFloorPlans` — flat list of `*.obj` / `*.dxf` under
/// `<asset_root>/floor-plans/`.
fn assets_list_floor_plans(state: &AppState) -> Vec<FloorPlanEntry> {
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
    out
}

/// Params for `assets.uploadFloorPlanObj`. Over HTTP the payload arrives as
/// `multipart/form-data`; over stdio the single user-picked OBJ is carried
/// base64-encoded in the JSON request (a deliberate, bounded exception to
/// the "no inline binary" rule — see design Requirement 4.5). `dataBase64`
/// on the wire.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadParams {
    filename: String,
    data_base64: String,
}

/// `assets.uploadFloorPlanObj` — decode the base64 OBJ payload and store it
/// under `<asset_root>/floor-plans/<filename>`. Mirrors the HTTP handler's
/// validation order: ensure the target dir exists, sanitize + require a
/// `.obj` filename, then write.
fn assets_upload(state: &AppState, params: UploadParams) -> Result<UploadResponse, ApiError> {
    let dir = state.asset_root.join("floor-plans");
    std::fs::create_dir_all(&dir)
        .map_err(|e| ApiError::internal("floor_plans_mkdir_failed", e.to_string()))?;

    let filename = sanitize_filename(&params.filename);
    if !filename.to_ascii_lowercase().ends_with(".obj") {
        return Err(ApiError::bad_request(
            "unsupported_extension",
            "uploaded file must have a .obj extension",
        ));
    }

    let bytes = base64_decode(&params.data_base64)
        .map_err(|e| ApiError::bad_request("invalid_base64", e))?;

    let target = dir.join(&filename);
    std::fs::write(&target, &bytes)
        .map_err(|e| ApiError::internal("write_failed", e.to_string()))?;
    Ok(UploadResponse {
        stored_path: format!("floor-plans/{}", filename),
        bytes: bytes.len() as u64,
    })
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
/// alphanumeric / dot / underscore / dash; reject everything else. Verbatim
/// from the HTTP handler.
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

/// Minimal standard-alphabet (RFC 4648) base64 decoder. The `stdio` feature
/// is deliberately dependency-slim (serde_json + http only), so rather than
/// pull in the `base64` crate for the single upload path this decodes
/// inline. Whitespace and `=` padding are ignored; any other
/// out-of-alphabet byte is an error.
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    fn sextet(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut buf: u32 = 0;
    let mut bits: u32 = 0;
    for &c in input.as_bytes() {
        match c {
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => {}
        }
        let v = sextet(c).ok_or_else(|| format!("invalid base64 character: {:?}", c as char))?;
        buf = (buf << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// urdf.* — mirror src/server/handlers/urdf.rs
// ---------------------------------------------------------------------------

/// `urdf.joints` — parse the URDF at `<asset_root>/<path>` and return the
/// actuated (non-Fixed) joints in declaration order.
fn urdf_joints(state: &AppState, q: PathQuery) -> Result<UrdfJointsResponse, ApiError> {
    let abs = state
        .resolve_asset(&q.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let urdf =
        parse_urdf(&abs).map_err(|e| ApiError::bad_request("urdf_parse_failed", e.to_string()))?;
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
    Ok(UrdfJointsResponse { joints })
}

// ---------------------------------------------------------------------------
// floorPlan.* — mirror src/server/handlers/floor_plan.rs
// ---------------------------------------------------------------------------

/// `floorPlan.segments` — parse the OBJ at the given relative path through
/// `floor_plan::load_floor_plan` and return the segment arrays. Offset and
/// scale are identity, exactly as the HTTP handler does.
fn floor_plan_segments(state: &AppState, q: PathQuery) -> Result<FloorPlanSegments, ApiError> {
    let abs = state
        .resolve_asset(&q.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let fp = load_floor_plan(&abs, [0.0; 3], 1.0)
        .map_err(|e| ApiError::bad_request("floor_plan_parse_failed", e.to_string()))?;
    Ok(FloorPlanSegments {
        segments: fp.segments,
        footprint_segments: fp.footprint_segments,
    })
}

// ---------------------------------------------------------------------------
// compose.validate — mirror src/server/handlers/compose.rs::validate
// ---------------------------------------------------------------------------

/// Params for `compose.validate`: the raw manifest YAML. Over HTTP this
/// arrives as the request body; over stdio it's a `{yaml}` JSON field.
#[derive(Debug, Deserialize)]
struct ValidateParams {
    yaml: String,
}

/// `compose.validate` — parse the manifest YAML in memory (resolving
/// relative paths against `<asset_root>/fitted-robots/`, the same
/// convention the HTTP handler uses) and validate it. Parse/validation
/// failures populate `errors` rather than producing an RPC error, matching
/// the HTTP handler's always-200 contract — hence `ok(...)` in `dispatch`.
fn compose_validate(state: &AppState, params: ValidateParams) -> ValidateResponse {
    let scene_dir = state.asset_root.join("fitted-robots");
    let scene = match parse_scene_from_yaml(&params.yaml, &scene_dir) {
        Ok(s) => s,
        Err(e) => {
            return ValidateResponse {
                ok: false,
                errors: vec![e.to_string()],
            };
        }
    };
    let errors = validate_scene(&scene)
        .into_iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>();
    ValidateResponse {
        ok: errors.is_empty(),
        errors,
    }
}

// ---------------------------------------------------------------------------
// compose.run / compose.urdfBundle — mirror
// src/server/handlers/compose.rs::{compose_zip, compose_urdf_bundle_zip}
// ---------------------------------------------------------------------------

/// Params for `compose.run`. Mirrors the HTTP `ComposeQuery`: `manifestPath`
/// under `asset_root` plus the two USDA fidelity flags. Defaults match the
/// HTTP handler (`full_robots` false, `full_tools` true) so behavior is
/// identical when a caller omits them.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComposeRunParams {
    manifest_path: String,
    #[serde(default)]
    full_robots: bool,
    #[serde(default = "default_full_tools")]
    full_tools: bool,
}

fn default_full_tools() -> bool {
    true
}

/// Params for `compose.urdfBundle`. Mirrors the HTTP `UrdfBundleQuery`: the
/// fidelity flags are USDA-only knobs that don't apply, so this takes just
/// the manifest path.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UrdfBundleParams {
    manifest_path: String,
}

/// Build a unique temp path for a composed artifact:
/// `<temp_dir>/rfr-compose-{pid}-{stem}-{n}.zip`, where `n` is drawn from a
/// process-local atomic counter. `process::id()` disambiguates concurrent
/// binary invocations; the counter disambiguates calls within one process.
fn unique_zip_path(stem: &str) -> std::path::PathBuf {
    let counter = COMPOSE_COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rfr-compose-{}-{}-{}.zip",
        process::id(),
        stem,
        counter
    ))
}

/// `compose.run` — mirror `POST /api/v1/compose`. Same sandboxing, parse,
/// and validation order as the HTTP handler, but instead of reading the zip
/// back into memory and deleting it, the archive is left on disk and its
/// path returned in a `ComposeArtifact`. Ownership of the temp file (moving
/// or deleting it) transfers to the caller.
fn compose_run(state: &AppState, params: ComposeRunParams) -> Result<ComposeArtifact, ApiError> {
    let manifest_abs = state
        .resolve_asset(&params.manifest_path)
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
        return Err(ApiError::bad_request("manifest_invalid", joined)
            .with_hint("call compose.validate to inspect each error"));
    }

    let stem = manifest_abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scene");
    let zip_path = unique_zip_path(stem);
    let opts = ComposeOptions {
        full_robots: params.full_robots,
        full_tools: params.full_tools,
    };
    compose_stage_with_options(&scene, &manifest_abs, &zip_path, &opts)
        .map_err(|e| ApiError::internal("compose_failed", e.to_string()))?;
    let bytes = std::fs::metadata(&zip_path)
        .map_err(|e| ApiError::internal("zip_stat_failed", e.to_string()))?
        .len();

    Ok(ComposeArtifact {
        zip_path: zip_path.display().to_string(),
        download_name: format!("{}.zip", stem),
        bytes,
    })
}

/// `compose.urdfBundle` — mirror `POST /api/v1/compose-urdf-bundle`. Same
/// contract as `compose_run` but builds the self-contained URDF-source
/// bundle (no fidelity flags) and suggests a `<stem>-urdf.zip` download
/// name.
fn compose_urdf_bundle_run(
    state: &AppState,
    params: UrdfBundleParams,
) -> Result<ComposeArtifact, ApiError> {
    let manifest_abs = state
        .resolve_asset(&params.manifest_path)
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
        return Err(ApiError::bad_request("manifest_invalid", joined)
            .with_hint("call compose.validate to inspect each error"));
    }

    let stem = manifest_abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("scene");
    let zip_path = unique_zip_path(stem);
    compose_urdf_bundle(&scene, &manifest_abs, &zip_path)
        .map_err(|e| ApiError::internal("bundle_failed", e.to_string()))?;
    let bytes = std::fs::metadata(&zip_path)
        .map_err(|e| ApiError::internal("zip_stat_failed", e.to_string()))?
        .len();

    Ok(ComposeArtifact {
        zip_path: zip_path.display().to_string(),
        download_name: format!("{}-urdf.zip", stem),
        bytes,
    })
}

// ---------------------------------------------------------------------------
// scene.* — mirror src/server/handlers/scene.rs
// ---------------------------------------------------------------------------

/// `scene.load` — return the raw YAML at `<asset_root>/<path>`, folded into
/// a `{yaml}` object. Mirrors `POST /api/v1/scenes/load`.
fn scene_load(state: &AppState, q: PathQuery) -> Result<SceneLoadResult, ApiError> {
    let abs = state
        .resolve_asset(&q.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    let yaml = std::fs::read_to_string(&abs)
        .map_err(|e| ApiError::not_found("manifest_not_found", e.to_string()))?;
    Ok(SceneLoadResult { yaml })
}

/// `scene.save` — resolve `path` against `asset_root` and write `yaml`
/// verbatim. Mirrors `POST /api/v1/scenes/save`.
fn scene_save(state: &AppState, body: ScenePathBody) -> Result<SaveResponse, ApiError> {
    let abs = state
        .resolve_asset(&body.path)
        .map_err(|e| ApiError::bad_request("invalid_path", e.to_string()))?;
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ApiError::internal("mkdir_failed", e.to_string()))?;
    }
    std::fs::write(&abs, &body.yaml)
        .map_err(|e| ApiError::internal("write_failed", e.to_string()))?;
    Ok(SaveResponse {
        ok: true,
        bytes: body.yaml.len() as u64,
    })
}
