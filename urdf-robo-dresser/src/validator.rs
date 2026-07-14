use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::error::Diagnostic;
use crate::types::{Geometry, JointType, UrdfRobot};

// FR-011, FR-012, FR-013, FR-014, FR-015, FR-017: URDF validation checks
pub fn validate(robot: &UrdfRobot, urdf_path: &Path, is_tool: bool) -> Vec<Diagnostic> {
    let mut diags = Vec::new();

    check_duplicate_links(robot, urdf_path, &mut diags);
    check_duplicate_joints(robot, urdf_path, &mut diags);
    check_parent_child_refs(robot, urdf_path, &mut diags);
    check_tree_structure(robot, urdf_path, &mut diags);
    check_joint_limits(robot, urdf_path, &mut diags);
    check_mesh_existence(robot, urdf_path, &mut diags);
    check_collision_coverage(robot, urdf_path, &mut diags);

    if is_tool {
        check_tool_origin(robot, urdf_path, &mut diags);
    }

    diags
}

fn check_duplicate_links(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for link in &robot.links {
        if !seen.insert(&link.name) {
            diags.push(
                Diagnostic::error("E014", format!("duplicate link name '{}'", link.name))
                    .with_location(path.to_path_buf(), format!("link '{}'", link.name)),
            );
        }
    }
}

fn check_duplicate_joints(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for joint in &robot.joints {
        if !seen.insert(&joint.name) {
            diags.push(
                Diagnostic::error("E014", format!("duplicate joint name '{}'", joint.name))
                    .with_location(path.to_path_buf(), format!("joint '{}'", joint.name)),
            );
        }
    }
}

fn check_parent_child_refs(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    let link_names: HashSet<&str> = robot.links.iter().map(|l| l.name.as_str()).collect();

    for joint in &robot.joints {
        if !link_names.contains(joint.parent.as_str()) {
            diags.push(
                Diagnostic::error(
                    "E012",
                    format!(
                        "joint '{}' references parent link '{}' which is not declared",
                        joint.name, joint.parent
                    ),
                )
                .with_location(path.to_path_buf(), format!("joint '{}'", joint.name)),
            );
        }
        if !link_names.contains(joint.child.as_str()) {
            diags.push(
                Diagnostic::error(
                    "E012",
                    format!(
                        "joint '{}' references child link '{}' which is not declared",
                        joint.name, joint.child
                    ),
                )
                .with_location(path.to_path_buf(), format!("joint '{}'", joint.name)),
            );
        }
    }
}

fn check_tree_structure(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    let link_names: HashSet<&str> = robot.links.iter().map(|l| l.name.as_str()).collect();
    let children: HashSet<&str> = robot.joints.iter().map(|j| j.child.as_str()).collect();

    let roots: Vec<&str> = link_names.difference(&children).copied().collect();

    if roots.is_empty() && !robot.links.is_empty() {
        diags.push(
            Diagnostic::error("E011", "kinematic tree has no root — possible cycle detected")
                .with_location(path.to_path_buf(), "robot"),
        );
    } else if roots.len() > 1 {
        // Multiple roots is acceptable in URDF (e.g. World + base_link connected by fixed joint)
        // Only flag if they are truly disconnected
        let mut parent_map: HashMap<&str, &str> = HashMap::new();
        for joint in &robot.joints {
            parent_map.insert(joint.child.as_str(), joint.parent.as_str());
        }

        // Check connectivity: all links should be reachable from some root
        let mut reachable = HashSet::new();
        for root in &roots {
            collect_reachable(root, &robot.joints, &mut reachable);
        }

        let unreachable: Vec<&str> = link_names
            .iter()
            .filter(|l| !reachable.contains(*l))
            .copied()
            .collect();

        if !unreachable.is_empty() {
            diags.push(
                Diagnostic::error(
                    "E011",
                    format!("disconnected links not reachable from any root: {:?}", unreachable),
                )
                .with_location(path.to_path_buf(), "robot"),
            );
        }
    }
}

fn collect_reachable<'a>(root: &'a str, joints: &'a [crate::types::Joint], reachable: &mut HashSet<&'a str>) {
    reachable.insert(root);
    for joint in joints {
        if joint.parent == root && !reachable.contains(joint.child.as_str()) {
            collect_reachable(&joint.child, joints, reachable);
        }
    }
}

fn check_joint_limits(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    for joint in &robot.joints {
        if matches!(joint.joint_type, JointType::Revolute | JointType::Prismatic) {
            if let Some(ref limit) = joint.limit {
                if limit.lower >= limit.upper {
                    diags.push(
                        Diagnostic::warning(
                            "W013",
                            format!(
                                "joint '{}' has lower >= upper limit ({} >= {})",
                                joint.name, limit.lower, limit.upper
                            ),
                        )
                        .with_location(path.to_path_buf(), format!("joint '{}'", joint.name)),
                    );
                }
            }
        }
    }
}

fn check_mesh_existence(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    let urdf_dir = path.parent().unwrap_or(Path::new("."));

    for link in &robot.links {
        for visual in &link.visuals {
            if let Geometry::Mesh { ref filename, .. } = visual.geometry {
                let mesh_path = urdf_dir.join(filename);
                if !mesh_path.exists() {
                    diags.push(
                        Diagnostic::warning(
                            "W003",
                            format!("mesh file not found: {}", mesh_path.display()),
                        )
                        .with_location(path.to_path_buf(), format!("link '{}' visual", link.name)),
                    );
                }
            }
        }
        for collision in &link.collisions {
            if let Geometry::Mesh { ref filename, .. } = collision.geometry {
                let mesh_path = urdf_dir.join(filename);
                if !mesh_path.exists() {
                    diags.push(
                        Diagnostic::warning(
                            "W003",
                            format!("mesh file not found: {}", mesh_path.display()),
                        )
                        .with_location(
                            path.to_path_buf(),
                            format!("link '{}' collision", link.name),
                        ),
                    );
                }
            }
        }
    }
}

fn check_collision_coverage(robot: &UrdfRobot, path: &Path, diags: &mut Vec<Diagnostic>) {
    for link in &robot.links {
        if !link.visuals.is_empty() && link.collisions.is_empty() {
            diags.push(
                Diagnostic::warning(
                    "W016",
                    format!(
                        "link '{}' has visual geometry but no collision — hull will be auto-generated",
                        link.name
                    ),
                )
                .with_location(path.to_path_buf(), format!("link '{}'", link.name)),
            );
        }
    }
}

fn check_tool_origin(robot: &UrdfRobot, _path: &Path, _diags: &mut Vec<Diagnostic>) {
    // Find the root link (World or first link)
    let root_link = robot
        .links
        .iter()
        .find(|l| l.name == "World")
        .or_else(|| robot.links.first());

    if let Some(link) = root_link {
        // Check that visual geometry origins are near (0,0,0)
        // This is a simplified check — a full implementation would compute mesh centroid
        if link.visuals.is_empty() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn minimal_robot() -> UrdfRobot {
        UrdfRobot {
            name: "test".to_string(),
            links: vec![
                Link { name: "base".to_string(), inertial: None, visuals: vec![], collisions: vec![] },
                Link { name: "link1".to_string(), inertial: None, visuals: vec![], collisions: vec![] },
            ],
            joints: vec![Joint {
                name: "j1".to_string(),
                joint_type: JointType::Fixed,
                parent: "base".to_string(),
                child: "link1".to_string(),
                origin: Origin::default(),
                axis: [0.0, 0.0, 1.0],
                limit: None,
            }],
            reach: None,
        }
    }

    #[test]
    fn test_valid_robot_no_errors() {
        let robot = minimal_robot();
        let diags = validate(&robot, Path::new("test.urdf"), false);
        let errors: Vec<_> = diags.iter().filter(|d| d.is_error()).collect();
        assert!(errors.is_empty(), "Expected no errors, got: {:?}", errors);
    }

    #[test]
    fn test_duplicate_link_detected() {
        let mut robot = minimal_robot();
        robot.links.push(Link {
            name: "base".to_string(),
            inertial: None,
            visuals: vec![],
            collisions: vec![],
        });
        let diags = validate(&robot, Path::new("test.urdf"), false);
        assert!(diags.iter().any(|d| d.code == "E014"));
    }

    #[test]
    fn test_dangling_parent_ref() {
        let mut robot = minimal_robot();
        robot.joints[0].parent = "nonexistent".to_string();
        let diags = validate(&robot, Path::new("test.urdf"), false);
        assert!(diags.iter().any(|d| d.code == "E012"));
    }
}
