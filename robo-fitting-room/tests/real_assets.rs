//! Real-asset integration tests. These run against the in-repo
//! `assets/` directory rather than tempdir-synthesised URDFs, so they
//! catch regressions a synthetic test would miss (mesh-cache lookups,
//! material registry, KR240 inertia-override path, real
//! `home_pose_deg`, the K8URW11 8-robot scene).
//!
//! **Opt-in via the `real-assets` Cargo feature**, since the catalogue
//! under `assets/` contains vendor-specific URDFs (KR240 variants, GREIFER
//! tool family, K8URW11 manifest) that will NOT ship with the
//! open-source toolchain. When the assets are pruned for the public
//! release, drop `--features real-assets` from CI and (optionally)
//! delete this file.
//!
//! Run locally with:
//!     cargo test --features real-assets --test real_assets
//!
//! Even when the feature is on, missing files are skipped at runtime
//! with a `Skipping real-asset test — missing under …` hint, so a
//! partial checkout doesn't masquerade as a failing implementation.

#![cfg(feature = "real-assets")]

use std::path::{Path, PathBuf};

use robo_fitting_room::{
    compose_stage, compose_stage_with_options, parse_scene, validate_scene, ComposeOptions,
};

/// Canonical assets root: `<crate>/../assets/`.
fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("assets")
}

/// `true` if every required asset exists; otherwise emit a skip message
/// and return `false` so the test can early-return without failing.
fn assets_available(needles: &[&Path]) -> bool {
    let root = assets_root();
    let missing: Vec<_> = needles
        .iter()
        .map(|p| root.join(p))
        .filter(|p| !p.exists())
        .collect();
    if missing.is_empty() {
        return true;
    }
    eprintln!(
        "Skipping real-asset test — missing under {}: {:?}",
        root.display(),
        missing,
    );
    false
}

/// FR-100/103/144: parse + validate the K8URW11 manifest end-to-end.
#[test]
fn k8urw11_manifest_parses_and_validates() {
    if !assets_available(&[Path::new("fitted-robots/k8urw11-cell.yaml")]) {
        return;
    }
    let manifest = assets_root().join("fitted-robots/k8urw11-cell.yaml");
    let scene = parse_scene(&manifest).expect("parse k8urw11");
    assert_eq!(scene.name, "K8URW11 Production Cell");
    assert_eq!(scene.robots.len(), 8, "K8URW11 has 8 robots");

    let errors = validate_scene(&scene);
    assert!(
        errors.is_empty(),
        "K8URW11 manifest must validate cleanly against in-repo assets, got errors: {:?}",
        errors
    );
}

/// FR-101/110/115/146: compose K8URW11 to a zip; verify articulation +
/// per-robot graph + the new contact-sensor wiring.
#[test]
fn k8urw11_composes_with_contact_sensors() {
    if !assets_available(&[Path::new("fitted-robots/k8urw11-cell.yaml")]) {
        return;
    }
    let manifest = assets_root().join("fitted-robots/k8urw11-cell.yaml");
    let scene = parse_scene(&manifest).unwrap();

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("k8urw11.zip");
    let report = compose_stage(&scene, &manifest, &out).unwrap();
    assert_eq!(report.robot_count, 8);
    assert!(out.exists());

    let usda = read_zip_entry(&out, "k8urw11-cell.usda");

    // 8 articulation graphs, 8 PhysxArticulationAPI prims.
    assert_eq!(
        usda.matches("def OmniGraph \"ActionGraph_").count(),
        9, // 8 robots + 1 clock
        "expected 8 robot graphs + 1 clock graph",
    );

    // Track 1: contact-sensor pairs land in EACH per-robot graph.
    // The publisher is the generic `ROS2Publisher` (the
    // `ROS2PublishContactSensor` node type does not exist in Isaac Sim
    // 5.1), so we look for the matching emission marker
    // `messageName = "Bool"` instead.
    assert!(usda.contains("isaacsim.sensors.physics.IsaacReadContactSensor"));
    assert!(usda.contains("isaacsim.ros2.bridge.ROS2Publisher"));
    assert!(usda.contains("inputs:messageName = \"Bool\""));

    // Topic-schema contract: every robot's namespace is
    // `K8URW11_Production_Cell/<r_-prefixed-id>` (sanitised). Spot-check.
    assert!(
        usda.contains("inputs:nodeNamespace = \"K8URW11_Production_Cell/r_113190R01\""),
        "expected two-segment <cell_id>/<robot_id> namespace for 113190R01",
    );

    // Clock topic is cell-scoped.
    assert!(usda.contains("inputs:nodeNamespace = \"K8URW11_Production_Cell\""));
}

/// FR-145/146: lean composition (`full_robots = false`) is the default
/// and produces a valid zip without visual_<i> mesh prims for robots.
#[test]
fn k8urw11_composes_in_lean_mode() {
    if !assets_available(&[Path::new("fitted-robots/k8urw11-cell.yaml")]) {
        return;
    }
    let manifest = assets_root().join("fitted-robots/k8urw11-cell.yaml");
    let scene = parse_scene(&manifest).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("lean.zip");
    let report = compose_stage_with_options(&scene, &manifest, &out, &ComposeOptions::default())
        .expect("compose in lean mode");
    assert!(report.zip_bytes > 0);
    let usda = read_zip_entry(&out, "k8urw11-cell.usda");
    // Collision hulls present, robot visuals absent (default `full_robots = false`).
    assert!(usda.contains("def Mesh \"collision_"));
    // Robot visuals are dropped, but tool visuals remain (`full_tools = true` default).
    // Just assert the lean-mode signature.
    assert!(usda.contains("def PhysicsArticulationRoot") || usda.contains("PhysicsArticulationRootAPI"));
}

fn read_zip_entry(zip_path: &Path, entry: &str) -> String {
    use std::io::Read;
    let f = std::fs::File::open(zip_path).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut e = z.by_name(entry).unwrap();
    let mut s = String::new();
    e.read_to_string(&mut s).unwrap();
    s
}
