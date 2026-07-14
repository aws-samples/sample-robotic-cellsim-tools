use std::path::Path;

use quick_xml::events::Event;

use crate::error::DresserError;
use crate::types::{
    Collision, Geometry, Inertial, InertiaMatrix, Joint, JointLimit, JointType, Link, Origin,
    UrdfRobot, Visual,
};

// FR-010: Parse URDF XML into typed kinematic tree
pub fn parse_urdf(path: &Path) -> Result<UrdfRobot, DresserError> {
    let bytes = std::fs::read(path).map_err(|e| DresserError::UrdfParse {
        path: path.display().to_string(),
        detail: format!("failed to read file: {}", e),
    })?;
    parse_urdf_bytes(&bytes, &path.display().to_string())
}

pub fn parse_urdf_bytes(bytes: &[u8], source: &str) -> Result<UrdfRobot, DresserError> {
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buf = Vec::with_capacity(256);

    let mut robot_name = String::new();
    let mut links: Vec<Link> = Vec::new();
    let mut joints: Vec<Joint> = Vec::new();
    let mut reach: Option<crate::reach::ReachEnvelope> = None;

    let mut state = ParseState::Root;

    loop {
        let event = reader.read_event_into(&mut buf);
        match event {
            Ok(Event::Start(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                match (tag_name.as_str(), &state) {
                    ("robot", ParseState::Root) => {
                        robot_name = get_attr_str(e, b"name").unwrap_or_default();
                        state = ParseState::InRobot;
                    }
                    ("link", ParseState::Root | ParseState::InRobot) => {
                        let name = get_attr_str(e, b"name").unwrap_or_default();
                        state = ParseState::InLink(PartialLink::new(name));
                    }
                    ("joint", ParseState::Root | ParseState::InRobot) => {
                        let name = get_attr_str(e, b"name").unwrap_or_default();
                        let jtype_str = get_attr_str(e, b"type").unwrap_or_default();
                        let jtype = JointType::from_str(&jtype_str);
                        state = ParseState::InJoint(PartialJoint::new(name, jtype));
                    }
                    ("visual", _) if matches!(state, ParseState::InLink(_)) => {
                        state = state.enter_visual();
                    }
                    ("collision", _) if matches!(state, ParseState::InLink(_)) => {
                        state = state.enter_collision();
                    }
                    ("inertial", _) if matches!(state, ParseState::InLink(_)) => {
                        state = state.enter_inertial();
                    }
                    _ => {
                        dispatch_inner_element(&mut state, &tag_name, e, source)?;
                    }
                }
            }
            Ok(Event::Empty(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                match (tag_name.as_str(), &state) {
                    ("link", ParseState::Root | ParseState::InRobot) => {
                        let name = get_attr_str(e, b"name").unwrap_or_default();
                        links.push(Link {
                            name,
                            inertial: None,
                            visuals: Vec::new(),
                            collisions: Vec::new(),
                        });
                    }
                    ("reach_envelope", ParseState::Root | ParseState::InRobot) => {
                        reach = Some(crate::reach::ReachEnvelope {
                            max_radius_2d: get_attr_f64(e, b"max_radius_2d").unwrap_or(0.0),
                            volume_3d: get_attr_f64(e, b"volume_3d").unwrap_or(0.0),
                            hull_obj_rel: get_attr_str(e, b"hull_mesh").unwrap_or_default(),
                        });
                    }
                    _ => {
                        dispatch_inner_element(&mut state, &tag_name, e, source)?;
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let tag_name = String::from_utf8_lossy(e.name().as_ref()).to_string();

                match (tag_name.as_str(), &state) {
                    ("link", ParseState::InLink(_)) => {
                        if let ParseState::InLink(partial) =
                            std::mem::replace(&mut state, ParseState::InRobot)
                        {
                            links.push(partial.into_link());
                        }
                    }
                    ("joint", ParseState::InJoint(_)) => {
                        if let ParseState::InJoint(partial) =
                            std::mem::replace(&mut state, ParseState::InRobot)
                        {
                            joints.push(partial.into_joint(source)?);
                        }
                    }
                    ("visual", _) if matches!(state, ParseState::InVisual { .. }) => {
                        state = state.exit_visual();
                    }
                    ("collision", _) if matches!(state, ParseState::InCollision { .. }) => {
                        state = state.exit_collision();
                    }
                    ("inertial", _) if matches!(state, ParseState::InInertial { .. }) => {
                        state = state.exit_inertial();
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(DresserError::UrdfParse {
                    path: source.to_string(),
                    detail: format!("XML syntax error: {}", e),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(UrdfRobot {
        name: robot_name,
        links,
        joints,
        reach,
    })
}

// --- State Machine ---

enum ParseState {
    Root,
    InRobot,
    InLink(PartialLink),
    InJoint(PartialJoint),
    InVisual {
        link: PartialLink,
        origin: Origin,
        geometry: Option<Geometry>,
    },
    InCollision {
        link: PartialLink,
        origin: Origin,
        geometry: Option<Geometry>,
    },
    InInertial {
        link: PartialLink,
        origin: Origin,
        mass: f64,
        inertia: InertiaMatrix,
    },
}

impl ParseState {
    fn enter_visual(self) -> Self {
        match self {
            ParseState::InLink(link) => ParseState::InVisual {
                link,
                origin: Origin::default(),
                geometry: None,
            },
            other => other,
        }
    }

    fn exit_visual(self) -> Self {
        match self {
            ParseState::InVisual {
                mut link,
                origin,
                geometry,
            } => {
                if let Some(geom) = geometry {
                    link.visuals.push(Visual {
                        origin,
                        geometry: geom,
                    });
                }
                ParseState::InLink(link)
            }
            other => other,
        }
    }

    fn enter_collision(self) -> Self {
        match self {
            ParseState::InLink(link) => ParseState::InCollision {
                link,
                origin: Origin::default(),
                geometry: None,
            },
            other => other,
        }
    }

    fn exit_collision(self) -> Self {
        match self {
            ParseState::InCollision {
                mut link,
                origin,
                geometry,
            } => {
                if let Some(geom) = geometry {
                    link.collisions.push(Collision {
                        origin,
                        geometry: geom,
                    });
                }
                ParseState::InLink(link)
            }
            other => other,
        }
    }

    fn enter_inertial(self) -> Self {
        match self {
            ParseState::InLink(link) => ParseState::InInertial {
                link,
                origin: Origin::default(),
                mass: 0.0,
                inertia: InertiaMatrix {
                    ixx: 0.0, ixy: 0.0, ixz: 0.0,
                    iyy: 0.0, iyz: 0.0, izz: 0.0,
                },
            },
            other => other,
        }
    }

    fn exit_inertial(self) -> Self {
        match self {
            ParseState::InInertial {
                mut link,
                origin,
                mass,
                inertia,
            } => {
                link.inertial = Some(Inertial {
                    origin,
                    mass,
                    inertia,
                });
                ParseState::InLink(link)
            }
            other => other,
        }
    }
}

fn dispatch_inner_element(
    state: &mut ParseState,
    tag_name: &str,
    e: &quick_xml::events::BytesStart,
    _source: &str,
) -> Result<(), DresserError> {
    match state {
        ParseState::InJoint(ref mut joint) => match tag_name {
            "parent" => {
                joint.parent = get_attr_str(e, b"link").unwrap_or_default();
            }
            "child" => {
                joint.child = get_attr_str(e, b"link").unwrap_or_default();
            }
            "origin" => {
                joint.origin = parse_origin_attrs(e);
            }
            "axis" => {
                joint.axis = parse_xyz_attr(e, b"xyz");
            }
            "limit" => {
                joint.limit = Some(JointLimit {
                    lower: get_attr_f64(e, b"lower").unwrap_or(0.0),
                    upper: get_attr_f64(e, b"upper").unwrap_or(0.0),
                    effort: get_attr_f64(e, b"effort").unwrap_or(0.0),
                    velocity: get_attr_f64(e, b"velocity").unwrap_or(0.0),
                });
            }
            _ => {}
        },
        ParseState::InVisual {
            ref mut origin,
            ref mut geometry,
            ..
        } => match tag_name {
            "origin" => *origin = parse_origin_attrs(e),
            "mesh" => *geometry = Some(parse_mesh_geometry(e)),
            "box" => *geometry = Some(parse_box_geometry(e)),
            "cylinder" => *geometry = Some(parse_cylinder_geometry(e)),
            "sphere" => *geometry = Some(parse_sphere_geometry(e)),
            _ => {}
        },
        ParseState::InCollision {
            ref mut origin,
            ref mut geometry,
            ..
        } => match tag_name {
            "origin" => *origin = parse_origin_attrs(e),
            "mesh" => *geometry = Some(parse_mesh_geometry(e)),
            "box" => *geometry = Some(parse_box_geometry(e)),
            "cylinder" => *geometry = Some(parse_cylinder_geometry(e)),
            "sphere" => *geometry = Some(parse_sphere_geometry(e)),
            _ => {}
        },
        ParseState::InInertial {
            ref mut origin,
            ref mut mass,
            ref mut inertia,
            ..
        } => match tag_name {
            "origin" => *origin = parse_origin_attrs(e),
            "mass" => *mass = get_attr_f64(e, b"value").unwrap_or(0.0),
            "inertia" => {
                inertia.ixx = get_attr_f64(e, b"ixx").unwrap_or(0.0);
                inertia.ixy = get_attr_f64(e, b"ixy").unwrap_or(0.0);
                inertia.ixz = get_attr_f64(e, b"ixz").unwrap_or(0.0);
                inertia.iyy = get_attr_f64(e, b"iyy").unwrap_or(0.0);
                inertia.iyz = get_attr_f64(e, b"iyz").unwrap_or(0.0);
                inertia.izz = get_attr_f64(e, b"izz").unwrap_or(0.0);
            }
            _ => {}
        },
        _ => {}
    }
    Ok(())
}

// --- Partial Accumulators ---

struct PartialLink {
    name: String,
    inertial: Option<Inertial>,
    visuals: Vec<Visual>,
    collisions: Vec<Collision>,
}

impl PartialLink {
    fn new(name: String) -> Self {
        Self {
            name,
            inertial: None,
            visuals: Vec::new(),
            collisions: Vec::new(),
        }
    }

    fn into_link(self) -> Link {
        Link {
            name: self.name,
            inertial: self.inertial,
            visuals: self.visuals,
            collisions: self.collisions,
        }
    }
}

struct PartialJoint {
    name: String,
    joint_type: Option<JointType>,
    parent: String,
    child: String,
    origin: Origin,
    axis: [f64; 3],
    limit: Option<JointLimit>,
}

impl PartialJoint {
    fn new(name: String, joint_type: Option<JointType>) -> Self {
        Self {
            name,
            joint_type,
            parent: String::new(),
            child: String::new(),
            origin: Origin::default(),
            axis: [0.0, 0.0, 1.0],
            limit: None,
        }
    }

    fn into_joint(self, source: &str) -> Result<Joint, DresserError> {
        let joint_type = self.joint_type.ok_or_else(|| DresserError::UrdfParse {
            path: source.to_string(),
            detail: format!("joint '{}' has unknown or missing type", self.name),
        })?;
        Ok(Joint {
            name: self.name,
            joint_type,
            parent: self.parent,
            child: self.child,
            origin: self.origin,
            axis: self.axis,
            limit: self.limit,
        })
    }
}

// --- Attribute Helpers ---

fn get_attr_str(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == key {
            return Some(String::from_utf8_lossy(&attr.value).to_string());
        }
    }
    None
}

fn get_attr_f64(e: &quick_xml::events::BytesStart, key: &[u8]) -> Option<f64> {
    get_attr_str(e, key).and_then(|s| s.trim().parse().ok())
}

fn parse_xyz(s: &str) -> [f64; 3] {
    let parts: Vec<f64> = s.split_whitespace().filter_map(|p| p.parse().ok()).collect();
    [
        parts.first().copied().unwrap_or(0.0),
        parts.get(1).copied().unwrap_or(0.0),
        parts.get(2).copied().unwrap_or(0.0),
    ]
}

fn parse_xyz_attr(e: &quick_xml::events::BytesStart, key: &[u8]) -> [f64; 3] {
    get_attr_str(e, key).map(|s| parse_xyz(&s)).unwrap_or([0.0; 3])
}

fn parse_origin_attrs(e: &quick_xml::events::BytesStart) -> Origin {
    Origin {
        xyz: parse_xyz_attr(e, b"xyz"),
        rpy: parse_xyz_attr(e, b"rpy"),
    }
}

fn parse_mesh_geometry(e: &quick_xml::events::BytesStart) -> Geometry {
    let filename = get_attr_str(e, b"filename").unwrap_or_default();
    let scale = get_attr_str(e, b"scale")
        .map(|s| parse_xyz(&s))
        .unwrap_or([1.0, 1.0, 1.0]);
    Geometry::Mesh { filename, scale }
}

fn parse_box_geometry(e: &quick_xml::events::BytesStart) -> Geometry {
    Geometry::Box {
        size: parse_xyz_attr(e, b"size"),
    }
}

fn parse_cylinder_geometry(e: &quick_xml::events::BytesStart) -> Geometry {
    Geometry::Cylinder {
        radius: get_attr_f64(e, b"radius").unwrap_or(0.0),
        length: get_attr_f64(e, b"length").unwrap_or(0.0),
    }
}

fn parse_sphere_geometry(e: &quick_xml::events::BytesStart) -> Geometry {
    Geometry::Sphere {
        radius: get_attr_f64(e, b"radius").unwrap_or(0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_xyz() {
        assert_eq!(parse_xyz("1.0 2.0 3.0"), [1.0, 2.0, 3.0]);
        assert_eq!(parse_xyz("0. 0. 0.860625"), [0.0, 0.0, 0.860625]);
        assert_eq!(parse_xyz(""), [0.0, 0.0, 0.0]);
        assert_eq!(parse_xyz("-1.5 2.5 -3.5"), [-1.5, 2.5, -3.5]);
    }

    fn find_first_urdf(dir: &Path) -> Option<std::path::PathBuf> {
        std::fs::read_dir(dir).ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().map(|x| x == "urdf").unwrap_or(false))
    }

    fn find_first_tool_urdf(dir: &Path) -> Option<std::path::PathBuf> {
        std::fs::read_dir(dir).ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .find_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let urdf = e.path().join(format!("{}.urdf", name));
                urdf.exists().then_some(urdf)
            })
    }

    #[test]
    fn test_parse_robot_urdf() {
        let robot_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-robot-collection");
        let urdf_path = match find_first_urdf(&robot_dir) {
            Some(p) => p,
            None => return,
        };
        let robot = parse_urdf(&urdf_path).expect("Failed to parse robot URDF");

        assert!(robot.joints.len() >= 7);
        assert!(robot.links.len() >= 8);

        let revolute_joints: Vec<_> = robot
            .joints
            .iter()
            .filter(|j| j.joint_type == JointType::Revolute)
            .collect();
        assert_eq!(revolute_joints.len(), 6);

        // First revolute joint should reference valid parent/child
        assert!(!revolute_joints[0].parent.is_empty());
        assert!(!revolute_joints[0].child.is_empty());

        // First non-World link should have inertial and geometry
        let first_body_link = robot.links.iter().find(|l| l.name != "World").unwrap();
        assert!(first_body_link.inertial.is_some());
        assert!(!first_body_link.visuals.is_empty());
        assert!(!first_body_link.collisions.is_empty());
    }

    #[test]
    fn test_parse_tool_urdf() {
        let tool_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-tool-collection");
        let urdf_path = match find_first_tool_urdf(&tool_dir) {
            Some(p) => p,
            None => return,
        };
        let robot = parse_urdf(&urdf_path).expect("Failed to parse tool URDF");

        // Tool should have at least one link with visual geometry
        let has_visuals = robot.links.iter().any(|l| !l.visuals.is_empty());
        assert!(has_visuals, "Tool should have visual geometry");
    }
}
