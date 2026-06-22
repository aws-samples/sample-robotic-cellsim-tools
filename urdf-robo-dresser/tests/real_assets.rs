//! Real-asset integration tests for `urdf-robo-dresser`. These exercise
//! the parser → validator → merger → serializer pipeline against the
//! in-repo `../assets/urdf-{robot,tool}-collection/` catalogue, plus the
//! catalogue scanner.
//!
//! **Opt-in via the `real-assets` Cargo feature** — the catalogue is
//! vendor-specific (industrial-arm + tool family) and will NOT ship with
//! the open-source toolchain. Drop `--features real-assets` from CI
//! when those URDFs are pruned, and (optionally) delete this file.
//!
//! Run locally with:
//!     cargo test --features real-assets --test real_assets
//!
//! Even when the feature is on, missing files are skipped at runtime
//! with a `Skipping real-asset test — missing under …` hint, so a
//! partial checkout doesn't masquerade as a failing implementation.

#![cfg(feature = "real-assets")]

use std::path::{Path, PathBuf};

use urdf_robo_dresser::catalogue;
use urdf_robo_dresser::collision::{generate_hulls, HullBuffer};
use urdf_robo_dresser::merger::merge;
use urdf_robo_dresser::parser::parse_urdf;
use urdf_robo_dresser::serializer::serialize_urdf;
use urdf_robo_dresser::validator::validate;
use urdf_robo_dresser::{MergeOptions, Origin, Severity};

fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("assets")
}

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

const KR240_URDF: &str = "urdf-robot-collection/kr240r2900/kr240r2900.urdf";
const GREIFER_URDF: &str =
    "urdf-tool-collection/GREIFER_URW1_OP3210_R01G1/GREIFER_URW1_OP3210_R01G1.urdf";
const RIVET_URDF: &str = "urdf-tool-collection/RIVET_DFB_956/RIVET_DFB_956.urdf";

/// Catalogue scanner picks up the real KR240 + every GREIFER variant.
#[test]
fn catalogue_scan_discovers_real_assets() {
    if !assets_available(&[Path::new("urdf-robot-collection"), Path::new("urdf-tool-collection")]) {
        return;
    }
    let cat = catalogue::scan(&assets_root()).expect("scan ok");
    assert!(
        cat.robots.iter().any(|r| r.name == "kr240r2900"),
        "expected kr240r2900 in robots: {:?}",
        cat.robots.iter().map(|r| &r.name).collect::<Vec<_>>(),
    );
    assert!(
        cat.tools.iter().any(|t| t.name == "GREIFER_URW1_OP3210_R01G1"),
        "expected GREIFER_URW1_OP3210_R01G1 in tools",
    );
}

/// Real KR240 URDF parses and validates without errors.
#[test]
fn kr240_parses_and_validates() {
    if !assets_available(&[Path::new(KR240_URDF)]) {
        return;
    }
    let path = assets_root().join(KR240_URDF);
    let urdf = parse_urdf(&path).expect("parse kr240");
    assert!(!urdf.links.is_empty(), "KR240 must have links");
    assert!(!urdf.joints.is_empty(), "KR240 must have joints");

    let diags = validate(&urdf, &path, false);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "KR240 URDF must have zero validation errors, got: {:?}",
        errors,
    );
}

/// Real GREIFER tool URDF parses + validates as a tool.
#[test]
fn greifer_parses_and_validates_as_tool() {
    if !assets_available(&[Path::new(GREIFER_URDF)]) {
        return;
    }
    let path = assets_root().join(GREIFER_URDF);
    let urdf = parse_urdf(&path).expect("parse greifer");
    let diags = validate(&urdf, &path, /* is_tool = */ true);
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "GREIFER tool URDF must have zero validation errors, got: {:?}",
        errors,
    );
}

/// End-to-end dress: KR240 + GREIFER → dressed URDF written to disk.
/// Mirrors what `urdf-robo-dresser dress --robot kr240r2900 --tool
/// GREIFER_URW1_OP3210_R01G1 --output ...` does on the CLI.
#[test]
fn dress_kr240_with_greifer_end_to_end() {
    if !assets_available(&[Path::new(KR240_URDF), Path::new(GREIFER_URDF)]) {
        return;
    }
    let robot_path = assets_root().join(KR240_URDF);
    let tool_path = assets_root().join(GREIFER_URDF);
    let out_dir = tempfile::tempdir().unwrap();

    let base = parse_urdf(&robot_path).unwrap();
    let mut tool = parse_urdf(&tool_path).unwrap();

    // Generate convex-hull collisions (matches CLI default `medium`).
    let robot_urdf_dir = robot_path.parent().unwrap().to_path_buf();
    let tool_urdf_dir = tool_path.parent().unwrap().to_path_buf();
    let _ = generate_hulls(
        &mut tool,
        &tool_urdf_dir,
        out_dir.path(),
        "tool",
        HullBuffer::Medium,
    );
    let mut base_with_hulls = base.clone();
    let _ = generate_hulls(
        &mut base_with_hulls,
        &robot_urdf_dir,
        out_dir.path(),
        "robot",
        HullBuffer::Medium,
    );

    let opts = MergeOptions {
        attach_to: None,
        orient: [0, 0, 0],
        offset: Origin::default(),
        tool_prefix: "tool__".to_string(),
        output_dir: out_dir.path().to_path_buf(),
        robot_urdf_dir,
        tool_urdf_dir,
    };
    let dressed = merge(&base_with_hulls, &tool, &opts).expect("merge ok");

    let xml_bytes = serialize_urdf(&dressed);
    let xml = String::from_utf8(xml_bytes).expect("URDF output must be UTF-8");
    let out_file = out_dir.path().join("dressed.urdf");
    std::fs::write(&out_file, &xml).unwrap();
    assert!(out_file.exists());
    assert!(xml.contains("<robot"), "serialised output must be a URDF");

    // The merger flattens tool geometry onto the flange link rather
    // than adding new links, so we don't compare link counts. Instead
    // verify the dressed URDF has more visual/collision meshes than the
    // bare robot — proves the tool's geometry actually got merged in.
    let dressed_mesh_count = xml.matches("<mesh ").count();
    let base_xml_bytes = serialize_urdf(&base);
    let base_xml = String::from_utf8(base_xml_bytes).expect("UTF-8");
    let base_mesh_count = base_xml.matches("<mesh ").count();
    assert!(
        dressed_mesh_count > base_mesh_count,
        "dressed URDF must include tool meshes on top of robot meshes ({} <= {})",
        dressed_mesh_count,
        base_mesh_count,
    );
}

/// Two different tools merged with the same robot each produce a
/// distinct dressed URDF whose mesh set exceeds the bare robot's.
/// Catches catastrophic regressions in the merger (e.g. tool geometry
/// dropped silently, hash collision in the inertia override path).
#[test]
fn dressing_multiple_tools_against_same_robot() {
    if !assets_available(&[
        Path::new(KR240_URDF),
        Path::new(GREIFER_URDF),
        Path::new(RIVET_URDF),
    ]) {
        return;
    }
    let robot = parse_urdf(&assets_root().join(KR240_URDF)).unwrap();
    let base_mesh_count =
        String::from_utf8(serialize_urdf(&robot)).unwrap().matches("<mesh ").count();
    let robot_dir = assets_root().join(KR240_URDF).parent().unwrap().to_path_buf();

    for tool_rel in [GREIFER_URDF, RIVET_URDF] {
        let tool_path = assets_root().join(tool_rel);
        let tool = parse_urdf(&tool_path).unwrap();
        let out_dir = tempfile::tempdir().unwrap();
        let opts = MergeOptions {
            attach_to: None,
            orient: [0, 0, 0],
            offset: Origin::default(),
            tool_prefix: "tool__".to_string(),
            output_dir: out_dir.path().to_path_buf(),
            robot_urdf_dir: robot_dir.clone(),
            tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        };
        let dressed = merge(&robot, &tool, &opts).expect("merge ok");
        let dressed_mesh_count = String::from_utf8(serialize_urdf(&dressed))
            .unwrap()
            .matches("<mesh ")
            .count();
        assert!(
            dressed_mesh_count > base_mesh_count,
            "merging {} must add tool meshes on top of the robot's ({} <= {})",
            tool_rel,
            dressed_mesh_count,
            base_mesh_count,
        );
    }
}
