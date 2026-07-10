//! Unit tests confirming the `stdio` transport's path-taking methods apply
//! the same asset-root sandboxing the HTTP handlers do.
//!
//! Gated behind the `stdio` feature. Run with:
//! `cargo test --features stdio --test stdio_sandboxing`.
//!
//! ## Property 3: Sandboxing preserved
//! **Validates: Requirements 3.6, 8.2**
//!
//! Every method that accepts a filesystem-relative path parameter
//! (`urdf.joints`, `floorPlan.segments`, `scene.load`, `scene.save`,
//! `compose.run`'s `manifestPath`, `compose.urdfBundle`'s `manifestPath`)
//! must reject both absolute paths and `..` traversal by delegating to
//! `AppState::resolve_asset`, producing the same `ApiError` the HTTP
//! handler produces (status 400 with the machine-readable code
//! `invalid_path` for the URDF/floor-plan/scene handlers, or
//! `invalid_manifest_path` for the two compose handlers). This mirrors the
//! HTTP-side expectations in `tests/server.rs` (e.g. a `../etc/passwd`
//! path on `/api/v1/urdf/joints` is never 2xx), keeping transport behavior
//! limited to framing/error-shape differences, not sandboxing logic
//! (Requirement 8.2).

#![cfg(feature = "stdio")]

use robo_fitting_room::stdio::dispatch::dispatch;
use robo_fitting_room::stdio::protocol::RpcError;
use robo_fitting_room::transport_common::AppState;
use serde_json::{json, Value};

/// Build an `AppState` rooted at a fresh temp directory. The directory is
/// intentionally empty: the sandbox check runs before any filesystem read,
/// so a valid (existing, canonicalizable) asset root plus a hostile path is
/// all these cases need.
fn state_over_temp_root() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("create temp asset root");
    let state = AppState::new(dir.path().to_path_buf(), None);
    (state, dir)
}

/// Assert that dispatching `method` with `params` is rejected as an invalid
/// path: an `Err(RpcError)` whose numeric code is 400 (matching the HTTP
/// status the corresponding handler returns) and whose `data.code` equals
/// `expected_code` (`invalid_path` or `invalid_manifest_path`).
fn assert_invalid_path(state: &AppState, method: &str, params: Value, expected_code: &str) {
    let outcome = dispatch(state, method, params);
    let err: RpcError = match outcome {
        Err(e) => e,
        Ok(v) => panic!("{method} should reject the hostile path, got Ok({v:?})"),
    };
    assert_eq!(
        err.code, 400,
        "{method} must map the sandbox rejection to HTTP-status-like code 400"
    );
    let data = err
        .data
        .as_ref()
        .unwrap_or_else(|| panic!("{method} error must carry data with a machine-readable code"));
    assert_eq!(
        data.code, expected_code,
        "{method} must produce data.code == {expected_code}"
    );
}

/// Each path-taking method paired with a closure building its params from a
/// single hostile path string, and the machine-readable error code the
/// corresponding HTTP handler produces.
///
/// - `urdf.joints`, `floorPlan.segments`, `scene.load`, `scene.save` all
///   resolve their `path` via `resolve_asset` → `ApiError::bad_request("invalid_path", ..)`.
/// - `compose.run` / `compose.urdfBundle` resolve `manifestPath` →
///   `ApiError::bad_request("invalid_manifest_path", ..)`.
fn path_taking_cases() -> Vec<(&'static str, fn(&str) -> Value, &'static str)> {
    vec![
        ("urdf.joints", (|p| json!({ "path": p })) as fn(&str) -> Value, "invalid_path"),
        ("floorPlan.segments", (|p| json!({ "path": p })) as fn(&str) -> Value, "invalid_path"),
        ("scene.load", (|p| json!({ "path": p })) as fn(&str) -> Value, "invalid_path"),
        (
            "scene.save",
            (|p| json!({ "path": p, "yaml": "name: t\n" })) as fn(&str) -> Value,
            "invalid_path",
        ),
        (
            "compose.run",
            (|p| json!({ "manifestPath": p, "fullRobots": false, "fullTools": true }))
                as fn(&str) -> Value,
            "invalid_manifest_path",
        ),
        (
            "compose.urdfBundle",
            (|p| json!({ "manifestPath": p })) as fn(&str) -> Value,
            "invalid_manifest_path",
        ),
    ]
}

/// Every path-taking method rejects an absolute path (`/etc/passwd`) with a
/// 400 invalid-path error, identically to the HTTP handlers.
#[test]
fn path_methods_reject_absolute_paths() {
    let (state, _dir) = state_over_temp_root();
    for (method, build_params, expected_code) in path_taking_cases() {
        assert_invalid_path(&state, method, build_params("/etc/passwd"), expected_code);
    }
}

/// Every path-taking method rejects a `..` traversal path
/// (`../../etc/passwd`) with a 400 invalid-path error, identically to the
/// HTTP handlers.
#[test]
fn path_methods_reject_parent_dir_traversal() {
    let (state, _dir) = state_over_temp_root();
    for (method, build_params, expected_code) in path_taking_cases() {
        assert_invalid_path(&state, method, build_params("../../etc/passwd"), expected_code);
    }
}

/// A single embedded `..` component (not just a leading one) is also
/// rejected, guarding against a resolver that only checks the first segment.
#[test]
fn path_methods_reject_embedded_parent_dir() {
    let (state, _dir) = state_over_temp_root();
    for (method, build_params, expected_code) in path_taking_cases() {
        assert_invalid_path(
            &state,
            method,
            build_params("floor-plans/../../etc/passwd"),
            expected_code,
        );
    }
}
