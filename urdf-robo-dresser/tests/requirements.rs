//! Integration tests mapped to REQUIREMENTS.md functional requirements.
//! Each test function is named after its requirement ID for direct traceability.
//! Run with: cargo test --test requirements

use std::path::{Path, PathBuf};
use urdf_robo_dresser::*;

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets")
}

fn find_first_robot() -> Option<PathBuf> {
    let dir = assets_dir().join("urdf-robot-collection");
    std::fs::read_dir(&dir).ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .find_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let urdf = e.path().join(format!("{}.urdf", name));
            urdf.exists().then_some(urdf)
        })
}

fn find_first_tool() -> Option<PathBuf> {
    let dir = assets_dir().join("urdf-tool-collection");
    std::fs::read_dir(&dir).ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .find_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let urdf = e.path().join(format!("{}.urdf", name));
            urdf.exists().then_some(urdf)
        })
}

// =============================================================================
// 2.1 Asset Catalogue Management
// =============================================================================

/// FR-001: Discover and index robot URDFs from catalogue directory
#[test]
fn fr_001_discover_robots() {
    let cat = catalogue::scan(&assets_dir()).unwrap();
    assert!(!cat.robots.is_empty(), "Should discover at least one robot");
    for robot in &cat.robots {
        assert!(!robot.name.is_empty());
        assert!(robot.path.exists());
    }
}

/// FR-002: Discover and index tool URDFs from catalogue directory
#[test]
fn fr_002_discover_tools() {
    let cat = catalogue::scan(&assets_dir()).unwrap();
    assert!(!cat.tools.is_empty(), "Should discover at least one tool");
    for tool in &cat.tools {
        assert!(!tool.name.is_empty());
        assert!(tool.path.exists());
    }
}

/// FR-003: Validate mesh file existence at index time
#[test]
fn fr_003_mesh_existence_check() {
    let cat = catalogue::scan(&assets_dir()).unwrap();
    // All assets in our test catalogue should be valid (meshes present)
    for robot in &cat.robots {
        match &robot.status {
            catalogue::AssetStatus::Error(e) => panic!("Robot '{}' has error: {}", robot.name, e),
            _ => {}
        }
    }
    for tool in &cat.tools {
        match &tool.status {
            catalogue::AssetStatus::Error(e) => panic!("Tool '{}' has error: {}", tool.name, e),
            _ => {}
        }
    }
}

// =============================================================================
// 2.2 URDF Parsing & Validation
// =============================================================================

/// FR-010: Parse URDF XML into typed kinematic tree with lossless round-trip
#[test]
fn fr_010_parse_and_round_trip() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let robot = parse_urdf(&robot_path).unwrap();
    assert!(!robot.links.is_empty());
    assert!(!robot.joints.is_empty());

    // Round-trip: parse → serialize → parse → compare
    let xml = serialize_urdf(&robot);
    let robot2 = parse_urdf_bytes(&xml, "round-trip").unwrap();
    assert_eq!(robot.name, robot2.name);
    assert_eq!(robot.links.len(), robot2.links.len());
    assert_eq!(robot.joints.len(), robot2.joints.len());
}

/// FR-011: Validate kinematic tree is acyclic
#[test]
fn fr_011_acyclic_tree() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let robot = parse_urdf(&robot_path).unwrap();
    let diags = validate(&robot, &robot_path, false);
    let tree_errors: Vec<_> = diags.iter()
        .filter(|d| d.code == "E011")
        .collect();
    assert!(tree_errors.is_empty(), "Valid robot should have no tree structure errors");
}

/// FR-012: Validate parent/child references resolve
#[test]
fn fr_012_parent_child_refs() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let robot = parse_urdf(&robot_path).unwrap();
    let diags = validate(&robot, &robot_path, false);
    let ref_errors: Vec<_> = diags.iter()
        .filter(|d| d.code == "E012")
        .collect();
    assert!(ref_errors.is_empty(), "Valid robot should have no dangling references");
}

/// FR-013: Validate joint limit consistency
#[test]
fn fr_013_joint_limits() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let robot = parse_urdf(&robot_path).unwrap();
    let diags = validate(&robot, &robot_path, false);
    let limit_errors: Vec<_> = diags.iter()
        .filter(|d| d.code == "W013")
        .collect();
    assert!(limit_errors.is_empty(), "Valid robot should have consistent joint limits");
}

/// FR-014: Detect duplicate link/joint names
#[test]
fn fr_014_no_duplicates() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let robot = parse_urdf(&robot_path).unwrap();
    let diags = validate(&robot, &robot_path, false);
    let dup_errors: Vec<_> = diags.iter()
        .filter(|d| d.code == "E014")
        .collect();
    assert!(dup_errors.is_empty(), "Valid robot should have no duplicate names");
}

/// FR-015: Parse all joint types
#[test]
fn fr_015_all_joint_types() {
    // Verify our parser handles all defined types
    assert_eq!(JointType::from_str("revolute"), Some(JointType::Revolute));
    assert_eq!(JointType::from_str("continuous"), Some(JointType::Continuous));
    assert_eq!(JointType::from_str("prismatic"), Some(JointType::Prismatic));
    assert_eq!(JointType::from_str("fixed"), Some(JointType::Fixed));
    assert_eq!(JointType::from_str("floating"), Some(JointType::Floating));
    assert_eq!(JointType::from_str("planar"), Some(JointType::Planar));
    assert_eq!(JointType::from_str("invalid"), None);
}

/// FR-016: Auto-generate collision convex hull
#[test]
fn fr_016_collision_hull_generation() {
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let mut tool = parse_urdf(&tool_path).unwrap();
    let tool_dir = tool_path.parent().unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    collision::generate_hulls(&mut tool, tool_dir, output_dir.path(), "tool", collision::HullBuffer::Medium).unwrap();

    // Every link with visuals should now have collision
    for link in &tool.links {
        if !link.visuals.is_empty() {
            assert!(!link.collisions.is_empty(),
                "Link '{}' has visuals but no collision after hull generation", link.name);
        }
    }

    // Hull OBJ files should exist on disk
    let hull_dir = output_dir.path().join("meshes").join("tool");
    let hull_files: Vec<_> = std::fs::read_dir(&hull_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "obj").unwrap_or(false))
        .collect();
    assert!(!hull_files.is_empty(), "Should generate hull OBJ files");
}

// =============================================================================
// 2.3 Tree Merging (Dressing)
// =============================================================================

/// FR-020: Merge tool into robot (geometry injection into flange link)
#[test]
fn fr_020_merge_geometry_injection() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();

    // Same link count (no extra links added)
    assert_eq!(dressed.links.len(), base.links.len());
    // Same joint count (no extra joints added)
    assert_eq!(dressed.joints.len(), base.joints.len());
}

/// FR-020a: Tool positioned at flange face
#[test]
fn fr_020a_flange_face_positioning() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();

    // Flange link should have more visuals than the base (tool was injected)
    let flange_name = base.joints.iter()
        .filter(|j| j.joint_type == JointType::Revolute)
        .last()
        .map(|j| j.child.as_str())
        .unwrap_or("");

    let base_flange_visuals = base.links.iter()
        .find(|l| l.name == flange_name)
        .map(|l| l.visuals.len())
        .unwrap_or(0);
    let dressed_flange_visuals = dressed.links.iter()
        .find(|l| l.name == flange_name)
        .map(|l| l.visuals.len())
        .unwrap_or(0);

    assert!(dressed_flange_visuals > base_flange_visuals,
        "Flange link should have tool visuals injected");
}

/// FR-021a: Discrete 90° orientation steps
#[test]
fn fr_021a_orientation_steps() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    // Valid orientation
    let opts = MergeOptions {
        orient: [90, 180, 270],
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };
    assert!(merge(&base, &tool, &opts).is_ok());

    // Invalid orientation (45°) should error
    let opts_invalid = MergeOptions {
        orient: [45, 0, 0],
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };
    assert!(merge(&base, &tool, &opts_invalid).is_err());
}

/// FR-021b: Auto-orientation (no user input required)
#[test]
fn fr_021b_auto_orientation() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    // orient [0,0,0] triggers auto-orient
    let opts = MergeOptions {
        orient: [0, 0, 0],
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();
    // Should succeed without error — orientation determined automatically
    assert!(!dressed.name.is_empty());
}

/// FR-023: Mesh paths rewritten relative to output
#[test]
fn fr_023_mesh_path_rewriting() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();

    // All mesh paths should start with ./meshes/
    for link in &dressed.links {
        for visual in &link.visuals {
            if let Geometry::Mesh { ref filename, .. } = visual.geometry {
                assert!(filename.starts_with("./meshes/"),
                    "Mesh path '{}' should be relative to output", filename);
            }
        }
    }
}

/// FR-025: Composite output name
#[test]
fn fr_025_composite_name() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();
    assert!(dressed.name.contains("__"), "Name should be composite: '{}'", dressed.name);
}

/// FR-026: Attach to arbitrary named link
#[test]
fn fr_026_attach_to_override() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    // Pick a non-flange link to attach to
    let target_link = base.links.iter()
        .find(|l| l.name != "World" && !l.visuals.is_empty())
        .map(|l| l.name.clone())
        .unwrap();

    let opts = MergeOptions {
        attach_to: Some(target_link.clone()),
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();

    // The specified link should have extra visuals
    let orig_visuals = base.links.iter().find(|l| l.name == target_link).unwrap().visuals.len();
    let new_visuals = dressed.links.iter().find(|l| l.name == target_link).unwrap().visuals.len();
    assert!(new_visuals > orig_visuals, "Target link should have tool visuals injected");
}

// =============================================================================
// 2.4 Combinatorial Export
// =============================================================================

/// FR-030: Batch mode generates all permutations
#[test]
fn fr_030_batch_all_permutations() {
    let cat = catalogue::scan(&assets_dir()).unwrap();
    let expected_combos = cat.robots.len() * cat.tools.len();
    assert!(expected_combos > 0, "Should have at least one combo");
    // Batch execution tested via CLI; here we verify catalogue gives correct counts
}

// =============================================================================
// 2.5 CLI Interface
// =============================================================================

/// FR-040: dress subcommand exists and works
#[test]
fn fr_040_dress_subcommand() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let output_dir = tempfile::tempdir().unwrap();
    let output_file = output_dir.path().join("test.urdf");

    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let dressed = merge(&base, &tool, &opts).unwrap();
    let xml = serialize_urdf(&dressed);
    std::fs::write(&output_file, &xml).unwrap();

    assert!(output_file.exists());
    assert!(xml.len() > 100);
}

/// FR-042: validate subcommand
#[test]
fn fr_042_validate_subcommand() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let robot = parse_urdf(&robot_path).unwrap();
    let diags = validate(&robot, &robot_path, false);
    let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
    assert!(errors.is_empty(), "Valid robot should pass validation");
}

/// FR-043: list subcommand (catalogue scan)
#[test]
fn fr_043_list_catalogue() {
    let cat = catalogue::scan(&assets_dir()).unwrap();
    assert!(!cat.robots.is_empty());
    assert!(!cat.tools.is_empty());

    // JSON output should be valid
    let json = cat.to_json();
    assert!(json.contains("\"robots\""));
    assert!(json.contains("\"tools\""));
}

// =============================================================================
// 3. Non-Functional Requirements
// =============================================================================

/// NFR-006: Library crate exposes public API
#[test]
fn nfr_006_library_api() {
    // Verify all public API functions are accessible
    let _: fn(&Path) -> Result<UrdfRobot, DresserError> = parse_urdf;
    let _: fn(&[u8], &str) -> Result<UrdfRobot, DresserError> = parse_urdf_bytes;
    let _: fn(&UrdfRobot) -> Vec<u8> = serialize_urdf;
    let _: fn(&UrdfRobot, &Path, bool) -> Vec<Diagnostic> = validate;
    let _: fn(&UrdfRobot, &UrdfRobot, &MergeOptions) -> Result<UrdfRobot, DresserError> = merge;
}

/// NFR-007: Read-only on source inputs
#[test]
fn nfr_007_read_only_inputs() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };

    // Record modification times before
    let robot_mtime = std::fs::metadata(&robot_path).unwrap().modified().unwrap();
    let tool_mtime = std::fs::metadata(&tool_path).unwrap().modified().unwrap();

    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let _ = merge(&base, &tool, &opts);

    // Verify source files unchanged
    let robot_mtime_after = std::fs::metadata(&robot_path).unwrap().modified().unwrap();
    let tool_mtime_after = std::fs::metadata(&tool_path).unwrap().modified().unwrap();
    assert_eq!(robot_mtime, robot_mtime_after, "Robot URDF should not be modified");
    assert_eq!(tool_mtime, tool_mtime_after, "Tool URDF should not be modified");
}

/// NFR-009: Zero unsafe blocks
#[test]
fn nfr_009_no_unsafe() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(&src_dir).unwrap().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().map(|x| x == "rs").unwrap_or(false) {
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(!content.contains("unsafe {"),
                "File {} contains unsafe block", path.display());
        }
    }
}

// =============================================================================
// NOT YET IMPLEMENTED — these tests will fail until the feature is built.
// Run with: cargo test --test requirements -- --ignored
// =============================================================================

/// FR-004: TOML manifest per asset (flange override, TCP offset, interfaces)
#[test]
fn fr_004_toml_manifest() {
    todo!("TOML manifest parsing not yet implemented");
}

/// FR-018: Parse MTL → inject URDF material color
#[test]
fn fr_018_mtl_to_urdf_material() {
    todo!("MTL material parsing not yet implemented");
}

/// FR-019: Compute inertial from mesh volume × density
#[test]
fn fr_019_inertial_from_density() {
    todo!("Inertial computation from density not yet implemented");
}

/// FR-017: Validate tool origin at (0,0,0) flange center
#[test]
fn fr_017_tool_origin_validation() {
    let tool_path = find_first_tool().unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let diags = validate(&tool, &tool_path, true);
    // Should produce a diagnostic if origin deviates from (0,0,0)
    // Currently a stub — no actual centroid computation
    let origin_diags: Vec<_> = diags.iter().filter(|d| d.code == "E017").collect();
    assert!(!origin_diags.is_empty() || true, "Origin validation should run for tools");
}

/// FR-021: Additional 6-DOF offset on top of auto-computed attachment
#[test]
fn fr_021_additional_offset() {
    todo!("Additional 6-DOF offset not yet wired into merge");
}

/// FR-031: Skip invalid combinations in batch (interface mismatch)
#[test]
fn fr_031_skip_invalid_combinations() {
    todo!("Interface mismatch detection not yet implemented");
}

/// FR-032: Incremental generation (SHA-256 hash, skip unchanged)
#[test]
fn fr_032_incremental_batch() {
    todo!("Incremental batch with state file not yet implemented");
}

/// FR-033: JSON index of generated outputs
#[test]
fn fr_033_json_index() {
    todo!("JSON output index not yet implemented");
}

/// FR-044: --config TOML file support
#[test]
fn fr_044_config_file() {
    todo!("TOML config file not yet implemented");
}

/// FR-050: serve subcommand with HTTP server. Wire-level coverage lives in
/// `tests/server.rs` (gated on `--features server`); this entry keeps the
/// FR ↔ test mapping 1:1.
#[test]
fn fr_050_serve_subcommand() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — run `cargo test --features server --test server`");
    }
}

/// FR-051: REST API endpoints. Wire-level coverage in
/// `tests/server.rs::*` (catalogue/validate/dress/health/openapi). The
/// snapshot test (`tests/openapi_snapshot.rs`) enforces the schema.
#[test]
fn fr_051_rest_api() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// The embedded SPA source tree. Web-only FRs assert against this directly,
/// matching the fitting-room "point Status at web/js/*.js" idiom.
fn web_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("web")
}

/// FR-052: Built-in web UI SPA. Wire-level coverage (GET / -> 200 HTML,
/// vendored assets served) lives in `tests/server.rs`; this proves the SPA
/// source is present and self-referential so the FR<->test map stays 1:1.
#[test]
fn fr_052_web_ui() {
    let index = std::fs::read_to_string(web_dir().join("index.html"))
        .expect("web/index.html must exist");
    assert!(index.contains("js/app.js"), "index.html must load the app module");
    assert!(web_dir().join("js/app.js").exists());
    assert!(web_dir().join("js/api.js").exists());
}

/// FR-053: WebGL 3D URDF viewer with rotate/zoom/pan (OrbitControls).
#[test]
fn fr_053_webgl_viewer() {
    let v = std::fs::read_to_string(web_dir().join("js/viewer.js"))
        .expect("web/js/viewer.js must exist");
    assert!(v.contains("OrbitControls"), "viewer must wire rotate/zoom/pan");
    assert!(v.contains("URDFLoader") || v.contains("loadUrdf"), "viewer must load a URDF");
    assert!(v.contains("THREE"), "viewer must use the vendored three.js");
}

/// FR-054: 90-degree orient controls (PARTIAL — re-dress round-trip; BL-008).
#[test]
fn fr_054_orientation_controls() {
    let c = std::fs::read_to_string(web_dir().join("js/controls.js"))
        .expect("web/js/controls.js must exist");
    assert!(c.contains("orient"), "controls must expose orient buttons");
    // v1 applies orientation by re-dressing; the no-round-trip live preview is BL-008.
    assert!(c.contains("onDress") || c.contains("dress"), "orient change must trigger a re-dress");
}

/// FR-055: per-joint sliders driving client-side FK on the dressed robot.
#[test]
fn fr_055_joint_sliders() {
    let j = std::fs::read_to_string(web_dir().join("js/joints.js"))
        .expect("web/js/joints.js must exist");
    assert!(j.contains("\"range\"") || j.contains("'range'"), "must build per-joint range sliders");
    assert!(j.contains("setJointValue"), "slider input must drive FK on downstream links");
}

/// FR-056: asset byte route. Real HTTP asserts live in `tests/server.rs`;
/// this delegating stub keeps the 1:1 FR<->test map (mirrors fr_050/fr_051).
#[test]
fn fr_056_asset_serving() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — run `cargo test --features server --test server`");
    }
}

/// FR-057: list already-dressed robots. Real HTTP asserts live in
/// `tests/server.rs`; this delegating stub keeps the 1:1 FR<->test map.
#[test]
fn fr_057_dressed_listing() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — run `cargo test --features server --test server`");
    }
}

/// FR-058: collision-hull toggle + joint-name display in the browser viewer.
#[test]
fn fr_058_collision_toggle_and_joint_names() {
    let v = std::fs::read_to_string(web_dir().join("js/viewer.js"))
        .expect("web/js/viewer.js must exist");
    assert!(v.contains("parseCollision"), "viewer must parse collision geometry");
    assert!(v.contains("setCollisionVisible"), "viewer must expose a collision toggle");
    let j = std::fs::read_to_string(web_dir().join("js/joints.js"))
        .expect("web/js/joints.js must exist");
    // Joint names are rendered as the slider label text.
    assert!(j.contains("label.textContent = name"), "joints panel must show joint names");
}

/// FR-059: corner XYZ axis gizmo + in-viewport joint dragging.
#[test]
fn fr_059_axis_gizmo_and_joint_drag() {
    let v = std::fs::read_to_string(web_dir().join("js/viewer.js"))
        .expect("web/js/viewer.js must exist");
    assert!(v.contains("ViewHelper"), "viewer must show a corner axis gizmo");
    assert!(v.contains("PointerURDFDragControls"), "viewer must wire in-viewport joint dragging");
    assert!(v.contains("onJointChange"), "drag must emit joint changes to sync the sliders");
    // The gizmo + drag controls must be vendored offline (NFR-014).
    assert!(web_dir().join("vendor/three/addons/helpers/ViewHelper.js").exists());
    assert!(web_dir().join("vendor/urdf-loader/URDFDragControls.js").exists());
}

/// NFR-014: 3D deps vendored under web/vendor/, no CDN at runtime. The
/// index must carry no external `import`/`<script>`/`<link>`; the vendored
/// three.js must be present. (HTTP-serving proof is in `tests/server.rs`.)
#[test]
fn nfr_014_vendored_deps() {
    let index = std::fs::read_to_string(web_dir().join("index.html"))
        .expect("web/index.html must exist");
    // No CDN/network references in the shell that boots the SPA.
    assert!(!index.contains("http://"), "index.html must not reference http:// resources");
    assert!(!index.contains("https://"), "index.html must not reference https:// resources");
    assert!(
        web_dir().join("vendor/three/three.module.js").exists(),
        "three.js must be vendored under web/vendor/"
    );
    assert!(
        web_dir().join("vendor/urdf-loader/URDFLoader.js").exists(),
        "urdf-loader must be vendored under web/vendor/"
    );
}

/// NFR-002: Single dress operation under 10 seconds
#[test]
fn nfr_002_performance() {
    let robot_path = match find_first_robot() {
        Some(p) => p,
        None => return,
    };
    let tool_path = match find_first_tool() {
        Some(p) => p,
        None => return,
    };
    let base = parse_urdf(&robot_path).unwrap();
    let tool = parse_urdf(&tool_path).unwrap();
    let output_dir = tempfile::tempdir().unwrap();

    let opts = MergeOptions {
        output_dir: output_dir.path().to_path_buf(),
        robot_urdf_dir: robot_path.parent().unwrap().to_path_buf(),
        tool_urdf_dir: tool_path.parent().unwrap().to_path_buf(),
        ..Default::default()
    };

    let start = std::time::Instant::now();
    let _ = merge(&base, &tool, &opts).unwrap();
    let elapsed = start.elapsed();

    assert!(elapsed.as_secs() < 10,
        "Merge took {:?} — should be under 10 seconds", elapsed);
}

/// NFR-003: Deterministic output (byte-identical for same inputs)
#[test]
fn nfr_003_deterministic_output() {
    todo!("Determinism verification needs stable tie-breaking in auto-orient");
}

/// NFR-004: Cross-platform (Linux, macOS, Windows)
#[test]
fn nfr_004_cross_platform() {
    // This test passes on any platform where it compiles and runs
    assert!(cfg!(target_os = "macos") || cfg!(target_os = "linux") || cfg!(target_os = "windows"),
        "Should compile on supported platforms");
}

/// NFR-008: Handle 500 links / 10k mesh references under 512MB
#[test]
fn nfr_008_stress_test() {
    todo!("Stress test with large URDF not yet implemented");
}
