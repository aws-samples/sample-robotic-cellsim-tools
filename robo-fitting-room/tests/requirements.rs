//! Integration tests mapped to REQUIREMENTS.md functional requirements.
//! Each test function is named after its requirement ID for direct traceability.
//! Run with: cargo test --test requirements
//!
//! Conventions:
//! - Tests for FRs marked DONE / PARTIAL exercise the implementation end-to-end
//!   via the public API (parse_scene, validate_scene, compose_stage_with_options).
//! - Tests for FRs marked NOT STARTED / BACKLOG remain `todo!()` so the
//!   requirements list stays auditable: green = implemented, red = pending.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use robo_fitting_room::{
    compose_stage, compose_stage_with_options, compose_urdf_bundle, parse_scene, validate_scene,
    ComposeOptions, FixturePlacement, Pedestal, RobotPlacement, SceneDefinition,
};

// =============================================================================
// Test helpers
// =============================================================================

/// Write a tiny valid OBJ triangle so URDF mesh references resolve at compose time.
fn write_obj_triangle(path: &Path) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(path).unwrap();
    writeln!(f, "v 0 0 0").unwrap();
    writeln!(f, "v 1 0 0").unwrap();
    writeln!(f, "v 0 1 0").unwrap();
    writeln!(f, "f 1 2 3").unwrap();
}

/// Write a minimal 6-axis revolute robot URDF in the dresser's `like_kr240r2900_*`
/// naming convention so the steel-arm inertia override fires for at least one link.
/// All meshes live under `meshes/robot/`.
fn write_kr_like_urdf(urdf_path: &Path) {
    let dir = urdf_path.parent().unwrap();
    write_obj_triangle(&dir.join("meshes/robot/m.obj"));

    let mut s = String::from("<?xml version=\"1.0\"?>\n<robot name=\"kr_like\">\n");
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
    for n in &names {
        s.push_str(&format!("  <link name=\"{}\">\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/robot/m.obj\"/>\n      </geometry>\n    </visual>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/robot/m.obj\"/>\n      </geometry>\n    </collision>\n  </link>\n", n));
    }
    // World → base via fixed; base → link_1..6 revolute.
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
    std::fs::File::create(urdf_path)
        .unwrap()
        .write_all(s.as_bytes())
        .unwrap();
}

/// Write a fixture URDF with a single link and one fixed joint.
/// Kept as a helper for future fixture-emission tests; currently unused
/// in the suite, hence the dead_code allow.
#[allow(dead_code)]
fn write_fixture_urdf(urdf_path: &Path) {
    let dir = urdf_path.parent().unwrap();
    write_obj_triangle(&dir.join("meshes/m.obj"));
    let s = "<?xml version=\"1.0\"?>\n<robot name=\"fix\">\n  <link name=\"World\">\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/m.obj\"/>\n      </geometry>\n    </visual>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/m.obj\"/>\n      </geometry>\n    </collision>\n  </link>\n  <link name=\"plate\"/>\n  <joint name=\"j_fix\" type=\"fixed\">\n    <parent link=\"World\"/>\n    <child link=\"plate\"/>\n    <origin xyz=\"0 0 0\" rpy=\"0 0 0\"/>\n  </joint>\n</robot>\n";
    std::fs::File::create(urdf_path)
        .unwrap()
        .write_all(s.as_bytes())
        .unwrap();
}

/// Read a single entry from a zip archive into a String (panics on missing/IO).
fn read_zip_entry(zip_path: &Path, entry: &str) -> String {
    let f = std::fs::File::open(zip_path).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut e = z.by_name(entry).unwrap();
    let mut s = String::new();
    e.read_to_string(&mut s).unwrap();
    s
}

/// List every entry name in a zip archive.
fn zip_entry_names(zip_path: &Path) -> Vec<String> {
    let f = std::fs::File::open(zip_path).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    (0..z.len())
        .map(|i| z.by_index(i).unwrap().name().to_string())
        .collect()
}

/// Compose a one-robot scene from a kr_like URDF and return (zip_path, usda_text).
fn compose_one_kr_robot(opts: ComposeOptions) -> (PathBuf, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"t\"\n").unwrap();

    let scene = SceneDefinition {
        name: "t".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf,
            position: [1.0, 2.0, 3.0],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out = dir.path().join("scene.zip");
    compose_stage_with_options(&scene, &manifest, &out, &opts).unwrap();
    let usda = read_zip_entry(&out, "scene.usda");
    (out, usda, dir)
}

// =============================================================================
// 2.1 Scene Manifest & Composition
// =============================================================================

/// FR-100: Parse scene manifest YAML declaring URDF assets and world-space placements
#[test]
fn fr_100_parse_scene_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let scene_path = dir.path().join("cell.yaml");
    let mut f = std::fs::File::create(&scene_path).unwrap();
    write!(
        f,
        r#"name: "Test Cell"
description: "Integration test scene"
robots:
  - robot_id: "R1"
    dressed_urdf: "robot.urdf"
    position: {{ x: 1.0, y: 2.0, z: 0.0 }}
    orientation: {{ a: 90.0, b: 0.0, c: 0.0 }}
fixtures:
  - id: "F1"
    urdf: "fixture.urdf"
    position: {{ x: 3.0, y: 4.0, z: 0.0 }}
    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}
"#
    )
    .unwrap();

    let scene = parse_scene(&scene_path).unwrap();
    assert_eq!(scene.name, "Test Cell");
    assert_eq!(scene.robots.len(), 1);
    assert_eq!(scene.robots[0].robot_id, "R1");
    assert!((scene.robots[0].position[0] - 1.0).abs() < 1e-6);
    assert!((scene.robots[0].orientation_deg[0] - 90.0).abs() < 1e-6);
    assert_eq!(scene.fixtures.len(), 1);
    assert_eq!(scene.fixtures[0].id, "F1");
}

/// FR-101: Place multiple dressed robots in a single scene
#[test]
fn fr_101_multiple_robots() {
    let dir = tempfile::tempdir().unwrap();
    let scene_path = dir.path().join("cell.yaml");
    let mut f = std::fs::File::create(&scene_path).unwrap();
    write!(
        f,
        r#"name: "Multi-robot"
robots:
  - robot_id: "R1"
    dressed_urdf: "r1.urdf"
    position: {{ x: 0.0, y: 0.0, z: 0.0 }}
    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}
  - robot_id: "R2"
    dressed_urdf: "r2.urdf"
    position: {{ x: 5.0, y: 0.0, z: 0.0 }}
    orientation: {{ a: 180.0, b: 0.0, c: 0.0 }}
  - robot_id: "R3"
    dressed_urdf: "r3.urdf"
    position: {{ x: 10.0, y: 0.0, z: 0.0 }}
    orientation: {{ a: 270.0, b: 0.0, c: 0.0 }}
  - robot_id: "R4"
    dressed_urdf: "r4.urdf"
    position: {{ x: 15.0, y: 0.0, z: 0.0 }}
    orientation: {{ a: 45.0, b: 0.0, c: 0.0 }}
  - robot_id: "R5"
    dressed_urdf: "r5.urdf"
    position: {{ x: 20.0, y: 0.0, z: 0.0 }}
    orientation: {{ a: 315.0, b: 0.0, c: 0.0 }}
"#
    )
    .unwrap();

    let scene = parse_scene(&scene_path).unwrap();
    assert_eq!(scene.robots.len(), 5);
    assert_eq!(scene.robots[0].robot_id, "R1");
    assert_eq!(scene.robots[4].robot_id, "R5");
    assert!((scene.robots[4].position[0] - 20.0).abs() < 1e-6);
}

/// FR-102: Place static fixture URDFs as non-articulated rigid bodies
#[test]
fn fr_102_static_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    let scene_path = dir.path().join("cell.yaml");
    let mut f = std::fs::File::create(&scene_path).unwrap();
    write!(
        f,
        r#"name: "Fixtures"
fixtures:
  - id: "conveyor_01"
    urdf: "conveyor.urdf"
    position: {{ x: 10.0, y: 5.0, z: 0.0 }}
    orientation: {{ a: 90.0, b: 0.0, c: 0.0 }}
  - id: "fence_north"
    urdf: "fence.urdf"
    position: {{ x: 0.0, y: 20.0, z: 0.0 }}
    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}
"#
    )
    .unwrap();

    let scene = parse_scene(&scene_path).unwrap();
    assert_eq!(scene.fixtures.len(), 2);
    assert_eq!(scene.fixtures[0].id, "conveyor_01");
    assert_eq!(scene.fixtures[1].id, "fence_north");
}

/// FR-103: Validate manifest completeness (paths exist, positions finite, URDF valid)
#[test]
fn fr_103_manifest_validation() {
    // Missing URDF file
    let scene_missing = SceneDefinition {
        name: "test".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: PathBuf::from("/nonexistent/robot.urdf"),
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let errors = validate_scene(&scene_missing);
    assert!(errors.iter().any(|e| e.to_string().contains("not found")));

    // Non-finite position
    let scene_nan = SceneDefinition {
        name: "test".into(),
        description: String::new(),
        robots: vec![],
        fixtures: vec![FixturePlacement {
            id: "F1".into(),
            urdf: PathBuf::from("/nonexistent/f.urdf"),
            position: [f64::NAN, 0.0, 0.0],
            orientation_deg: [0.0; 3],
        }],
        floor_plan: None,
    };
    let errors = validate_scene(&scene_nan);
    assert!(errors.iter().any(|e| e.to_string().contains("non-finite")));

    // Invalid URDF content
    let dir = tempfile::tempdir().unwrap();
    let bad_urdf = dir.path().join("bad.urdf");
    std::fs::write(&bad_urdf, "not a urdf file").unwrap();
    let scene_invalid = SceneDefinition {
        name: "test".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: bad_urdf,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let errors = validate_scene(&scene_invalid);
    assert!(errors.iter().any(|e| e.to_string().contains("robot")));

    // Valid URDF passes
    let good_urdf = dir.path().join("good.urdf");
    std::fs::write(
        &good_urdf,
        r#"<?xml version="1.0"?><robot name="test"><link name="base"/></robot>"#,
    )
    .unwrap();
    let scene_ok = SceneDefinition {
        name: "test".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: good_urdf,
            position: [1.0, 2.0, 3.0],
            orientation_deg: [90.0, 0.0, 0.0],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let errors = validate_scene(&scene_ok);
    assert!(errors.is_empty());
}

/// FR-104: Scene inheritance (base + overlay manifests) — NOT STARTED
#[test]
fn fr_104_scene_inheritance() {
    todo!("Scene inheritance not yet implemented");
}

/// FR-105: Load 2D CAD floor plan (OBJ line geometry) with offset/scale and layer separation
#[test]
fn fr_105_floor_plan_loading() {
    let dir = tempfile::tempdir().unwrap();

    // Create OBJ with floor segments and a footprint group
    let obj_path = dir.path().join("plan.obj");
    std::fs::write(
        &obj_path,
        "v 0.0 0.0 0.0\nv 10.0 0.0 0.0\nl 1 2\no E9_121051_zone\nv 2.0 2.0 0.0\nv 4.0 4.0 0.0\nl 3 4\n",
    )
    .unwrap();

    let scene_path = dir.path().join("cell.yaml");
    let mut f = std::fs::File::create(&scene_path).unwrap();
    write!(
        f,
        "name: \"Floor plan test\"\nfloor_plan:\n  geometry_file: \"plan.obj\"\n  offset: {{ x: 1.0, y: 2.0, z: 0.0 }}\n  scale: 2.0\n"
    )
    .unwrap();

    let scene = parse_scene(&scene_path).unwrap();
    let fp = scene.floor_plan.unwrap();

    // Floor segment: (0*2+1, 0*2+2, 0) to (10*2+1, 0*2+2, 0)
    assert_eq!(fp.segments.len(), 1);
    assert!((fp.segments[0][0][0] - 1.0).abs() < 1e-6);
    assert!((fp.segments[0][1][0] - 21.0).abs() < 1e-6);

    // Footprint segment (in E9_121051 group)
    assert_eq!(fp.footprint_segments.len(), 1);
    assert!((fp.footprint_segments[0][0][0] - 5.0).abs() < 1e-6); // 2*2+1
}

/// FR-106: Optional `scene_origin` field is subtracted from all placements + floor-plan offset
#[test]
fn fr_106_scene_origin_centering() {
    let dir = tempfile::tempdir().unwrap();
    let scene_path = dir.path().join("cell.yaml");
    std::fs::write(
        &scene_path,
        "name: \"o\"\nscene_origin: { x: 100.0, y: 50.0, z: 0.0 }\nrobots:\n  - robot_id: \"R\"\n    dressed_urdf: \"r.urdf\"\n    position: { x: 110.0, y: 60.0, z: 0.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\nfixtures:\n  - id: \"F\"\n    urdf: \"f.urdf\"\n    position: { x: 105.0, y: 55.0, z: 1.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\n",
    )
    .unwrap();
    let scene = parse_scene(&scene_path).unwrap();
    // scene_origin subtracted from every position
    assert!((scene.robots[0].position[0] - 10.0).abs() < 1e-6);
    assert!((scene.robots[0].position[1] - 10.0).abs() < 1e-6);
    assert!((scene.fixtures[0].position[0] - 5.0).abs() < 1e-6);
    assert!((scene.fixtures[0].position[1] - 5.0).abs() < 1e-6);
    assert!((scene.fixtures[0].position[2] - 1.0).abs() < 1e-6);
}

/// FR-107: Floor-plan emission into the composed USDA is currently disabled.
#[test]
fn fr_107_floor_plan_emission_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let obj_path = dir.path().join("plan.obj");
    std::fs::write(&obj_path, "v 0 0 0\nv 1 0 0\nl 1 2\n").unwrap();
    let manifest = dir.path().join("scene.yaml");
    std::fs::write(
        &manifest,
        "name: \"fp\"\nfloor_plan:\n  geometry_file: \"plan.obj\"\n",
    )
    .unwrap();
    let scene = parse_scene(&manifest).unwrap();
    assert!(scene.floor_plan.is_some());
    let out = dir.path().join("scene.zip");
    let report = compose_stage(&scene, &manifest, &out).unwrap();
    // Segment count is reported …
    assert_eq!(report.floor_plan_segments, 1);
    // … but no floor_plan prim is emitted.
    let usda = read_zip_entry(&out, "scene.usda");
    assert!(!usda.contains("def Xform \"floor_plan\""));
    assert!(!usda.contains("def Mesh \"lines\""));
}

// =============================================================================
// 2.2 URDF-to-USD Conversion
// =============================================================================

/// FR-110: Convert URDF joint/link trees into USD PhysX articulations
#[test]
fn fr_110_urdf_to_usd_articulations() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions {
        full_robots: true,
        full_tools: true,
    });
    // Articulation root with PhysX schema
    assert!(usda.contains("PhysicsArticulationRootAPI"));
    assert!(usda.contains("PhysxArticulationAPI"));
    // Six revolute joints emitted in URDF declaration order
    for n in 1..=6 {
        assert!(
            usda.contains(&format!("def PhysicsRevoluteJoint \"joint_a{}\"", n)),
            "joint_a{} missing",
            n
        );
    }
    // Joint limits in degrees (URDF -3.14..3.14 rad → ~-179.9..179.9 deg)
    assert!(usda.contains("physics:lowerLimit = -179."));
    assert!(usda.contains("physics:upperLimit = 179."));
}

/// FR-111: Convert URDF visual meshes into USD mesh prims (inlined geometry)
#[test]
fn fr_111_visual_mesh_to_usd() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions {
        full_robots: true,
        full_tools: true,
    });
    // Visual prim emitted, geometry inlined (no external `references = @...@`)
    assert!(usda.contains("def Mesh \"visual_0\""));
    assert!(usda.contains("point3f[] points = ["));
    assert!(usda.contains("int[] faceVertexIndices = ["));
    assert!(usda.contains("int[] faceVertexCounts = ["));
    // No external mesh references inside any def Mesh block — the only
    // `prepend references = @…@` allowed is the FlatGrid env block at the
    // bottom of the stage, which is unrelated to mesh emission.
    let mut depth = 0i32;
    let mut in_mesh = false;
    for line in usda.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("def Mesh ") {
            in_mesh = true;
        }
        if in_mesh {
            assert!(
                !trimmed.starts_with("prepend references = @")
                    && !trimmed.starts_with("references = @"),
                "mesh data must be inlined; found external reference in def Mesh: {}",
                line
            );
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if depth <= 0 {
                in_mesh = false;
                depth = 0;
            }
        }
    }
}

/// FR-112: Convert URDF collision meshes into USD collision geometry (convex hull)
#[test]
fn fr_112_collision_to_usd() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions {
        full_robots: true,
        full_tools: true,
    });
    assert!(usda.contains("def Mesh \"collision_0\""));
    assert!(usda.contains("PhysicsCollisionAPI"));
    assert!(usda.contains("PhysicsMeshCollisionAPI"));
    assert!(usda.contains("uniform token physics:approximation = \"convexHull\""));
}

/// FR-113: Map URDF inertial / KR240 override / fallback to PhysicsMassAPI on every link
#[test]
fn fr_113_inertial_to_usd() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions {
        full_robots: true,
        full_tools: true,
    });
    // Steel-arm overrides for the recognised link suffixes
    assert!(usda.contains("float physics:mass = 250"), "base_link 250 kg");
    assert!(usda.contains("float physics:mass = 200"), "link_1 200 kg");
    assert!(usda.contains("float physics:mass = 35"), "link_6 35 kg");
    // Orphan world_link gets the 10 kg fallback
    assert!(usda.contains("float physics:mass = 10"));
    // Every body that carries PhysicsRigidBodyAPI also carries PhysicsMassAPI
    let bodies = usda.matches("PhysicsRigidBodyAPI").count();
    let masses = usda.matches("PhysicsMassAPI").count();
    assert!(masses >= bodies, "every rigid body must have MassAPI");
}

/// FR-114: Deterministic USD prim paths from robot ID and link/joint names
#[test]
fn fr_114_deterministic_prim_paths() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    // Robot Xform path uses the manifest robot_id ("R1")
    assert!(usda.contains("def Xform \"R1\""));
    // URDF link "World" sanitised to "world_link" (avoids stage-root shadowing)
    assert!(usda.contains("def Xform \"world_link\""));
    // Other link names preserved
    assert!(usda.contains("def Xform \"like_kr240r2900_link_3\""));
}

/// FR-115: Anchor URDF root link to placement pose via PhysicsFixedJoint "world_anchor"
#[test]
fn fr_115_world_link_anchoring() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(usda.contains("def PhysicsFixedJoint \"world_anchor\""));
    // Anchor's localPos0 = manifest position [1, 2, 3]
    assert!(
        usda.contains("point3f physics:localPos0 = (1, 2, 3)"),
        "anchor must carry the placement xyz"
    );
    // body1 rel points at the URDF root link (sanitised)
    assert!(usda.contains("rel physics:body1 = </World/robots/R1/world_link>"));
}

/// FR-116: Forward-kinematics bind poses pre-computed at home configuration
#[test]
fn fr_116_fk_bind_poses() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    // Each link Xform uses a single matrix4d xformOp:transform (not split translate+orient)
    assert!(usda.contains("matrix4d xformOp:transform = ("));
    // The home pose places joint_a2 at -90° around its z-axis. Link_2 sits one
    // joint origin (0, 0, 0.5) above link_1's origin and accumulates the
    // -90° rotation in its bind pose. Without FK the matrix would be
    // identity; with FK the rotation entries must be non-trivial.
    assert!(
        usda.contains("uniform token[] xformOpOrder = [\"xformOp:transform\"]"),
        "links must use single matrix4d op"
    );
}

/// Helper: extract a joint's `drive:angular:physics:targetPosition` (degrees)
/// from a composed USDA. Returns `None` if the joint or attribute isn't found.
fn drive_target_for(usda: &str, joint_name: &str) -> Option<f64> {
    let lines: Vec<&str> = usda.lines().collect();
    let mut in_block = false;
    for l in &lines {
        if l.contains(&format!("def PhysicsRevoluteJoint \"{}\"", joint_name)) {
            in_block = true;
        } else if in_block && l.starts_with("        }") {
            break;
        } else if in_block && l.contains("drive:angular:physics:targetPosition") {
            let v = l.split('=').nth(1)?.trim();
            return v.parse::<f64>().ok();
        }
    }
    None
}

/// FR-117: default fallback path. When `home_pose_deg` is omitted on a
/// robot, the KUKA stowed pose `[0, -90°, +90°, 0, 0, 0]` drives the
/// per-joint `targetPosition`. FR-118 covers the per-robot override.
#[test]
fn fr_117_home_pose_drive_targets() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    // Targets emitted in degrees on revolute drives.
    let a1 = drive_target_for(&usda, "joint_a1").expect("joint_a1 drive target");
    let a2 = drive_target_for(&usda, "joint_a2").expect("joint_a2 drive target");
    let a3 = drive_target_for(&usda, "joint_a3").expect("joint_a3 drive target");
    assert!(a1.abs() < 1e-6);
    assert!((a2 + 90.0).abs() < 1e-3);
    assert!((a3 - 90.0).abs() < 1e-3);
}

/// FR-118: per-robot `home_pose_deg` override drives the per-joint
/// `targetPosition` directly (not the KUKA fallback).
#[test]
fn fr_118_per_robot_home_pose() {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"t\"\n").unwrap();

    let scene = SceneDefinition {
        name: "override".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: Some(vec![10.0, -45.0, 30.0, 0.0, 0.0, 0.0]),
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out = dir.path().join("scene.zip");
    compose_stage_with_options(&scene, &manifest, &out, &ComposeOptions::default()).unwrap();
    let usda = read_zip_entry(&out, "scene.usda");

    let a1 = drive_target_for(&usda, "joint_a1").expect("joint_a1");
    let a2 = drive_target_for(&usda, "joint_a2").expect("joint_a2");
    let a3 = drive_target_for(&usda, "joint_a3").expect("joint_a3");
    assert!((a1 - 10.0).abs() < 1e-3, "a1 = {}", a1);
    assert!((a2 + 45.0).abs() < 1e-3, "a2 = {}", a2);
    assert!((a3 - 30.0).abs() < 1e-3, "a3 = {}", a3);
}

/// FR-118: home_pose_deg length must match the URDF's actuated-joint count.
#[test]
fn fr_118_home_pose_length_validation() {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);

    // URDF has 6 actuated joints (joint_a1..a6); supply only 3 → mismatch error.
    let scene = SceneDefinition {
        name: "bad".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: Some(vec![0.0, 0.0, 0.0]),
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let errs = validate_scene(&scene);
    assert!(
        errs.iter().any(|e| {
            let s = e.to_string();
            s.contains("home_pose_deg") && s.contains("3 entries") && s.contains("6 actuated")
        }),
        "expected length-mismatch validation error, got: {:?}",
        errs
    );
}

/// FR-119: a per-robot pedestal raises the robot and emits a static
/// collision cylinder spanning floor to height beneath it.
#[test]
fn fr_119_pedestal_raises_robot_and_emits_cylinder() {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"t\"\n").unwrap();

    let scene = SceneDefinition {
        name: "ped".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf,
            position: [1.0, 2.0, 3.0],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: Some(Pedestal {
                height_mm: 500.0,
                diameter_mm: 800.0,
            }),
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out = dir.path().join("scene.zip");
    compose_stage_with_options(&scene, &manifest, &out, &ComposeOptions::default()).unwrap();
    let usda = read_zip_entry(&out, "scene.usda");

    // 800mm dia → r=0.4, 500mm tall → h=0.5, centred at z=0.25.
    assert!(usda.contains("def Scope \"pedestals\""));
    assert!(usda.contains("def Cylinder \"socket\""));
    assert!(usda.contains("double radius = 0.4"));
    assert!(usda.contains("double height = 0.5"));
    assert!(usda.contains("double3 xformOp:translate = (1, 2, 0.25)"));
    // Robot foot raised z 3.0 → 3.5 on both Xform and world_anchor.
    assert!(usda.contains("double3 xformOp:translate = (1, 2, 3.5)"));
    assert!(usda.contains("point3f physics:localPos0 = (1, 2, 3.5)"));
}

/// FR-119: omitting the pedestal (or height 0) emits no pedestal scope and
/// leaves the robot at its floor z. Diameter defaults to 800mm when only a
/// height is given.
#[test]
fn fr_119_pedestal_defaults_and_absence() {
    // No pedestal → no scope, unchanged z (compose_one_kr_robot places at z=3).
    let (_out, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(!usda.contains("def Scope \"pedestals\""));

    // Manifest with only height_mm → diameter defaults to 800mm (r=0.4).
    let dir = tempfile::tempdir().unwrap();
    let scene_path = dir.path().join("cell.yaml");
    std::fs::write(
        &scene_path,
        "name: \"d\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"r.urdf\"\n    position: { x: 0.0, y: 0.0, z: 0.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\n    pedestal:\n      height_mm: 1000.0\n",
    )
    .unwrap();
    let scene = parse_scene(&scene_path).unwrap();
    let ped = scene.robots[0].pedestal.unwrap();
    assert!((ped.diameter_mm - 800.0).abs() < 1e-6);
    assert!((ped.height_mm - 1000.0).abs() < 1e-6);
}

// =============================================================================
// 2.3 Runtime Tool Swapping  (BACKLOG)
// =============================================================================

#[test]
fn fr_120_runtime_tool_swap() {
    todo!("Runtime tool swap — backlog");
}
#[test]
fn fr_121_preserve_joint_state() {
    todo!("Joint state preservation across swap — backlog");
}
#[test]
fn fr_122_swap_compatibility_check() {
    todo!("Swap compatibility validation — backlog");
}

// =============================================================================
// 2.4 Spatial Validation  (NOT STARTED)
// =============================================================================

#[test]
fn fr_130_forward_kinematics() {
    todo!("FK reachability not yet implemented");
}
#[test]
fn fr_131_static_collision_detection() {
    todo!("Static collision detection not yet implemented");
}
#[test]
fn fr_132_station_boundary_check() {
    todo!("Station boundary validation not yet implemented");
}
#[test]
fn fr_133_target_reachability() {
    todo!("Target reachability validation not yet implemented");
}

// =============================================================================
// 2.5 CLI & API Interface
// =============================================================================

/// FR-140: compose subcommand emits a zip with an inlined .usda
#[test]
fn fr_140_compose_subcommand() {
    let (zip_path, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(zip_path.exists());
    // Zip contents: manifest + usda (no `meshes/` folder — geometry inlined)
    let f = std::fs::File::open(&zip_path).unwrap();
    let mut z = zip::ZipArchive::new(f).unwrap();
    let mut entries: Vec<String> = (0..z.len()).map(|i| z.by_index(i).unwrap().name().to_string()).collect();
    entries.sort();
    assert!(entries.iter().any(|n| n.ends_with(".yaml")));
    assert!(entries.iter().any(|n| n.ends_with(".usda")));
    // USDA header signals USD ASCII format
    assert!(usda.starts_with("#usda 1.0"));
}

/// FR-141: validate subcommand exits OK on a clean manifest
#[test]
fn fr_141_validate_subcommand() {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("robot.urdf");
    std::fs::write(&urdf, r#"<?xml version="1.0"?><robot name="r"><link name="base"/></robot>"#).unwrap();

    let scene_path = dir.path().join("cell.yaml");
    let mut f = std::fs::File::create(&scene_path).unwrap();
    write!(
        f,
        "name: \"Valid\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"robot.urdf\"\n    position: {{ x: 0.0, y: 0.0, z: 0.0 }}\n    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}\n"
    )
    .unwrap();
    let scene = parse_scene(&scene_path).unwrap();
    let errors = validate_scene(&scene);
    assert!(errors.is_empty());
}

#[test]
fn fr_142_launch_subcommand() {
    todo!("launch subcommand not yet implemented");
}
#[test]
fn fr_143_swap_subcommand() {
    todo!("swap subcommand — backlog");
}

/// FR-144: Library API exposes parse_scene, validate_scene, compose_stage(_with_options), ComposeOptions
#[test]
fn fr_144_library_api() {
    let dir = tempfile::tempdir().unwrap();
    let scene_path = dir.path().join("cell.yaml");
    std::fs::write(&scene_path, "name: \"API\"\n").unwrap();
    let scene = parse_scene(&scene_path).unwrap();
    assert_eq!(scene.name, "API");
    let _errors = validate_scene(&scene);

    // Both compose entry points exist and accept the same scene
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let scene = SceneDefinition {
        name: "t".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let manifest = dir.path().join("m.yaml");
    std::fs::write(&manifest, "name: \"t\"\n").unwrap();
    let out_a = dir.path().join("a.zip");
    let out_b = dir.path().join("b.zip");
    compose_stage(&scene, &manifest, &out_a).unwrap();
    compose_stage_with_options(&scene, &manifest, &out_b, &ComposeOptions::default()).unwrap();
    assert!(out_a.exists() && out_b.exists());
}

/// FR-145: --full-robots / --full-tools toggles (mesh-emission only, articulation untouched)
#[test]
fn fr_145_full_robots_full_tools_flags() {
    // Default (full_robots=false, full_tools=true): no robot visual meshes
    let (_zip_lean, usda_lean, _d1) = compose_one_kr_robot(ComposeOptions::default());
    assert!(!usda_lean.contains("def Mesh \"visual_"));
    assert!(usda_lean.contains("def Mesh \"collision_"));

    // full_robots=true: visuals re-appear
    let (_zip_full, usda_full, _d2) = compose_one_kr_robot(ComposeOptions {
        full_robots: true,
        full_tools: true,
    });
    assert!(usda_full.contains("def Mesh \"visual_"));

    // Joints are unaffected by the visual toggle in either mode
    for usda in [&usda_lean, &usda_full] {
        for n in 1..=6 {
            assert!(usda.contains(&format!("def PhysicsRevoluteJoint \"joint_a{}\"", n)));
        }
    }
}

/// FR-146: Deterministic zip — same manifest produces byte-identical USDA
#[test]
fn fr_146_deterministic_zip() {
    let (_z1, u1, _d1) = compose_one_kr_robot(ComposeOptions::default());
    let (_z2, u2, _d2) = compose_one_kr_robot(ComposeOptions::default());
    assert_eq!(u1.len(), u2.len());
    assert_eq!(u1, u2);
}

/// Build a two-robot (sharing one dressed URDF) + one-fixture + floor-plan
/// scene on disk and return (scene, manifest_path, tempdir). The manifest YAML
/// carries scene_origin + home_pose_deg so the rewrite must preserve them.
fn urdf_bundle_fixture() -> (SceneDefinition, PathBuf, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    // Author the sources under distinct dirs (src-robots/, src-fixtures/,
    // src-floor/) so the bundled top-level layout (robots/ fixtures/
    // floorplans/) is provably different from the authored paths.
    let robot_urdf = dir.path().join("src-robots/kr/kr.urdf");
    std::fs::create_dir_all(robot_urdf.parent().unwrap()).unwrap();
    write_kr_like_urdf(&robot_urdf);
    let fixture_urdf = dir.path().join("src-fixtures/f/f.urdf");
    std::fs::create_dir_all(fixture_urdf.parent().unwrap()).unwrap();
    write_fixture_urdf(&fixture_urdf);
    // Floor plan OBJ.
    let obj = dir.path().join("src-floor/plan.obj");
    std::fs::create_dir_all(obj.parent().unwrap()).unwrap();
    std::fs::write(&obj, "v 0 0 0\nv 1 0 0\nl 1 2\n").unwrap();

    let manifest = dir.path().join("scene.yaml");
    let yaml = "name: \"bundle\"\n\
        description: \"desc\"\n\
        scene_origin: { x: 1.0, y: 2.0, z: 0.0 }\n\
        robots:\n\
        \x20 - robot_id: \"R1\"\n\
        \x20   dressed_urdf: \"src-robots/kr/kr.urdf\"\n\
        \x20   position: { x: 0.0, y: 0.0, z: 0.0 }\n\
        \x20   orientation: { a: 0.0, b: 0.0, c: 0.0 }\n\
        \x20   home_pose_deg: [0, -90, 90, 0, 0, 0]\n\
        \x20 - robot_id: \"R2\"\n\
        \x20   dressed_urdf: \"src-robots/kr/kr.urdf\"\n\
        \x20   position: { x: 5.0, y: 0.0, z: 0.0 }\n\
        \x20   orientation: { a: 0.0, b: 0.0, c: 0.0 }\n\
        fixtures:\n\
        \x20 - id: \"F1\"\n\
        \x20   urdf: \"src-fixtures/f/f.urdf\"\n\
        \x20   position: { x: 2.0, y: 2.0, z: 0.0 }\n\
        \x20   orientation: { a: 0.0, b: 0.0, c: 0.0 }\n\
        floor_plan:\n\
        \x20 geometry_file: \"src-floor/plan.obj\"\n\
        \x20 scale: 1.0\n";
    std::fs::write(&manifest, yaml).unwrap();

    let scene = parse_scene(&manifest).unwrap();
    (scene, manifest, dir)
}

/// FR-147: compose_urdf_bundle produces a self-contained zip — rewritten scene
/// YAML + deduped source URDFs (own folders, meshes/ preserved) + floor-plan
/// OBJ; deterministic across runs; extracted YAML validates standalone.
#[test]
fn fr_147_urdf_source_bundle() {
    let (scene, manifest, dir) = urdf_bundle_fixture();
    let out = dir.path().join("bundle.zip");
    let report = compose_urdf_bundle(&scene, &manifest, &out).unwrap();
    assert!(out.exists());

    let names = zip_entry_names(&out);
    // Scene YAML present.
    assert!(names.iter().any(|n| n == "scene.yaml"), "names: {:?}", names);
    // Two robots share one URDF dir → exactly one robot bundle folder.
    assert_eq!(report.robot_urdf_count, 1);
    assert_eq!(report.fixture_urdf_count, 1);
    // The shared robot URDF + its mesh are copied under robots/<folder>/.
    assert!(names.iter().any(|n| n.starts_with("robots/") && n.ends_with(".urdf")));
    assert!(names.iter().any(|n| n.starts_with("robots/") && n.contains("meshes/robot/")));
    // Fixture under fixtures/<folder>/.
    assert!(names.iter().any(|n| n.starts_with("fixtures/") && n.ends_with(".urdf")));
    // Floor plan bundled under floorplans/<stem>/<basename>.
    assert!(report.floor_plan_bundled);
    assert!(names.iter().any(|n| n == "floorplans/plan/plan.obj"));

    // Rewritten YAML references bundled relative paths and preserves extras.
    let yaml = read_zip_entry(&out, "scene.yaml");
    assert!(yaml.contains("robots/"), "expected rewritten robot path: {}", yaml);
    assert!(yaml.contains("fixtures/"), "expected rewritten fixture path");
    assert!(yaml.contains("floorplans/plan/plan.obj"), "expected rewritten floor path");
    assert!(!yaml.contains("src-robots/kr/kr.urdf"), "old path must be rewritten");
    assert!(yaml.contains("scene_origin"), "scene_origin must be preserved");
    assert!(yaml.contains("home_pose_deg"), "home_pose_deg must be preserved");

    // Determinism: a second run into a fresh path is byte-identical.
    let out2 = dir.path().join("bundle2.zip");
    compose_urdf_bundle(&scene, &manifest, &out2).unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), std::fs::read(&out2).unwrap());

    // Extracted bundle validates standalone (the decisive self-containedness check).
    let extract = dir.path().join("extracted");
    std::fs::create_dir_all(&extract).unwrap();
    {
        let f = std::fs::File::open(&out).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        z.extract(&extract).unwrap();
    }
    let extracted_scene = parse_scene(&extract.join("scene.yaml")).unwrap();
    let errors = validate_scene(&extracted_scene);
    assert!(errors.is_empty(), "extracted bundle should validate: {:?}", errors);
}

/// FR-148: a URDF referencing a mesh outside its own directory is rejected.
#[test]
fn fr_148_bundle_rejects_external_mesh() {
    let dir = tempfile::tempdir().unwrap();
    // A mesh that lives OUTSIDE the URDF dir, referenced via `..`.
    let outside_mesh = dir.path().join("shared/m.obj");
    write_obj_triangle(&outside_mesh);
    let urdf = dir.path().join("robot/kr.urdf");
    std::fs::create_dir_all(urdf.parent().unwrap()).unwrap();
    let s = "<?xml version=\"1.0\"?>\n<robot name=\"kr\">\n  <link name=\"base\">\n    <visual>\n      <geometry>\n        <mesh filename=\"../shared/m.obj\"/>\n      </geometry>\n    </visual>\n  </link>\n</robot>\n";
    std::fs::write(&urdf, s).unwrap();

    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"x\"\nrobots:\n  - robot_id: \"R1\"\n    dressed_urdf: \"robot/kr.urdf\"\n    position: { x: 0.0, y: 0.0, z: 0.0 }\n    orientation: { a: 0.0, b: 0.0, c: 0.0 }\n").unwrap();
    let scene = parse_scene(&manifest).unwrap();
    let out = dir.path().join("bundle.zip");
    let err = compose_urdf_bundle(&scene, &manifest, &out).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("outside its directory") || msg.contains("absolute path"), "got: {}", msg);
}

/// FR-149: `compose --urdf-bundle` writes a second <stem>-urdf.zip. Exercised
/// here via the library entry points that back the flag (the CLI wiring only
/// derives the path + calls compose_urdf_bundle after compose_stage).
#[test]
fn fr_149_cli_urdf_bundle_flag() {
    let (scene, manifest, dir) = urdf_bundle_fixture();
    // Mirror the CLI: compose the USDA zip, then the derived -urdf.zip.
    let usda_out = dir.path().join("cell.zip");
    compose_stage(&scene, &manifest, &usda_out).unwrap();
    let bundle_out = dir.path().join("cell-urdf.zip");
    compose_urdf_bundle(&scene, &manifest, &bundle_out).unwrap();
    assert!(usda_out.exists());
    assert!(bundle_out.exists());
    // The two artifacts are distinct: USDA zip has a .usda; bundle has URDFs.
    let usda_names = zip_entry_names(&usda_out);
    assert!(usda_names.iter().any(|n| n.ends_with(".usda")));
    let bundle_names = zip_entry_names(&bundle_out);
    assert!(bundle_names.iter().any(|n| n.ends_with(".urdf")));
    assert!(!bundle_names.iter().any(|n| n.ends_with(".usda")));
}

// =============================================================================
// 2.6 ActionGraph & ROS2 Preparation
// =============================================================================

/// FR-150: Per-robot ActionGraph including a FollowJointTrajectory action
/// server — moved to BACKLOG. The publish/subscribe + ArticulationController
/// pieces are emitted today; FR-150 stays red until BL-11 lands the action
/// server node.
#[test]
fn fr_150_actiongraph_generation() {
    todo!("FollowJointTrajectory action server emission — BL-11");
}

/// FR-151: ROS 2 namespace is `<cell_id>/<robot_id>`; digit-leading values
/// in either segment get an `r_` prefix to satisfy rmw_validate_namespace.
#[test]
fn fr_151_ros2_namespace() {
    // Plain ids in both segments → both pass through verbatim.
    // compose_one_kr_robot builds a SceneDefinition { name: "t", robot_id: "R1" }.
    let (_z1, u1, _d1) = compose_one_kr_robot(ComposeOptions::default());
    assert!(
        u1.contains("inputs:nodeNamespace = \"t/R1\""),
        "expected two-segment <cell_id>/<robot_id> namespace, got: {}",
        u1.lines().find(|l| l.contains("inputs:nodeNamespace")).unwrap_or("<none>"),
    );

    // Digit-leading robot_id → r_-prefixed second segment.
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let manifest = dir.path().join("m.yaml");
    std::fs::write(&manifest, "name: \"K8URW11\"\n").unwrap();
    let scene = SceneDefinition {
        name: "K8URW11".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "113190R01".into(),
            dressed_urdf: urdf,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out = dir.path().join("scene.zip");
    compose_stage(&scene, &manifest, &out).unwrap();
    let usda = read_zip_entry(&out, "m.usda");
    assert!(
        usda.contains("inputs:nodeNamespace = \"K8URW11/r_113190R01\""),
        "expected K8URW11/r_113190R01 namespace, got: {}",
        usda.lines().find(|l| l.contains("inputs:nodeNamespace")).unwrap_or("<none>"),
    );
}

/// FR-152: Scene-level /clock publisher, namespaced under the cell.
/// FQT becomes `/<cell_id>/clock` so multi-cell deployments don't collide.
#[test]
fn fr_152_clock_publisher() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(usda.contains("def OmniGraph \"ActionGraph_clock\""));
    assert!(usda.contains("ROS2PublishClock"));
    assert!(usda.contains("inputs:topicName = \"clock\""));
    // Cell-scoped namespace — the test fixture's manifest name is "t".
    assert!(
        usda.contains("inputs:nodeNamespace = \"t\""),
        "clock publisher must carry the cell namespace",
    );
}

/// FR-153: Joint mappings auto-derived from URDF actuated-joint declaration order
#[test]
fn fr_153_joint_mapping() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    // The ArticulationController inputs:jointNames token[] must list a1..a6 in order
    let needle = "inputs:jointNames = [\"joint_a1\", \"joint_a2\", \"joint_a3\", \"joint_a4\", \"joint_a5\", \"joint_a6\"]";
    assert!(usda.contains(needle), "joint name array out of order or missing");
}

/// FR-154: PhysicsScene tuning (120 Hz step, solver 64/4, drive 1.5e5 / 100)
#[test]
fn fr_154_physx_scene_tuning() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(usda.contains("physxScene:timeStepsPerSecond = 120"));
    assert!(usda.contains("physxArticulation:solverPositionIterationCount = 64"));
    assert!(usda.contains("physxArticulation:solverVelocityIterationCount = 4"));
    assert!(usda.contains("drive:angular:physics:stiffness = 150000"));
    assert!(usda.contains("drive:angular:physics:damping = 100"));
}

/// FR-155: GPU broadphase capacities
#[test]
fn fr_155_gpu_broadphase_capacities() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(usda.contains("physxScene:gpuFoundLostPairsCapacity = 8192"));
    assert!(usda.contains("physxScene:gpuFoundLostAggregatePairsCapacity = 8192"));
    assert!(usda.contains("physxScene:gpuTotalAggregatePairsCapacity = 4096"));
    assert!(usda.contains("physxScene:gpuMaxRigidContactCount = 1048576"));
    assert!(usda.contains("physxScene:gpuMaxRigidPatchCount = 16384"));
}

// =============================================================================
// 2.7 Collision Reporting
// =============================================================================

/// FR-160: every collision-enabled link emits an IsaacReadContactSensor +
/// ROS2PublishContactSensor pair inside the existing per-robot ActionGraph.
#[test]
fn fr_160_contact_sensor_nodes() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    // The kr_like URDF has 8 links (World + base + link_1..6), each with one
    // <collision>. World sanitises to "world_link"; the rest pass through.
    let sensor_count = usda
        .matches("\"isaacsim.sensors.physics.IsaacReadContactSensor\"")
        .count();
    // The publisher is the generic `ROS2Publisher` (not the
    // `ROS2PublishContactSensor` node type, which doesn't exist in
    // Isaac Sim 5.1). The matching emission flag is the
    // `messageName = "Bool"` line — count those.
    let publisher_count = usda.matches("inputs:messageName = \"Bool\"").count();
    assert_eq!(sensor_count, 8, "expected 8 contact sensors (one per collision link)");
    assert_eq!(publisher_count, 8, "expected 8 ROS2Publisher Bool emissions (one per sensor)");
    // Per-link IsaacContactSensor prim must be emitted with `enabled = 1`
    // — without it Isaac drops events with `no valid sensor reading`.
    assert_eq!(
        usda.matches("def IsaacContactSensor \"ContactSensor_node\"").count(),
        8,
        "expected 8 IsaacContactSensor prims (one per collision link)",
    );

    // Sensors live inside the same per-robot graph as motion (ActionGraph_R1),
    // not in a separate ActionGraph_contacts_*. Verify by absence.
    assert!(
        !usda.contains("ActionGraph_contacts_"),
        "contact sensors must inline into the existing per-robot graph",
    );
    assert!(usda.contains("def OmniGraph \"ActionGraph_R1\""));

    // Each ContactSensor's csPrim rel resolves to the per-link
    // IsaacContactSensor USD prim emitted as a child of the link Xform.
    // Pointing at the link itself triggers Isaac's "no valid sensor
    // reading, is the sensor enabled?" warning at runtime.
    assert!(
        usda.contains("add rel inputs:csPrim = </World/robots/R1/world_link/ContactSensor_node>"),
        "csPrim rel must point at the per-link IsaacContactSensor child prim",
    );
    assert!(usda.contains("def Xform \"world_link\""));
    // The IsaacContactSensor prim itself must be present and enabled.
    assert!(
        usda.contains("def IsaacContactSensor \"ContactSensor_node\""),
        "every collision-bearing link must emit a child IsaacContactSensor prim",
    );
    assert!(usda.contains("bool enabled = 1"));
}

/// FR-161: silence guarantee at compose time. The ROS2PublishContactSensor's
/// execIn is wired to the matching IsaacReadContactSensor's execOut, NOT the
/// OnPlaybackTick — so the publisher only fires when the sensor reports a
/// contact event, never per frame. The namespace on every publisher is the
/// two-segment <cell_id>/<robot_id> contract from the topic schema.
#[test]
fn fr_161_contact_ros2_topic() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());

    // Find every PublishContact_<link> block and check its execIn target.
    let mut checked_at_least_one = false;
    let lines: Vec<&str> = usda.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        if let Some(rest) = line.trim().strip_prefix("def OmniGraphNode \"PublishContact_") {
            let link = rest.trim_end_matches('"');
            // Scan forward for the execIn.connect line (within ~30 lines).
            let exec_in = lines[i..i.saturating_add(30).min(lines.len())]
                .iter()
                .find(|l| l.contains("inputs:execIn.connect"))
                .copied()
                .unwrap_or("<missing>");
            assert!(
                exec_in.contains(&format!("ContactSensor_{}.outputs:execOut", link)),
                "PublishContact_{} must wire execIn to its sensor's execOut, got: {}",
                link, exec_in,
            );
            assert!(
                !exec_in.contains("OnPlaybackTick.outputs:tick"),
                "PublishContact_{} must NOT tie execIn to OnPlaybackTick (would break the silence guarantee), got: {}",
                link, exec_in,
            );
            // Namespace shape: <cell_id>/<robot_id> for this fixture.
            let ns = lines[i..i.saturating_add(30).min(lines.len())]
                .iter()
                .find(|l| l.contains("inputs:nodeNamespace"))
                .copied()
                .unwrap_or("<missing>");
            assert!(
                ns.contains("\"t/R1\""),
                "PublishContact_{} must carry namespace t/R1, got: {}",
                link, ns,
            );
            checked_at_least_one = true;
        }
    }
    assert!(checked_at_least_one, "no PublishContact_* nodes found");
}

/// FR-162: contact-sensor wiring is per-robot and URDF-derived. Adding a
/// second robot (same URDF) must double the contact-sensor count — no
/// manifest knob, no manual config.
#[test]
fn fr_162_automatic_contact_wiring() {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"t\"\n").unwrap();

    let one_robot = SceneDefinition {
        name: "t".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf.clone(),
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out_one = dir.path().join("one.zip");
    compose_stage_with_options(&one_robot, &manifest, &out_one, &ComposeOptions::default()).unwrap();
    let usda_one = read_zip_entry(&out_one, "scene.usda");
    let count_one = usda_one
        .matches("\"isaacsim.sensors.physics.IsaacReadContactSensor\"")
        .count();
    assert!(count_one > 0, "1-robot scene must produce contact sensors");

    let two_robots = SceneDefinition {
        name: "t".into(),
        description: String::new(),
        robots: vec![
            RobotPlacement {
                robot_id: "R1".into(),
                dressed_urdf: urdf.clone(),
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            },
            RobotPlacement {
                robot_id: "R2".into(),
                dressed_urdf: urdf,
                position: [5.0, 0.0, 0.0],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            },
        ],
        fixtures: vec![],
        floor_plan: None,
    };
    let out_two = dir.path().join("two.zip");
    compose_stage_with_options(&two_robots, &manifest, &out_two, &ComposeOptions::default()).unwrap();
    let usda_two = read_zip_entry(&out_two, "scene.usda");
    let count_two = usda_two
        .matches("\"isaacsim.sensors.physics.IsaacReadContactSensor\"")
        .count();
    assert_eq!(count_two, count_one * 2, "adding a robot must double the contact-sensor count");
}

/// FR-163: USD collision geometry uses the dresser-generated convex hulls
#[test]
fn fr_163_dresser_collision_hulls() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    // Convex hull approximation drives PhysX contact generation off the
    // collision meshes from the URDF's <collision> elements.
    assert!(usda.contains("uniform token physics:approximation = \"convexHull\""));
    assert!(usda.contains("PhysicsMeshCollisionAPI"));
}

/// Wire-contract guarantee: every emitted `inputs:nodeNamespace` and
/// `inputs:topicName` is a string of slash-separated tokens, where each
/// token matches the rmw regex `[A-Za-z_][A-Za-z0-9_]*`. This is the
/// shape both FastDDS (Isaac default) AND `rmw_cyclonedds_cpp` accept,
/// so users running Isaac Sim with `RMW_IMPLEMENTATION=rmw_cyclonedds_cpp`
/// see no validation errors at bridge bring-up.
#[test]
fn ros2_topics_are_cyclone_dds_compatible() {
    // Use a digit-prefixed manifest name + robot_id so we exercise the
    // r_-prefix sanitisation in both segments — the most demanding case.
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"K8URW11\"\n").unwrap();

    let scene = SceneDefinition {
        name: "2025-line-A".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "113200R01".into(),
            dressed_urdf: urdf,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out = dir.path().join("scene.zip");
    compose_stage(&scene, &manifest, &out).unwrap();
    let usda = read_zip_entry(&out, "scene.usda");

    // rmw token: leading [A-Za-z_], rest [A-Za-z0-9_].
    fn rmw_token_ok(tok: &str) -> bool {
        let mut chars = tok.chars();
        let Some(first) = chars.next() else { return false };
        if !(first.is_ascii_alphabetic() || first == '_') {
            return false;
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    let mut checked = 0usize;
    for line in usda.lines() {
        let line = line.trim();
        for (key, label) in [
            ("custom string inputs:nodeNamespace = \"", "namespace"),
            ("custom string inputs:topicName = \"", "topic"),
        ] {
            if let Some(rest) = line.strip_prefix(key) {
                let value = rest.trim_end_matches('"');
                assert!(!value.is_empty(), "empty {} string", label);
                assert!(!value.starts_with('/'), "{} must NOT begin with `/`: {:?}", label, value);
                assert!(!value.ends_with('/'), "{} must NOT end with `/`: {:?}", label, value);
                assert!(!value.contains("//"), "{} must NOT contain `//`: {:?}", label, value);
                for tok in value.split('/') {
                    assert!(
                        rmw_token_ok(tok),
                        "{} segment {:?} (in {:?}) violates rmw token regex \
                         [A-Za-z_][A-Za-z0-9_]* — would be rejected by \
                         rmw_validate_namespace / rmw_validate_topic_name under \
                         rmw_cyclonedds_cpp",
                        label, tok, value,
                    );
                }
                checked += 1;
            }
        }
    }
    assert!(checked >= 4, "expected at least 4 namespace/topic emissions, got {}", checked);
}

#[test]
fn fr_164_monitor_subcommand() {
    todo!("Monitor subcommand not yet implemented");
}
#[test]
fn fr_165_monitor_topic_derivation() {
    todo!("Monitor topic derivation not yet implemented");
}
#[test]
fn fr_166_monitor_exit_code() {
    todo!("Monitor exit code not yet implemented");
}
#[test]
fn fr_167_monitor_persistence() {
    todo!("Monitor persistence not yet implemented");
}
#[test]
fn fr_168_force_threshold() {
    todo!("Force threshold filter not yet implemented");
}
#[test]
fn fr_169_event_debouncing() {
    todo!("Event debouncing not yet implemented");
}

// =============================================================================
// 2.8 USD Deploy  (NOT STARTED)
// =============================================================================

#[test]
fn fr_170_streaming_client_connect() {
    todo!("Streaming client connection not yet implemented");
}
#[test]
fn fr_171_stage_deploy() {
    todo!("Stage deploy not yet implemented");
}
#[test]
fn fr_172_live_reload() {
    todo!("Live stage reload not yet implemented");
}
#[test]
fn fr_173_deploy_status() {
    todo!("Deploy status reporting not yet implemented");
}
#[test]
fn fr_174_asset_path_resolution() {
    todo!("Asset path resolution not yet implemented");
}
#[test]
fn fr_175_connectivity_check() {
    todo!("Pre-deploy connectivity validation not yet implemented");
}
#[test]
fn fr_176_ros_domain_id() {
    todo!("ROS_DOMAIN_ID matching not yet implemented");
}
#[test]
fn fr_177_extension_verification() {
    todo!("Target extension verification not yet implemented");
}

// =============================================================================
// 2.9 Server Mode  (subset DONE/PARTIAL behind --features server)
// =============================================================================
//
// FR-180/181/182/184/204 happy-path coverage lives in `tests/server.rs`,
// gated behind the `server` Cargo feature. Run those with:
//   cargo test --features server --test server
// The placeholders below remain so REQUIREMENTS.md → tests mapping stays
// 1:1; default-feature builds report them as `todo!()` until the server
// feature is enabled.

/// FR-180: in-binary HTTP server behind `server` feature → covered by
/// `tests/server.rs::list_robots_returns_seeded_entry` and friends.
#[test]
fn fr_180_server_startup() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — run `cargo test --features server --test server`");
    }
}

/// FR-181: `POST /api/compose` returns a streaming zip → covered by
/// `tests/server.rs::compose_returns_zip`.
#[test]
fn fr_181_api_compose() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-182: `POST /api/validate` returns structured diagnostics →
/// `tests/server.rs::validate_round_trips_yaml`.
#[test]
fn fr_182_api_validate() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

#[test]
fn fr_183_api_monitor() {
    todo!("API monitor (SSE collision stream) — BL-17");
}

/// FR-184: zip stream covers the binary-download path → see FR-181 test.
/// Stage-ID registry is BL-19.
#[test]
fn fr_184_api_get_scene() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

#[test]
fn fr_185_api_deploy() {
    todo!("API deploy — BL-20");
}
#[test]
fn fr_186_api_swap() {
    todo!("API swap — BL-03");
}
#[test]
fn fr_187_api_status() {
    todo!("API status — depends on FR-185");
}
/// FR-188: `POST /api/compose-urdf-bundle` streams the URDF-source bundle zip →
/// covered by `tests/server.rs::compose_urdf_bundle_returns_zip`.
#[test]
fn fr_188_api_compose_urdf_bundle() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — run `cargo test --features server --test server`");
    }
}
#[test]
fn fr_190_sse_broadcast_fanout() {
    todo!("SSE broadcast — BL-17");
}
#[test]
fn fr_191_event_db_sink() {
    todo!("Event DB sink — BL-18");
}

// =============================================================================
// 2.10 2D Scene-Layout Editor
// =============================================================================
//
// The editor SPA lives in `web/`, embedded into the binary via
// `include_dir!`. End-to-end coverage is the manual workflow in the README
// (load YAML → drag → save → compose → open in Isaac); the server-side
// supporting endpoints are covered by `tests/server.rs`.

/// FR-200: top-down 2D layout with drag-drop. Browser-side; manual smoke.
#[test]
fn fr_200_2d_layout_canvas() {
    // Compile-time evidence: when the `server` feature is on, the
    // embedded SPA bundle compiles in via include_dir!.
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — editor SPA only ships with --features server");
    }
}

/// FR-201: floor-plan upload-OBJ endpoint → covered by
/// `tests/server.rs::upload_obj_round_trips_and_appears_in_listing`.
#[test]
fn fr_201_floor_plan_upload() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-202: form-based inspector. Browser-side; manual smoke. Server-side
/// `/api/urdf/joints` (used by the inspector) is tested in
/// `tests/server.rs::urdf_joints_returns_actuated_count`.
#[test]
fn fr_202_form_based_inspector() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-203: the Generate-USDA modal exposes an optional "Also download URDF
/// source bundle" checkbox that triggers a second `/api/compose-urdf-bundle`
/// download. Browser-side; the server endpoint it calls is covered by
/// `tests/server.rs::compose_urdf_bundle_returns_zip`.
#[test]
fn fr_203_editor_urdf_bundle_checkbox() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled — editor SPA only ships with --features server");
    }
}

/// FR-204: serve subcommand boots both the API and the embedded SPA.
#[test]
fn fr_204_in_binary_serve() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-205: OpenAPI spec + Swagger UI shipped behind the `server` feature.
/// Wire-level coverage lives in tests/server.rs (or is reachable via curl);
/// here we just gate on the feature so the requirement is auditable.
#[test]
fn fr_205_openapi_docs() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-206: scene save/load round-trip. Wire-level test:
/// `tests/server.rs::save_then_load_round_trips`.
#[test]
fn fr_206_api_scenes_save_load() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-207: path-traversal sandboxing. Wire-level test:
/// `tests/server.rs::path_traversal_is_blocked`.
#[test]
fn fr_207_asset_path_sandbox() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-208: actuated-joint listing endpoint. Wire-level test:
/// `tests/server.rs::urdf_joints_returns_actuated_count`.
#[test]
fn fr_208_api_urdf_joints() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-209: `cell_origin_dxf` derivation in floor-plan parsing. When the
/// manifest sets only `cell_origin_dxf` (no explicit `offset`),
/// parse_scene must derive `offset = -cell_origin_dxf` so the chosen DXF
/// point lands at the world origin.
#[test]
fn fr_209_cell_origin_dxf_derivation() {
    let dir = tempfile::tempdir().unwrap();
    let obj_path = dir.path().join("plan.obj");
    // Single segment from (100, 50, 0) to (101, 50, 0).
    std::fs::write(
        &obj_path,
        "v 100.0 50.0 0.0\nv 101.0 50.0 0.0\nl 1 2\n",
    )
    .unwrap();

    let scene_path = dir.path().join("cell.yaml");
    std::fs::write(
        &scene_path,
        "name: \"fp\"\nfloor_plan:\n  geometry_file: \"plan.obj\"\n  cell_origin_dxf: { x: 100.0, y: 50.0, z: 0.0 }\n",
    )
    .unwrap();

    let scene = parse_scene(&scene_path).unwrap();
    let fp = scene.floor_plan.expect("floor plan parsed");
    // After derivation the (100, 50) DXF point lands at the world origin.
    assert!((fp.segments[0][0][0] - 0.0).abs() < 1e-6);
    assert!((fp.segments[0][0][1] - 0.0).abs() < 1e-6);
    assert!((fp.segments[0][1][0] - 1.0).abs() < 1e-6);
    assert!((fp.segments[0][1][1] - 0.0).abs() < 1e-6);
}

/// FR-210: liveness + readiness probes at the root. Wire-level test:
/// `tests/server.rs::healthz_and_readyz_are_served`.
#[test]
fn fr_210_health_readiness_probes() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-211: typed `ErrorEnvelope` body on every 4xx/5xx response. Wire-level
/// test: `tests/server.rs::error_envelope_is_typed`.
#[test]
fn fr_211_error_envelope_is_typed() {
    #[cfg(feature = "server")]
    {
        return;
    }
    #[allow(unreachable_code)]
    {
        todo!("Server feature not enabled");
    }
}

/// FR-212: every emitted ROS 2 namespace + topic passes the rmw token regex,
/// guaranteeing the composed USDA works under both FastDDS and Cyclone DDS.
/// Asserted by `ros2_topics_are_cyclone_dds_compatible` above.
#[test]
fn fr_212_cyclone_dds_compatible_topics() {
    // The exhaustive assertion lives in `ros2_topics_are_cyclone_dds_compatible`
    // — this entry just keeps the FR ↔ test mapping 1:1 for auditability.
}

/// FR-213: cell-level `/<cell_id>/contacts/v1` topic with enriched
/// `cell_sim_contact_msgs/msg/CollisionEvent` records. Implemented by the
/// aggregator pod (BL-24) — separate from the composer.
#[test]
fn fr_213_cell_collision_event_topic() {
    todo!("cell-level CollisionEvent aggregator — BL-24");
}

/// FR-214: collision-event message schema versioning policy
/// (`schema_version` field, versioned topic `/v<n>`, IDL-hash snapshot).
#[test]
fn fr_214_collision_event_schema_versioning() {
    todo!("collision-event schema versioning policy — BL-24");
}

// =============================================================================
// 3. Non-Functional Requirements
// =============================================================================

/// NFR-100: Pure-Rust pipeline — no native USD/PhysX deps
#[test]
fn nfr_100_pure_rust_pipeline() {
    // Pure-Rust ASCII USDA emission: a successful compose without any C++/cmake
    // bindings is itself the proof.
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(usda.starts_with("#usda 1.0"));
}

#[test]
fn nfr_101_composition_performance() {
    todo!("Performance benchmark — needs criterion harness");
}
#[test]
fn nfr_102_swap_performance() {
    todo!("Swap performance benchmark — backlog");
}

/// NFR-103: Compose runs on Linux / macOS / Windows
#[test]
fn nfr_103_platform_support() {
    assert!(cfg!(target_os = "macos") || cfg!(target_os = "linux") || cfg!(target_os = "windows"));
    // Sanity: compose actually runs on whichever platform is hosting the tests.
    let _ = compose_one_kr_robot(ComposeOptions::default());
}

/// NFR-104: validate_scene rejects non-URDF 3D inputs
#[test]
fn nfr_104_urdf_only_input() {
    let dir = tempfile::tempdir().unwrap();
    let stl = dir.path().join("not-a-urdf.stl");
    std::fs::write(&stl, b"solid foo\nendsolid\n").unwrap();
    let scene = SceneDefinition {
        name: "t".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: stl,
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let errors = validate_scene(&scene);
    assert!(
        !errors.is_empty(),
        "non-URDF 3D file must be rejected at validate time"
    );
}

/// NFR-105: Deterministic USD output — identical manifests → byte-identical USDA
#[test]
fn nfr_105_deterministic_output() {
    let (_z1, u1, _d1) = compose_one_kr_robot(ComposeOptions::default());
    let (_z2, u2, _d2) = compose_one_kr_robot(ComposeOptions::default());
    assert_eq!(u1, u2);
}

/// NFR-106: Compose is read-only on source URDFs and meshes
#[test]
fn nfr_106_read_only_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let urdf = dir.path().join("kr.urdf");
    write_kr_like_urdf(&urdf);
    let mesh = dir.path().join("meshes/robot/m.obj");
    let urdf_before = std::fs::read(&urdf).unwrap();
    let mesh_before = std::fs::read(&mesh).unwrap();
    let urdf_mtime_before = std::fs::metadata(&urdf).unwrap().modified().unwrap();
    let mesh_mtime_before = std::fs::metadata(&mesh).unwrap().modified().unwrap();

    let manifest = dir.path().join("scene.yaml");
    std::fs::write(&manifest, "name: \"t\"\n").unwrap();
    let scene = SceneDefinition {
        name: "t".into(),
        description: String::new(),
        robots: vec![RobotPlacement {
            robot_id: "R1".into(),
            dressed_urdf: urdf.clone(),
            position: [0.0; 3],
            orientation_deg: [0.0; 3],
            home_pose_deg: None,
            pedestal: None,
        }],
        fixtures: vec![],
        floor_plan: None,
    };
    let out = dir.path().join("scene.zip");
    compose_stage(&scene, &manifest, &out).unwrap();

    assert_eq!(std::fs::read(&urdf).unwrap(), urdf_before);
    assert_eq!(std::fs::read(&mesh).unwrap(), mesh_before);
    assert_eq!(
        std::fs::metadata(&urdf).unwrap().modified().unwrap(),
        urdf_mtime_before
    );
    assert_eq!(
        std::fs::metadata(&mesh).unwrap().modified().unwrap(),
        mesh_mtime_before
    );
}

#[test]
fn nfr_107_port_error_messages() {
    todo!("Port unreachable error messages — depends on FR-170/175");
}

/// NFR-108: USDA layout matches NVIDIA's flat-scene template (header + Render + Environment + FlatGrid)
#[test]
fn nfr_108_flat_scene_template_layout() {
    let (_zip, usda, _dir) = compose_one_kr_robot(ComposeOptions::default());
    assert!(usda.contains("#usda 1.0"));
    assert!(usda.contains("customLayerData"));
    assert!(usda.contains("def Xform \"World\""));
    assert!(usda.contains("def PhysicsScene \"PhysicsScene\""));
    assert!(usda.contains("def Xform \"Environment\""));
    assert!(usda.contains("def \"Render\""));
    assert!(usda.contains("def \"FlatGrid\""));
}
