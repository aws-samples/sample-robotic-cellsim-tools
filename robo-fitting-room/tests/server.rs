//! Happy-path integration tests for the in-binary HTTP server. Gated behind
//! the `server` feature so default-feature builds don't pull in axum/tokio/
//! reqwest. Run with: `cargo test --features server --test server`.

#![cfg(feature = "server")]

use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use robo_fitting_room::server::{router, AppState};
use serde_json::Value;

// -----------------------------------------------------------------------------
// Test scaffolding
// -----------------------------------------------------------------------------

/// Stand up the router on a random port and return its base URL plus the
/// tempdir that holds the asset_root.
async fn spawn_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let asset_root = dir.path().to_path_buf();
    populate_assets(&asset_root);

    let state = AppState::new(asset_root, None);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{}", addr), dir)
}

/// Lay down a minimal asset tree under `root`:
///   urdf-dressed-robots/r1/r1.urdf            (6-axis revolute)
///   urdf-fixture-collection/f1/f1.urdf        (single fixed-joint fixture)
///   floor-plans/empty.obj
///   fitted-robots/scene.yaml                  (one robot, no fixtures, no floor)
fn populate_assets(root: &Path) {
    // Robot URDF (6 actuated joints to mirror real cells)
    let r_dir = root.join("urdf-dressed-robots/r1");
    std::fs::create_dir_all(r_dir.join("meshes/robot")).unwrap();
    write_obj_triangle(&r_dir.join("meshes/robot/m.obj"));
    write_kr_urdf(&r_dir.join("r1.urdf"));

    // Fixture URDF
    let f_dir = root.join("urdf-fixture-collection/f1");
    std::fs::create_dir_all(f_dir.join("meshes")).unwrap();
    write_obj_triangle(&f_dir.join("meshes/m.obj"));
    write_fixture_urdf(&f_dir.join("f1.urdf"));

    // Floor plan
    let fp_dir = root.join("floor-plans");
    std::fs::create_dir_all(&fp_dir).unwrap();
    std::fs::write(
        fp_dir.join("empty.obj"),
        "v 0 0 0\nv 1 0 0\nl 1 2\n",
    )
    .unwrap();

    // Sample manifest
    let s_dir = root.join("fitted-robots");
    std::fs::create_dir_all(&s_dir).unwrap();
    let yaml = format!(
        "name: \"t\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"../urdf-dressed-robots/r1/r1.urdf\"\n    position: {{ x: 0.0, y: 0.0, z: 0.0 }}\n    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}\n",
    );
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

fn write_fixture_urdf(path: &Path) {
    let s = "<?xml version=\"1.0\"?>\n<robot name=\"f\">\n  <link name=\"World\">\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/m.obj\"/>\n      </geometry>\n    </visual>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/m.obj\"/>\n      </geometry>\n    </collision>\n  </link>\n  <link name=\"plate\"/>\n  <joint name=\"j_fix\" type=\"fixed\">\n    <parent link=\"World\"/>\n    <child link=\"plate\"/>\n    <origin xyz=\"0 0 0\" rpy=\"0 0 0\"/>\n  </joint>\n</robot>\n";
    std::fs::write(path, s).unwrap();
}

fn rq() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap()
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[tokio::test]
async fn list_robots_returns_seeded_entry() {
    let (base, _dir) = spawn_server().await;
    let resp = rq().get(format!("{}/api/v1/assets/robots", base)).send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let v: Value = resp.json().await.unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "r1");
    assert_eq!(arr[0]["urdf_rel"], "urdf-dressed-robots/r1/r1.urdf");
}

#[tokio::test]
async fn list_fixtures_returns_seeded_entry() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .get(format!("{}/api/v1/assets/fixtures", base))
        .send().await.unwrap()
        .json().await.unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "f1");
}

#[tokio::test]
async fn list_floor_plans_returns_obj() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .get(format!("{}/api/v1/assets/floor-plans", base))
        .send().await.unwrap()
        .json().await.unwrap();
    let arr = v.as_array().unwrap();
    assert!(arr.iter().any(|e| e["name"] == "empty.obj"));
}

#[tokio::test]
async fn urdf_joints_returns_actuated_count() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .post(format!("{}/api/v1/urdf/joints", base))
        .json(&serde_json::json!({ "path": "urdf-dressed-robots/r1/r1.urdf" }))
        .send().await.unwrap()
        .json().await.unwrap();
    let joints = v["joints"].as_array().unwrap();
    // 6 revolute joints (joint_a1..a6); the world_to_base fixed joint must be filtered.
    assert_eq!(joints.len(), 6);
    assert_eq!(joints[0]["name"], "joint_a1");
    assert_eq!(joints[0]["kind"], "revolute");
}

#[tokio::test]
async fn floor_plan_segments_returns_geometry() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .post(format!("{}/api/v1/floor-plan", base))
        .json(&serde_json::json!({ "path": "floor-plans/empty.obj" }))
        .send().await.unwrap()
        .json().await.unwrap();
    let segs = v["segments"].as_array().unwrap();
    assert_eq!(segs.len(), 1);
}

#[tokio::test]
async fn validate_round_trips_yaml() {
    let (base, _dir) = spawn_server().await;
    // Craft a valid manifest in-memory referencing the URDF that's on disk.
    // The validate handler resolves manifest paths against
    // `<asset_root>/fitted-robots/`, so reach back via `..` to hit the URDF.
    let yaml = "name: \"t\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"../urdf-dressed-robots/r1/r1.urdf\"\n    position: { x: 0.0, y: 0.0, z: 0.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\n";
    let resp = rq()
        .post(format!("{}/api/v1/validate", base))
        .header("Content-Type", "application/x-yaml")
        .body(yaml)
        .send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["ok"], true, "{:?}", v);
}

#[tokio::test]
async fn validate_rejects_bad_home_pose_length() {
    let (base, _dir) = spawn_server().await;
    // Manifest paths resolve relative to <asset_root>/fitted-robots/.
    let yaml = "name: \"t\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"../urdf-dressed-robots/r1/r1.urdf\"\n    position: { x: 0.0, y: 0.0, z: 0.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\n    home_pose_deg: [0.0, 0.0]\n";
    let v: Value = rq()
        .post(format!("{}/api/v1/validate", base))
        .header("Content-Type", "application/x-yaml")
        .body(yaml)
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(v["ok"], false);
    let errs = v["errors"].as_array().unwrap();
    assert!(errs.iter().any(|e| e.as_str().unwrap_or("").contains("home_pose_deg")));
}

#[tokio::test]
async fn save_then_load_round_trips() {
    let (base, _dir) = spawn_server().await;
    let body = serde_json::json!({
        "path": "fitted-robots/edited.yaml",
        "yaml": "name: \"edited\"\n",
    });
    let resp = rq()
        .post(format!("{}/api/v1/scenes/save", base))
        .json(&body)
        .send().await.unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    assert!(status.is_success(), "save failed: {} body={}", status, text);
    let v: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("save body not json: {} (body={})", e, text));
    assert_eq!(v["ok"], true);

    let loaded = rq()
        .post(format!("{}/api/v1/scenes/load", base))
        .json(&serde_json::json!({ "path": "fitted-robots/edited.yaml" }))
        .send().await.unwrap()
        .text().await.unwrap();
    assert!(loaded.contains("edited"));
}

#[tokio::test]
async fn compose_returns_zip() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!(
            "{}/api/v1/compose?manifest_path=fitted-robots/scene.yaml",
            base
        ))
        .send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(ct, "application/zip", "wrong content-type");
    let bytes = resp.bytes().await.unwrap();
    // PK\x03\x04 zip local-file header
    assert_eq!(&bytes[..4], b"PK\x03\x04", "first 4 bytes should be PK header");
}

#[tokio::test]
async fn compose_urdf_bundle_returns_zip() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!(
            "{}/api/v1/compose-urdf-bundle?manifest_path=fitted-robots/scene.yaml",
            base
        ))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(ct, "application/zip", "wrong content-type");
    let bytes = resp.bytes().await.unwrap();
    assert_eq!(&bytes[..4], b"PK\x03\x04", "first 4 bytes should be PK header");

    // Unzip in memory: expect the scene YAML + at least one bundled URDF.
    let reader = std::io::Cursor::new(bytes.to_vec());
    let mut z = zip::ZipArchive::new(reader).unwrap();
    let names: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
    assert!(names.iter().any(|n| n.ends_with(".yaml")), "names: {:?}", names);
    assert!(
        names.iter().any(|n| n.starts_with("robots/") && n.ends_with(".urdf")),
        "expected a bundled robot URDF, names: {:?}",
        names
    );
}

#[tokio::test]
async fn upload_obj_round_trips_and_appears_in_listing() {
    let (base, _dir) = spawn_server().await;
    let form = reqwest::multipart::Form::new()
        .text("filename", "uploaded.obj")
        .part(
            "file",
            reqwest::multipart::Part::bytes(
                b"v 0 0 0\nv 1 0 0\nl 1 2\n".to_vec(),
            )
            .file_name("uploaded.obj"),
        );
    let v: Value = rq()
        .post(format!("{}/api/v1/assets/floor-plans/upload-obj", base))
        .multipart(form)
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(v["stored_path"], "floor-plans/uploaded.obj");

    let list: Value = rq()
        .get(format!("{}/api/v1/assets/floor-plans", base))
        .send().await.unwrap()
        .json().await.unwrap();
    let arr = list.as_array().unwrap();
    assert!(arr.iter().any(|e| e["name"] == "uploaded.obj"));
}

#[tokio::test]
async fn path_traversal_is_blocked() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!("{}/api/v1/urdf/joints", base))
        .json(&serde_json::json!({ "path": "../etc/passwd" }))
        .send().await.unwrap();
    // Either 400 (rejected by resolver) or 404; never 200.
    assert!(!resp.status().is_success(), "{}", resp.status());
}

/// FR-205: OpenAPI spec is served at /openapi.json (and Swagger UI at /docs).
#[tokio::test]
async fn openapi_spec_is_served() {
    let (base, _dir) = spawn_server().await;
    let resp = rq().get(format!("{}/openapi.json", base)).send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let v: Value = resp.json().await.unwrap();
    // Minimal sanity: it really is an OpenAPI 3.x doc.
    assert!(v.get("openapi").is_some(), "missing top-level `openapi`");
    assert!(v.get("paths").is_some(), "missing `paths`");
    // Every documented path is under /api/v1 (or the root probes).
    let paths = v["paths"].as_object().unwrap();
    for k in paths.keys() {
        assert!(
            k.starts_with("/api/v1/") || k == "/healthz" || k == "/readyz",
            "spec path {} not under /api/v1 contract",
            k,
        );
    }
}

/// Liveness + readiness probes at the root (k8s convention).
#[tokio::test]
async fn healthz_and_readyz_are_served() {
    let (base, _dir) = spawn_server().await;
    let h: Value = rq()
        .get(format!("{}/healthz", base))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(h["status"], "ok");
    assert!(h["version"].is_string());

    let r: Value = rq()
        .get(format!("{}/readyz", base))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(r["ready"], true, "readyz should be true once asset_root is seeded: {:?}", r);
    assert!(r["urdf_count"].as_u64().unwrap() >= 1);
}

/// Legacy `/api/...` aliases continue to work but carry `Deprecation: true`.
#[tokio::test]
async fn legacy_api_alias_is_marked_deprecated() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .get(format!("{}/api/assets/robots", base))
        .send().await.unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let dep = resp
        .headers()
        .get("deprecation")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(dep, "true", "legacy /api alias must carry `Deprecation: true`");
}

/// Error envelope is a typed body, not a bare string.
#[tokio::test]
async fn error_envelope_is_typed() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!("{}/api/v1/urdf/joints", base))
        .json(&serde_json::json!({ "path": "does/not/exist.urdf" }))
        .send().await.unwrap();
    assert!(!resp.status().is_success());
    let v: Value = resp.json().await.unwrap();
    assert!(v["code"].is_string(), "envelope must carry `code`: {:?}", v);
    assert!(v["message"].is_string(), "envelope must carry `message`: {:?}", v);
}

// avoid unused-import lint when one helper isn't called in a given test path
#[allow(dead_code)]
fn _silence(_: PathBuf) {}
