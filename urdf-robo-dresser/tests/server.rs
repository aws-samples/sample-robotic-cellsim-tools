//! Happy-path integration tests for `urdf-robo-dresser serve`. Mirrors
//! the fitting-room's `tests/server.rs` shape so both crates' wire
//! contracts are exercised the same way. Gated behind the `server`
//! feature so default-feature builds don't pull in axum/tokio/reqwest.
//! Run with: `cargo test --features server --test server`.

#![cfg(feature = "server")]

use std::net::SocketAddr;
use std::path::Path;

use serde_json::Value;
use urdf_robo_dresser::server::{router, AppState};

async fn spawn_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let catalogue_root = dir.path().to_path_buf();
    seed_catalogue(&catalogue_root);

    let state = AppState::new(catalogue_root);
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{}", addr), dir)
}

/// Seed a minimal catalogue:
///   urdf-robot-collection/r1/r1.urdf
///   urdf-tool-collection/t1/t1.urdf
fn seed_catalogue(root: &Path) {
    let r_dir = root.join("urdf-robot-collection/r1");
    std::fs::create_dir_all(&r_dir).unwrap();
    write_robot_urdf(&r_dir.join("r1.urdf"));

    let t_dir = root.join("urdf-tool-collection/t1");
    std::fs::create_dir_all(&t_dir).unwrap();
    write_tool_urdf(&t_dir.join("t1.urdf"));
}

fn write_robot_urdf(path: &Path) {
    // Single-link static URDF so validate / catalogue scan work without
    // pulling in mesh files. The dresser's full `dress` flow needs more
    // than this; these tests cover catalogue / validate / health, plus
    // the dress sad-path (unknown id).
    let s = "<?xml version=\"1.0\"?>\n<robot name=\"r1\">\n  <link name=\"base\"/>\n</robot>\n";
    std::fs::write(path, s).unwrap();
}

fn write_tool_urdf(path: &Path) {
    let s = "<?xml version=\"1.0\"?>\n<robot name=\"t1\">\n  <link name=\"tcp\"/>\n</robot>\n";
    std::fs::write(path, s).unwrap();
}

fn rq() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .unwrap()
}

// ---------- catalogue ----------

#[tokio::test]
async fn catalogue_listing_includes_seeded_assets() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .get(format!("{}/api/v1/catalogue", base))
        .send().await.unwrap()
        .json().await.unwrap();
    let robots = v["robots"].as_array().unwrap();
    let tools = v["tools"].as_array().unwrap();
    assert!(robots.iter().any(|r| r["name"] == "r1"), "robots: {:?}", robots);
    assert!(tools.iter().any(|t| t["name"] == "t1"), "tools: {:?}", tools);
}

#[tokio::test]
async fn catalogue_scoped_listings_split_robots_and_tools() {
    let (base, _dir) = spawn_server().await;
    let robots: Value = rq()
        .get(format!("{}/api/v1/catalogue/robots", base))
        .send().await.unwrap()
        .json().await.unwrap();
    let tools: Value = rq()
        .get(format!("{}/api/v1/catalogue/tools", base))
        .send().await.unwrap()
        .json().await.unwrap();
    assert!(robots.as_array().unwrap().iter().any(|r| r["name"] == "r1"));
    assert!(tools.as_array().unwrap().iter().any(|t| t["name"] == "t1"));
}

// ---------- validate ----------

#[tokio::test]
async fn validate_returns_diagnostics_envelope() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .post(format!("{}/api/v1/validate", base))
        .json(&serde_json::json!({
            "path": "urdf-robot-collection/r1/r1.urdf",
            "tool": false,
        }))
        .send().await.unwrap()
        .json().await.unwrap();
    assert!(v.get("ok").is_some(), "missing ok field: {:?}", v);
    assert!(v["diagnostics"].is_array(), "missing diagnostics: {:?}", v);
}

#[tokio::test]
async fn validate_rejects_path_traversal() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!("{}/api/v1/validate", base))
        .json(&serde_json::json!({"path": "../etc/passwd"}))
        .send().await.unwrap();
    assert!(!resp.status().is_success(), "should not be 2xx, got {}", resp.status());
    let v: Value = resp.json().await.unwrap();
    assert!(v["code"].is_string(), "expected typed envelope: {:?}", v);
}

// ---------- dress (sad path) ----------

#[tokio::test]
async fn dress_unknown_robot_returns_404() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!("{}/api/v1/dress", base))
        .json(&serde_json::json!({
            "robot_id": "does_not_exist",
            "tool_id": "t1",
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NOT_FOUND);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["code"], "robot_not_in_catalogue");
}

#[tokio::test]
async fn dress_invalid_orient_returns_400() {
    let (base, _dir) = spawn_server().await;
    let resp = rq()
        .post(format!("{}/api/v1/dress", base))
        .json(&serde_json::json!({
            "robot_id": "r1",
            "tool_id": "t1",
            "orient": [45, 0, 0],
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["code"], "invalid_orient");
}

// ---------- health / readiness ----------

#[tokio::test]
async fn healthz_returns_status_ok() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .get(format!("{}/healthz", base))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(v["status"], "ok");
    assert!(v["version"].is_string());
}

#[tokio::test]
async fn readyz_reports_catalogue_counts() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .get(format!("{}/readyz", base))
        .send().await.unwrap()
        .json().await.unwrap();
    assert_eq!(v["ready"], true, "{:?}", v);
    assert!(v["robot_count"].as_u64().unwrap() >= 1);
    assert!(v["tool_count"].as_u64().unwrap() >= 1);
}

// ---------- openapi ----------

#[tokio::test]
async fn openapi_spec_is_served_under_v1_only() {
    let (base, _dir) = spawn_server().await;
    let v: Value = rq()
        .get(format!("{}/openapi.json", base))
        .send().await.unwrap()
        .json().await.unwrap();
    assert!(v.get("openapi").is_some());
    let paths = v["paths"].as_object().unwrap();
    for k in paths.keys() {
        assert!(
            k.starts_with("/api/v1/") || k == "/healthz" || k == "/readyz",
            "spec path {} not under /api/v1 contract",
            k,
        );
    }
}
