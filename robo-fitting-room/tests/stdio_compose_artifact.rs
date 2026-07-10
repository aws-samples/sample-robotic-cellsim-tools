//! Unit/integration tests for the `stdio` transport's compose artifact
//! temp-file behavior.
//!
//! Gated behind the `stdio` feature. Run with:
//! `cargo test --features stdio --test stdio_compose_artifact`.
//!
//! ## Property 6: Artifact existence
//! **Validates: Requirements 4.1, 4.2, 4.3, 4.4**
//!
//! On a successful `compose.run` / `compose.urdfBundle` dispatch, the
//! returned `ComposeArtifact` names a file that exists on disk whose real
//! size equals the reported `bytes` exactly (Req 4.1), the file is not
//! deleted before the response is returned (Req 4.3), and it opens as a
//! valid zip archive. On a validation/parse failure, dispatch returns an
//! error and no temp zip file path is referenced anywhere in the error
//! response (Req 4.4).

#![cfg(feature = "stdio")]

use std::io::Write;
use std::path::{Path, PathBuf};

use robo_fitting_room::stdio::dispatch::dispatch;
use robo_fitting_room::transport_common::AppState;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixture asset tree (minimal: one 6-axis robot + one valid manifest)
// ---------------------------------------------------------------------------

/// Stand up an `AppState` over a freshly-seeded asset tempdir. The returned
/// `TempDir` guard must be kept alive for the duration of the test.
fn state_over_fixture() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    populate_assets(dir.path());
    let state = AppState::new(dir.path().to_path_buf(), None);
    (state, dir)
}

/// Lay down a minimal asset tree that composes cleanly:
///   urdf-dressed-robots/r1/r1.urdf   (6-axis revolute, mesh-backed)
///   fitted-robots/scene.yaml         (one robot, no fixtures, no floor)
fn populate_assets(root: &Path) {
    let r_dir = root.join("urdf-dressed-robots/r1");
    std::fs::create_dir_all(r_dir.join("meshes/robot")).unwrap();
    write_obj_triangle(&r_dir.join("meshes/robot/m.obj"));
    write_kr_urdf(&r_dir.join("r1.urdf"));

    let s_dir = root.join("fitted-robots");
    std::fs::create_dir_all(&s_dir).unwrap();
    let yaml = "name: \"t\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"../urdf-dressed-robots/r1/r1.urdf\"\n    position: { x: 0.0, y: 0.0, z: 0.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\n";
    std::fs::write(s_dir.join("scene.yaml"), yaml).unwrap();
}

fn write_obj_triangle(path: &Path) {
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "v 0 0 0").unwrap();
    writeln!(f, "v 1 0 0").unwrap();
    writeln!(f, "v 0 1 0").unwrap();
    writeln!(f, "f 1 2 3").unwrap();
}

fn write_kr_urdf(path: &Path) {
    let names = [
        "World",
        "like_kr240r2900_base_link",
        "like_kr240r2900_link_1",
        "like_kr240r2900_link_2",
        "like_kr240r2900_link_3",
        "like_kr240r2900_link_4",
        "like_kr240r2900_link_5",
        "like_kr240r2900_link_6",
    ];
    let mut s = String::from("<?xml version=\"1.0\"?>\n<robot name=\"r\">\n");
    for n in &names {
        s.push_str(&format!(
            "  <link name=\"{}\">\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/robot/m.obj\"/>\n      </geometry>\n    </visual>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/robot/m.obj\"/>\n      </geometry>\n    </collision>\n  </link>\n",
            n
        ));
    }
    s.push_str(
        "  <joint name=\"world_to_base\" type=\"fixed\">\n    <parent link=\"World\"/>\n    <child link=\"like_kr240r2900_base_link\"/>\n    <origin xyz=\"0 0 0\" rpy=\"0 0 0\"/>\n  </joint>\n",
    );
    for i in 1..=6 {
        let parent = if i == 1 {
            "like_kr240r2900_base_link".to_string()
        } else {
            format!("like_kr240r2900_link_{}", i - 1)
        };
        s.push_str(&format!(
            "  <joint name=\"joint_a{}\" type=\"revolute\">\n    <parent link=\"{}\"/>\n    <child link=\"like_kr240r2900_link_{}\"/>\n    <origin xyz=\"0 0 0.5\" rpy=\"0 0 0\"/>\n    <axis xyz=\"0 0 1\"/>\n    <limit lower=\"-3.14\" upper=\"3.14\" effort=\"100\" velocity=\"1\"/>\n  </joint>\n",
            i, parent, i
        ));
    }
    s.push_str("</robot>\n");
    std::fs::write(path, s).unwrap();
}

/// Extract and validate a `ComposeArtifact` JSON `Value`: the file at
/// `zipPath` must exist with an on-disk size exactly equal to `bytes`, and
/// it must open as a valid zip archive. Returns the `zipPath` so the caller
/// can clean it up.
fn assert_artifact_on_disk(result: &Value, expected_download: &str) -> PathBuf {
    let zip_path = result["zipPath"]
        .as_str()
        .unwrap_or_else(|| panic!("ComposeArtifact missing zipPath: {result}"));
    let bytes = result["bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("ComposeArtifact missing bytes: {result}"));
    assert_eq!(
        result["downloadName"].as_str(),
        Some(expected_download),
        "unexpected downloadName in {result}"
    );

    let path = PathBuf::from(zip_path);
    assert!(
        path.exists(),
        "temp zip must exist on disk (Req 4.3): {}",
        path.display()
    );

    // Req 4.1: the reported `bytes` equals the file's real on-disk size.
    let actual = std::fs::metadata(&path)
        .expect("temp zip metadata")
        .len();
    assert_eq!(
        actual, bytes,
        "reported bytes ({bytes}) must equal real on-disk size ({actual})"
    );

    // The artifact opens as a valid, non-empty zip archive.
    assert!(bytes > 0, "composed zip must be non-empty");
    let f = std::fs::File::open(&path).expect("open temp zip");
    let archive = zip::ZipArchive::new(f).expect("temp zip must be a valid zip archive");
    assert!(archive.len() > 0, "zip archive must contain entries");

    path
}

/// Assert that a dispatch error carries no reference to a temp zip file
/// path (Req 4.4). We serialize the whole error and confirm none of its
/// fields name a `.zip` file or a `zipPath`/`path`-shaped key.
fn assert_no_temp_file_referenced(err: &robo_fitting_room::stdio::protocol::RpcError) {
    let serialized = serde_json::to_value(err).expect("error serializes");
    // No `zipPath` field anywhere in the error response.
    assert!(
        serialized.get("zipPath").is_none(),
        "error response must not carry a zipPath: {serialized}"
    );
    if let Some(data) = serialized.get("data") {
        assert!(
            data.get("zipPath").is_none() && data.get("path").is_none(),
            "error data must not carry a path/zipPath: {data}"
        );
    }
    // Defensive: no substring naming a composed temp artifact leaks through
    // the message/hint either.
    let text = serialized.to_string();
    assert!(
        !text.contains("rfr-compose-"),
        "error must not reference a composed temp file name: {text}"
    );
}

// ---------------------------------------------------------------------------
// SUCCESS — compose.run
// ---------------------------------------------------------------------------

#[test]
fn compose_run_success_writes_temp_zip_with_exact_size() {
    let (state, _dir) = state_over_fixture();
    let params = json!({
        "manifestPath": "fitted-robots/scene.yaml",
        "fullRobots": false,
        "fullTools": true,
    });

    let result = dispatch(&state, "compose.run", params)
        .expect("compose.run over a valid manifest must succeed");

    let zip_path = assert_artifact_on_disk(&result, "scene.zip");

    // Cleanup: ownership of the temp file transfers to the caller.
    std::fs::remove_file(&zip_path).ok();
}

// ---------------------------------------------------------------------------
// SUCCESS — compose.urdfBundle
// ---------------------------------------------------------------------------

#[test]
fn compose_urdf_bundle_success_writes_temp_zip_with_exact_size() {
    let (state, _dir) = state_over_fixture();
    let params = json!({ "manifestPath": "fitted-robots/scene.yaml" });

    let result = dispatch(&state, "compose.urdfBundle", params)
        .expect("compose.urdfBundle over a valid manifest must succeed");

    let zip_path = assert_artifact_on_disk(&result, "scene-urdf.zip");

    std::fs::remove_file(&zip_path).ok();
}

// ---------------------------------------------------------------------------
// FAILURE — compose.run references no temp file
// ---------------------------------------------------------------------------

#[test]
fn compose_run_failure_references_no_temp_file() {
    let (state, _dir) = state_over_fixture();
    // A well-formed relative path that resolves under asset_root but does
    // not exist → parse_scene fails before any temp file is created.
    let params = json!({ "manifestPath": "fitted-robots/does-not-exist.yaml" });

    let err = dispatch(&state, "compose.run", params)
        .expect_err("compose.run over a missing manifest must fail");

    assert_no_temp_file_referenced(&err);
}

#[test]
fn compose_run_failure_on_traversal_references_no_temp_file() {
    let (state, _dir) = state_over_fixture();
    // Sandbox-escaping path → rejected by resolve_asset before any work.
    let params = json!({ "manifestPath": "../secret.yaml" });

    let err = dispatch(&state, "compose.run", params)
        .expect_err("compose.run over a traversal path must fail");

    assert_no_temp_file_referenced(&err);
}

// ---------------------------------------------------------------------------
// FAILURE — compose.urdfBundle references no temp file
// ---------------------------------------------------------------------------

#[test]
fn compose_urdf_bundle_failure_references_no_temp_file() {
    let (state, _dir) = state_over_fixture();
    let params = json!({ "manifestPath": "fitted-robots/does-not-exist.yaml" });

    let err = dispatch(&state, "compose.urdfBundle", params)
        .expect_err("compose.urdfBundle over a missing manifest must fail");

    assert_no_temp_file_referenced(&err);
}
