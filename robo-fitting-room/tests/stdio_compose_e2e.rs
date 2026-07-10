//! End-to-end integration test driving `compose.run` against the *built*
//! `robo-fitting-room` binary over its `stdio-serve` transport.
//!
//! Gated behind the `stdio` feature. Run with:
//! `cargo test --features stdio --test stdio_compose_e2e`.
//!
//! ## Property 6: Artifact existence
//! **Validates: Requirement 4.1**
//!
//! Unlike `tests/stdio_compose_artifact.rs` (which exercises `dispatch()`
//! in-process), this test spawns the real binary with
//! `stdio-serve --asset-root <fixture>`, feeds a single `compose.run`
//! request line on stdin, closes stdin, and inspects the NDJSON response
//! line on stdout. It asserts the returned `ComposeArtifact.zipPath`
//! exists on disk, is non-empty, its on-disk size equals the reported
//! `bytes`, and the file opens as a valid zip archive.

#![cfg(feature = "stdio")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

// ---------------------------------------------------------------------------
// Fixture asset tree (minimal: one 6-axis robot + one valid manifest).
// Copied from `tests/stdio_compose_artifact.rs` so this e2e test is
// self-contained (integration test crates cannot share private helpers).
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// End-to-end: spawn the built binary and drive `compose.run` over stdio.
// ---------------------------------------------------------------------------

#[test]
fn compose_run_e2e_returns_valid_zip_artifact() {
    let dir = tempfile::tempdir().expect("create asset tempdir");
    populate_assets(dir.path());

    let bin = env!("CARGO_BIN_EXE_robo-fitting-room");
    let mut child = Command::new(bin)
        .arg("stdio-serve")
        .arg("--asset-root")
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn robo-fitting-room stdio-serve");

    // Send one compose.run request, then close stdin so the loop exits on EOF.
    let request =
        "{\"id\":1,\"method\":\"compose.run\",\"params\":{\"manifestPath\":\"fitted-robots/scene.yaml\",\"fullRobots\":false,\"fullTools\":true}}\n";
    {
        let mut stdin = child.stdin.take().expect("child stdin piped");
        stdin
            .write_all(request.as_bytes())
            .expect("write compose.run request");
        stdin.flush().expect("flush request");
        // Dropping `stdin` here closes it, signalling EOF to the child.
    }

    // Read exactly one NDJSON response line from stdout.
    let stdout = child.stdout.take().expect("child stdout piped");
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let n = reader.read_line(&mut line).expect("read response line");
    assert!(n > 0, "expected one NDJSON response line, got EOF");

    // The child should exit cleanly (code 0) once stdin closes.
    let status = child.wait().expect("wait for child exit");

    let response: Value = serde_json::from_str(line.trim_end())
        .unwrap_or_else(|e| panic!("response line must be valid JSON ({e}): {line}"));

    assert_eq!(response["id"].as_u64(), Some(1), "response echoes request id");

    let result = response
        .get("result")
        .unwrap_or_else(|| panic!("expected a success `result`, got: {response}"));

    // Property 6 / Requirement 4.1: the artifact names a real, non-empty,
    // valid zip whose on-disk size equals the reported `bytes`.
    let zip_path = result["zipPath"]
        .as_str()
        .unwrap_or_else(|| panic!("ComposeArtifact missing zipPath: {result}"));
    let bytes = result["bytes"]
        .as_u64()
        .unwrap_or_else(|| panic!("ComposeArtifact missing bytes: {result}"));
    assert_eq!(
        result["downloadName"].as_str(),
        Some("scene.zip"),
        "unexpected downloadName in {result}"
    );

    let path = PathBuf::from(zip_path);
    assert!(
        path.exists(),
        "returned zipPath must exist on disk: {}",
        path.display()
    );

    let actual = std::fs::metadata(&path).expect("temp zip metadata").len();
    assert_eq!(
        actual, bytes,
        "reported bytes ({bytes}) must equal real on-disk size ({actual})"
    );
    assert!(bytes > 0, "composed zip must be non-empty");

    let f = std::fs::File::open(&path).expect("open returned temp zip");
    let archive = zip::ZipArchive::new(f).expect("returned file must be a valid zip archive");
    assert!(archive.len() > 0, "zip archive must contain entries");

    // Clean up the temp zip (ownership transferred to us, the caller).
    std::fs::remove_file(&path).ok();

    assert!(
        status.success(),
        "stdio-serve should exit 0 on stdin EOF, got: {status}"
    );
}
