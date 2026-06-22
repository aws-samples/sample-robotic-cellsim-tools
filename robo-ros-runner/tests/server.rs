//! Happy-path integration tests for the in-binary HTTP server.
//! Run with: `cargo test --test server`.

use std::io::Write;
use std::net::SocketAddr;
use std::path::Path;

use robo_ros_runner::server::{router, AppState};
use serde_json::Value;

/// Stand up the router on a random port; return its base URL + the asset tempdir.
async fn spawn_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let asset_root = dir.path().to_path_buf();
    populate_assets(&asset_root);

    let state = AppState::new(asset_root);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{}", addr), dir)
}

/// Lay down a minimal asset tree: a cell manifest, a 6-axis URDF, a capture.
fn populate_assets(root: &Path) {
    let r_dir = root.join("urdf-dressed-robots/r1");
    std::fs::create_dir_all(&r_dir).unwrap();
    write_kr_urdf(&r_dir.join("r1.urdf"));

    let s_dir = root.join("fitted-robots");
    std::fs::create_dir_all(&s_dir).unwrap();
    std::fs::write(
        s_dir.join("cell.yaml"),
        "name: \"Test Cell\"\nrobots:\n  - robot_id: \"113190R01\"\n    dressed_urdf: \"../urdf-dressed-robots/r1/r1.urdf\"\n    position: { x: 0, y: 0, z: 0 }\n    orientation: { a: 0, b: 0, c: 0 }\n",
    )
    .unwrap();

    let c_dir = root.join("data-captures");
    std::fs::create_dir_all(&c_dir).unwrap();
    std::fs::write(
        c_dir.join("cap.csv"),
        "t_profinet_s,113190R01_A1_deg,113190R01_A2_deg,113190R01_A3_deg,113190R01_A4_deg,113190R01_A5_deg,113190R01_A6_deg\n0.0,0,0,0,0,0,0\n0.1,90,0,0,0,0,0\n",
    )
    .unwrap();
}

fn write_kr_urdf(path: &Path) {
    let mut s = String::from("<?xml version=\"1.0\"?>\n<robot name=\"r\">\n");
    for n in &["World", "base", "l1", "l2", "l3", "l4", "l5", "l6"] {
        s.push_str(&format!("  <link name=\"{}\"/>\n", n));
    }
    s.push_str("  <joint name=\"root_joint\" type=\"fixed\">\n    <parent link=\"World\"/>\n    <child link=\"base\"/>\n  </joint>\n");
    for i in 1..=6 {
        let parent = if i == 1 { "base".to_string() } else { format!("l{}", i - 1) };
        s.push_str(&format!(
            "  <joint name=\"joint_a{}\" type=\"revolute\">\n    <parent link=\"{}\"/>\n    <child link=\"l{}\"/>\n    <axis xyz=\"0 0 1\"/>\n    <limit lower=\"-3.14\" upper=\"3.14\" effort=\"100\" velocity=\"1\"/>\n  </joint>\n",
            i, parent, i
        ));
    }
    s.push_str("</robot>\n");
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(s.as_bytes()).unwrap();
}

// FR-707.
#[tokio::test]
async fn healthz_and_readyz() {
    let (base, _dir) = spawn_server().await;
    let h = reqwest::get(format!("{}/healthz", base)).await.unwrap();
    assert_eq!(h.status(), 200);
    let body: Value = h.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    let r = reqwest::get(format!("{}/readyz", base)).await.unwrap();
    assert_eq!(r.status(), 200);
}

// FR-701.
#[tokio::test]
async fn topics_endpoint() {
    let (base, _dir) = spawn_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/topics", base))
        .json(&serde_json::json!({"cell_path": "fitted-robots/cell.yaml", "csv_path": "data-captures/cap.csv"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["cell_name"], "Test Cell");
    assert_eq!(body["topics"][0]["topic"], "/Test_Cell/r_113190R01/joint_command");
    assert_eq!(body["topics"][0]["has_csv_data"], true);
}

// FR-702.
#[tokio::test]
async fn csv_inspect_endpoint() {
    let (base, _dir) = spawn_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/csv/inspect", base))
        .json(&serde_json::json!({"csv_path": "data-captures/cap.csv"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["robots"][0], "113190R01");
    assert_eq!(body["axes_per_robot"], 6);
    assert_eq!(body["row_count"], 2);
}

// FR-704, FR-706.
#[tokio::test]
async fn playback_start_and_status() {
    let (base, _dir) = spawn_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/playback", base))
        .json(&serde_json::json!({
            "cell_path": "fitted-robots/cell.yaml",
            "csv_path": "data-captures/cap.csv",
            "rate": 1000.0
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let job_id = body["job_id"].as_str().unwrap().to_string();
    assert_eq!(body["status"], "running");

    // Give the blocking task a moment to finish the 2-frame capture.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    let status = client
        .get(format!("{}/api/v1/playback/{}", base, job_id))
        .send()
        .await
        .unwrap();
    assert_eq!(status.status(), 200);
    let sbody: Value = status.json().await.unwrap();
    assert_eq!(sbody["job_id"], job_id);
    // Either done (finished) or running; frames published should be > 0 once done.
    assert!(sbody["status"] == "done" || sbody["status"] == "running");
}

// FR-705.
#[tokio::test]
async fn playback_stop() {
    let (base, _dir) = spawn_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/playback", base))
        .json(&serde_json::json!({
            "cell_path": "fitted-robots/cell.yaml",
            "csv_path": "data-captures/cap.csv",
            "loop_play": true,
            "rate": 1.0
        }))
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    let job_id = body["job_id"].as_str().unwrap().to_string();

    let del = client
        .delete(format!("{}/api/v1/playback/{}", base, job_id))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);
}

// FR-706 (unknown job → 404 with envelope).
#[tokio::test]
async fn unknown_job_is_404() {
    let (base, _dir) = spawn_server().await;
    let resp = reqwest::get(format!("{}/api/v1/playback/does-not-exist", base))
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "unknown_job");
}

// FR-709, FR-710: an asset path with `..` is rejected with the error envelope.
#[tokio::test]
async fn asset_sandbox_rejects_traversal() {
    let (base, _dir) = spawn_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/v1/csv/inspect", base))
        .json(&serde_json::json!({"csv_path": "../../etc/passwd"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["code"], "invalid_path");
}
