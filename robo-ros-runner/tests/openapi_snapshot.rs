//! OpenAPI spec snapshot test. The published `openapi.json` at the crate root
//! is the wire contract — every change to a handler's path, parameters, or
//! response shape must update this snapshot consciously.
//!
//! To regenerate after a deliberate change:
//!     R3_UPDATE_SNAPSHOTS=1 cargo test --test openapi_snapshot
//!
//! Plain `cargo test` enforces the snapshot.

use std::path::PathBuf;

use pretty_assertions::assert_eq;
use robo_ros_runner::server::openapi::ApiDoc;
use utoipa::OpenApi;

fn snapshot_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("openapi.json")
}

#[test]
fn openapi_spec_matches_snapshot() {
    let actual = ApiDoc::openapi().to_pretty_json().expect("serialize spec");
    let actual = format!("{}\n", actual.trim_end());

    let path = snapshot_path();

    if std::env::var("R3_UPDATE_SNAPSHOTS").is_ok() {
        std::fs::write(&path, &actual).expect("write snapshot");
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} not found — generate it with `R3_UPDATE_SNAPSHOTS=1 cargo test \
             --features server --test openapi_snapshot`",
            path.display(),
        )
    });

    assert_eq!(
        expected.trim_end(),
        actual.trim_end(),
        "OpenAPI spec drifted from {}. If the change is intentional, run \
         `R3_UPDATE_SNAPSHOTS=1 cargo test --features server --test openapi_snapshot` \
         to refresh.",
        path.display(),
    );
}
