//! Real-asset integration tests against the in-repo `../assets/` capture + cell
//! manifest, so they catch regressions a synthetic test would miss (the real
//! 7-robot capture, the 8-robot cell, the authored ros-topics.txt).
//!
//! **Opt-in via the `real-assets` Cargo feature**, since those captures are
//! vendor-specific and will NOT ship with the open-source release. Even when
//! the feature is on, missing files are skipped at runtime, so a partial
//! checkout doesn't masquerade as a failing implementation.
//!
//! Run locally with:
//!     cargo test --features real-assets --test real_assets

#![cfg(feature = "real-assets")]

use std::path::{Path, PathBuf};

use robo_ros_runner::{inspect_csv, resolve_topics, CsvFormat, RunConfig};

fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("assets")
}

fn available(paths: &[&Path]) -> bool {
    for p in paths {
        if !p.exists() {
            eprintln!("Skipping real-asset test — missing {}", p.display());
            return false;
        }
    }
    true
}

// FR-104 against the real capture: 7 robots, 6 axes, ~420 s, ~99,774 rows.
#[test]
fn fr_104_real_capture_schema() {
    let csv = assets_root().join("data-captures/joint_angles_timeline.csv");
    if !available(&[&csv]) {
        return;
    }
    let s = inspect_csv(&csv, CsvFormat::Wide).unwrap();
    assert_eq!(s.robots.len(), 7, "expected 7 robots in the capture");
    assert_eq!(s.axes_per_robot, 6);
    let rows = s.row_count.unwrap();
    assert!(rows > 99_000 && rows < 100_500, "row count {} off expected ~99,774", rows);
    let t_max = s.t_max.unwrap();
    assert!(t_max > 400.0 && t_max < 440.0, "t_max {} off expected ~420 s", t_max);
    // The 8th robot has a topic but no capture columns.
    assert!(!s.robots.iter().any(|r| r == "123180R01"));
}

// FR-302: resolved topics equal the authored ros-topics.txt for the real cell.
#[test]
fn fr_302_topics_match_capture() {
    let cell = assets_root().join("fitted-robots/k8urw11-cell.yaml");
    let csv = assets_root().join("data-captures/joint_angles_timeline.csv");
    let topics_txt = assets_root().join("data-captures/ros-topics.txt");
    if !available(&[&cell, &csv, &topics_txt]) {
        return;
    }

    let scene = robo_fitting_room::parse_scene(&cell).unwrap();
    let csv_robots = inspect_csv(&csv, CsvFormat::Wide).unwrap().robots;
    let cfg = RunConfig::from_paths(cell, csv);
    let resolved = resolve_topics(&scene, &cfg, &csv_robots).unwrap();

    let expected: Vec<String> = std::fs::read_to_string(&topics_txt)
        .unwrap()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let actual: Vec<String> = resolved.iter().map(|t| t.topic.clone()).collect();

    assert_eq!(actual, expected, "resolved topics drifted from ros-topics.txt");

    // The 8th robot (123180R01) is in the cell + topics but absent from the CSV.
    let r8 = resolved.iter().find(|t| t.robot_id == "123180R01").unwrap();
    assert!(!r8.has_csv_data);
}

// FR-200/500: every robot's dressed URDF yields six actuated joints and a plan.
#[test]
fn fr_500_real_cell_builds_plan() {
    let cell = assets_root().join("fitted-robots/k8urw11-cell.yaml");
    let csv = assets_root().join("data-captures/joint_angles_timeline.csv");
    if !available(&[&cell, &csv]) {
        return;
    }
    let cfg = RunConfig::from_paths(cell, csv);
    let plan = robo_ros_runner::build_plan(&cfg).unwrap();
    // 7 robots have capture data → 7 robot plans; all six joint names a1..a6.
    assert_eq!(plan.robots.len(), 7);
    for rp in plan.robots.values() {
        assert_eq!(rp.limits.names[0], "joint_a1");
        assert_eq!(rp.limits.names[5], "joint_a6");
    }
}
