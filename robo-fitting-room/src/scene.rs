use std::path::{Path, PathBuf};

use serde::Deserialize;
use urdf_robo_dresser::{parse_urdf, JointType};

use crate::error::FittingRoomError;
use crate::floor_plan::{self, FloorPlan};

#[derive(Debug, Clone)]
pub struct SceneDefinition {
    pub name: String,
    pub description: String,
    pub robots: Vec<RobotPlacement>,
    pub fixtures: Vec<FixturePlacement>,
    pub floor_plan: Option<FloorPlan>,
}

#[derive(Debug, Clone)]
pub struct RobotPlacement {
    pub robot_id: String,
    pub dressed_urdf: PathBuf,
    pub position: [f64; 3],
    pub orientation_deg: [f64; 3],
    /// FR-118: optional per-robot home pose, one entry per actuated joint
    /// (in URDF declaration order, fixed joints excluded), in degrees.
    /// `None` means "fall back to the KUKA stowed default in compose".
    pub home_pose_deg: Option<Vec<f64>>,
    /// FR-119: optional elevated mounting pedestal. `None` = flush floor mount.
    pub pedestal: Option<Pedestal>,
}

/// FR-119: elevated mounting fundament beneath a robot. Height raises the robot
/// off the floor; a static collision cylinder of the given diameter fills the gap.
#[derive(Debug, Clone, Copy)]
pub struct Pedestal {
    pub height_mm: f64,
    pub diameter_mm: f64,
}

#[derive(Debug, Clone)]
pub struct FixturePlacement {
    pub id: String,
    pub urdf: PathBuf,
    pub position: [f64; 3],
    pub orientation_deg: [f64; 3],
}

// --- serde intermediate types (match reference repo YAML convention) ---

#[derive(Debug, Deserialize)]
struct SceneYaml {
    name: String,
    #[serde(default)]
    description: Option<String>,
    /// World-space origin of the cell. All robot/fixture/floor-plan positions
    /// have this subtracted at parse time so the composed scene is centred
    /// around (0, 0, 0). Defaults to (0, 0, 0) — no shift.
    #[serde(default)]
    scene_origin: Option<PositionYaml>,
    #[serde(default)]
    robots: Vec<RobotYaml>,
    #[serde(default)]
    fixtures: Vec<FixtureYaml>,
    #[serde(default)]
    floor_plan: Option<FloorPlanYaml>,
}

#[derive(Debug, Deserialize)]
struct RobotYaml {
    robot_id: String,
    dressed_urdf: String,
    position: PositionYaml,
    orientation: OrientationYaml,
    /// FR-118 per-robot home pose. Optional list of N angles in degrees,
    /// one per actuated joint of the dressed URDF. Length is validated
    /// against the URDF at validate-time. Omitting the field falls back
    /// to the KUKA stowed default in compose.
    #[serde(default)]
    home_pose_deg: Option<Vec<f64>>,
    /// FR-119 optional mounting pedestal. Omit for a flush floor mount.
    #[serde(default)]
    pedestal: Option<PedestalYaml>,
}

#[derive(Debug, Deserialize)]
struct FixtureYaml {
    id: String,
    urdf: String,
    position: PositionYaml,
    orientation: OrientationYaml,
}

#[derive(Debug, Deserialize)]
struct PositionYaml {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Debug, Deserialize)]
struct OrientationYaml {
    a: f64,
    b: f64,
    c: f64,
}

#[derive(Debug, Deserialize)]
struct PedestalYaml {
    #[serde(default)]
    height_mm: f64,
    #[serde(default = "default_pedestal_diameter_mm")]
    diameter_mm: f64,
}

fn default_pedestal_diameter_mm() -> f64 {
    800.0
}

#[derive(Debug, Deserialize)]
struct FloorPlanYaml {
    geometry_file: String,
    #[serde(default)]
    offset: Option<PositionYaml>,
    /// Optional companion to `offset` for the editor's "pick cell origin"
    /// flow. Coordinate of the DXF/OBJ point that should map to world
    /// (0, 0, 0). When set without `offset`, parse_scene derives
    /// `effective_offset = scene_origin? - cell_origin_dxf`. When both
    /// are set, `offset` wins (it's the canonical math input); a
    /// disagreement emits a warning at validate-time.
    #[serde(default)]
    cell_origin_dxf: Option<PositionYaml>,
    #[serde(default = "default_scale")]
    scale: f64,
}

fn default_scale() -> f64 {
    1.0
}

// FR-100, FR-101, FR-102
pub fn parse_scene(scene_path: &Path) -> Result<SceneDefinition, FittingRoomError> {
    let content = std::fs::read_to_string(scene_path).map_err(|e| {
        FittingRoomError::SceneParse(format!("{}: {}", scene_path.display(), e))
    })?;
    let scene_dir = scene_path.parent().unwrap_or(Path::new("."));
    parse_scene_from_yaml(&content, scene_dir)
}

/// Parse a manifest from a YAML string. `scene_dir` is the directory used
/// to resolve relative paths (URDFs, floor plan). Useful for the HTTP
/// server's validate endpoint, which has the YAML in memory and a known
/// would-be parent directory but no on-disk file.
pub fn parse_scene_from_yaml(
    content: &str,
    scene_dir: &Path,
) -> Result<SceneDefinition, FittingRoomError> {
    let yaml: SceneYaml = serde_yaml::from_str(content)
        .map_err(|e| FittingRoomError::SceneParse(format!("yaml parse: {}", e)))?;

    let origin = yaml
        .scene_origin
        .as_ref()
        .map(|o| [o.x, o.y, o.z])
        .unwrap_or([0.0; 3]);

    let robots: Vec<RobotPlacement> = yaml
        .robots
        .into_iter()
        .map(|r| RobotPlacement {
            robot_id: r.robot_id,
            dressed_urdf: scene_dir.join(&r.dressed_urdf),
            position: [
                r.position.x - origin[0],
                r.position.y - origin[1],
                r.position.z - origin[2],
            ],
            orientation_deg: [r.orientation.a, r.orientation.b, r.orientation.c],
            home_pose_deg: r.home_pose_deg,
            pedestal: r.pedestal.map(|p| Pedestal {
                height_mm: p.height_mm,
                diameter_mm: p.diameter_mm,
            }),
        })
        .collect();

    let fixtures: Vec<FixturePlacement> = yaml
        .fixtures
        .into_iter()
        .map(|f| FixturePlacement {
            id: f.id,
            urdf: scene_dir.join(&f.urdf),
            position: [
                f.position.x - origin[0],
                f.position.y - origin[1],
                f.position.z - origin[2],
            ],
            orientation_deg: [f.orientation.a, f.orientation.b, f.orientation.c],
        })
        .collect();

    let floor_plan = match yaml.floor_plan {
        Some(fp) => {
            let obj_path = scene_dir.join(&fp.geometry_file);
            // Floor-plan offset shifts the OBJ-local coords into world space.
            // After scene_origin subtraction, the effective offset is
            // (manifest_offset - scene_origin).
            //
            // Step 3a: when `cell_origin_dxf` is set without an explicit
            // `offset`, derive `manifest_offset = -cell_origin_dxf` so the
            // chosen DXF point lands at the world origin. When both are
            // present, `offset` wins (canonical math input); validate_scene
            // reports any disagreement as a warning.
            let manifest_offset = match (fp.offset.as_ref(), fp.cell_origin_dxf.as_ref()) {
                (Some(o), _) => [o.x, o.y, o.z],
                (None, Some(c)) => [-c.x, -c.y, -c.z],
                (None, None) => [0.0; 3],
            };
            let effective_offset = [
                manifest_offset[0] - origin[0],
                manifest_offset[1] - origin[1],
                manifest_offset[2] - origin[2],
            ];
            Some(floor_plan::load_floor_plan(&obj_path, effective_offset, fp.scale)?)
        }
        None => None,
    };

    Ok(SceneDefinition {
        name: yaml.name,
        description: yaml.description.unwrap_or_default(),
        robots,
        fixtures,
        floor_plan,
    })
}

// FR-103
pub fn validate_scene(scene: &SceneDefinition) -> Vec<FittingRoomError> {
    let mut errors = Vec::new();

    for robot in &scene.robots {
        if !robot.dressed_urdf.exists() {
            errors.push(FittingRoomError::UrdfNotFound(
                robot.dressed_urdf.display().to_string(),
            ));
        } else if let Err(e) = validate_urdf_file(&robot.dressed_urdf) {
            errors.push(e);
        }
        if !is_finite_placement(robot.position, robot.orientation_deg) {
            errors.push(FittingRoomError::Validation(format!(
                "Robot '{}': non-finite position or orientation",
                robot.robot_id,
            )));
        }

        // FR-118: when home_pose_deg is set, its length must match the
        // URDF's actuated-joint count (fixed joints excluded). We only
        // run this check when the URDF is parseable; URDF parse errors
        // already get reported above.
        if let Some(angles) = &robot.home_pose_deg {
            if !angles.iter().all(|a| a.is_finite()) {
                errors.push(FittingRoomError::Validation(format!(
                    "Robot '{}': home_pose_deg contains non-finite values",
                    robot.robot_id,
                )));
            }
            if robot.dressed_urdf.exists() {
                match parse_urdf(&robot.dressed_urdf) {
                    Ok(urdf) => {
                        let actuated = urdf
                            .joints
                            .iter()
                            .filter(|j| !matches!(j.joint_type, JointType::Fixed))
                            .count();
                        if angles.len() != actuated {
                            errors.push(FittingRoomError::Validation(format!(
                                "Robot '{}': home_pose_deg has {} entries but URDF has {} actuated joints",
                                robot.robot_id,
                                angles.len(),
                                actuated,
                            )));
                        }
                    }
                    Err(_) => {
                        // URDF parse error already surfaced via validate_urdf_file.
                    }
                }
            }
        }

        // FR-119: pedestal height must be non-negative, diameter positive.
        if let Some(ped) = &robot.pedestal {
            if !ped.height_mm.is_finite() || ped.height_mm < 0.0 {
                errors.push(FittingRoomError::Validation(format!(
                    "Robot '{}': pedestal height_mm must be finite and >= 0",
                    robot.robot_id,
                )));
            }
            if !ped.diameter_mm.is_finite() || ped.diameter_mm <= 0.0 {
                errors.push(FittingRoomError::Validation(format!(
                    "Robot '{}': pedestal diameter_mm must be finite and > 0",
                    robot.robot_id,
                )));
            }
        }
    }

    for fixture in &scene.fixtures {
        if !fixture.urdf.exists() {
            errors.push(FittingRoomError::UrdfNotFound(
                fixture.urdf.display().to_string(),
            ));
        } else if let Err(e) = validate_urdf_file(&fixture.urdf) {
            errors.push(e);
        }
        if !is_finite_placement(fixture.position, fixture.orientation_deg) {
            errors.push(FittingRoomError::Validation(format!(
                "Fixture '{}': non-finite position or orientation",
                fixture.id,
            )));
        }
    }

    errors
}

fn validate_urdf_file(path: &Path) -> Result<(), FittingRoomError> {
    let content = std::fs::read_to_string(path).map_err(|e| FittingRoomError::UrdfInvalid {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    if !content.contains("<robot") {
        return Err(FittingRoomError::UrdfInvalid {
            path: path.display().to_string(),
            detail: "file does not contain a <robot> element".into(),
        });
    }
    Ok(())
}

fn is_finite_placement(position: [f64; 3], orientation: [f64; 3]) -> bool {
    position.iter().all(|v| v.is_finite()) && orientation.iter().all(|v| v.is_finite())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_minimal_scene() {
        let dir = tempfile::tempdir().unwrap();
        let scene_path = dir.path().join("test.yaml");
        let mut f = std::fs::File::create(&scene_path).unwrap();
        write!(
            f,
            "name: \"Test Cell\"\ndescription: \"A test scene\"\nrobots: []\nfixtures: []\n"
        )
        .unwrap();

        let scene = parse_scene(&scene_path).unwrap();
        assert_eq!(scene.name, "Test Cell");
        assert_eq!(scene.description, "A test scene");
        assert!(scene.robots.is_empty());
        assert!(scene.fixtures.is_empty());
        assert!(scene.floor_plan.is_none());
    }

    #[test]
    fn parse_scene_with_placements() {
        let dir = tempfile::tempdir().unwrap();
        let scene_path = dir.path().join("cell.yaml");
        let mut f = std::fs::File::create(&scene_path).unwrap();
        write!(
            f,
            r#"name: "MUC14"
robots:
  - robot_id: "M14A01"
    dressed_urdf: "robots/kr240.urdf"
    position: {{ x: 64.534, y: 31.123, z: 0.0 }}
    orientation: {{ a: 270.0, b: 0.0, c: 0.0 }}
fixtures:
  - id: "fixture_01"
    urdf: "fixtures/jig.urdf"
    position: {{ x: 65.0, y: 33.0, z: 0.0 }}
    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}
"#
        )
        .unwrap();

        let scene = parse_scene(&scene_path).unwrap();
        assert_eq!(scene.robots.len(), 1);
        assert_eq!(scene.robots[0].robot_id, "M14A01");
        assert!((scene.robots[0].position[0] - 64.534).abs() < 1e-6);
        assert!((scene.robots[0].orientation_deg[0] - 270.0).abs() < 1e-6);

        assert_eq!(scene.fixtures.len(), 1);
        assert_eq!(scene.fixtures[0].id, "fixture_01");
    }

    #[test]
    fn parse_scene_with_pedestal() {
        let dir = tempfile::tempdir().unwrap();
        let scene_path = dir.path().join("cell.yaml");
        let mut f = std::fs::File::create(&scene_path).unwrap();
        write!(
            f,
            r#"name: "Pedestal"
robots:
  - robot_id: "R1"
    dressed_urdf: "robots/kr240.urdf"
    position: {{ x: 1.0, y: 2.0, z: 0.0 }}
    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}
    pedestal:
      height_mm: 500.0
  - robot_id: "R2"
    dressed_urdf: "robots/kr240.urdf"
    position: {{ x: 3.0, y: 4.0, z: 0.0 }}
    orientation: {{ a: 0.0, b: 0.0, c: 0.0 }}
"#
        )
        .unwrap();

        let scene = parse_scene(&scene_path).unwrap();
        // R1 has a pedestal; diameter defaults to 800mm when omitted.
        let ped = scene.robots[0].pedestal.unwrap();
        assert!((ped.height_mm - 500.0).abs() < 1e-6);
        assert!((ped.diameter_mm - 800.0).abs() < 1e-6);
        // R2 has no pedestal block at all.
        assert!(scene.robots[1].pedestal.is_none());
    }

    #[test]
    fn parse_scene_with_floor_plan() {
        let dir = tempfile::tempdir().unwrap();

        // Create a minimal OBJ floor plan
        let obj_path = dir.path().join("plan.obj");
        std::fs::write(&obj_path, "v 0.0 0.0 0.0\nv 1.0 0.0 0.0\nl 1 2\n").unwrap();

        let scene_path = dir.path().join("cell.yaml");
        let mut f = std::fs::File::create(&scene_path).unwrap();
        write!(
            f,
            "name: \"Test\"\nfloor_plan:\n  geometry_file: \"plan.obj\"\n  scale: 1.0\n"
        )
        .unwrap();

        let scene = parse_scene(&scene_path).unwrap();
        let fp = scene.floor_plan.unwrap();
        assert_eq!(fp.segments.len(), 1);
        assert_eq!(fp.segments[0][0], [0.0, 0.0, 0.0]);
        assert_eq!(fp.segments[0][1], [1.0, 0.0, 0.0]);
    }

    #[test]
    fn validate_missing_urdf() {
        let scene = SceneDefinition {
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

        let errors = validate_scene(&scene);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], FittingRoomError::UrdfNotFound(_)));
    }

    #[test]
    fn validate_non_finite_position() {
        let scene = SceneDefinition {
            name: "test".into(),
            description: String::new(),
            robots: vec![],
            fixtures: vec![FixturePlacement {
                id: "F1".into(),
                urdf: PathBuf::from("/nonexistent/fixture.urdf"),
                position: [f64::NAN, 0.0, 0.0],
                orientation_deg: [0.0; 3],
            }],
            floor_plan: None,
        };

        let errors = validate_scene(&scene);
        assert!(errors
            .iter()
            .any(|e| matches!(e, FittingRoomError::Validation(_))));
    }

    #[test]
    fn validate_invalid_urdf_content() {
        let dir = tempfile::tempdir().unwrap();
        let urdf_path = dir.path().join("bad.urdf");
        std::fs::write(&urdf_path, "this is not a URDF file").unwrap();

        let scene = SceneDefinition {
            name: "test".into(),
            description: String::new(),
            robots: vec![RobotPlacement {
                robot_id: "R1".into(),
                dressed_urdf: urdf_path,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };

        let errors = validate_scene(&scene);
        assert_eq!(errors.len(), 1);
        assert!(matches!(errors[0], FittingRoomError::UrdfInvalid { .. }));
    }
}
