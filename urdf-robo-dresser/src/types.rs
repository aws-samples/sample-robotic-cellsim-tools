use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct UrdfRobot {
    pub name: String,
    pub links: Vec<Link>,
    pub joints: Vec<Joint>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub name: String,
    pub inertial: Option<Inertial>,
    pub visuals: Vec<Visual>,
    pub collisions: Vec<Collision>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    pub name: String,
    pub joint_type: JointType,
    pub parent: String,
    pub child: String,
    pub origin: Origin,
    pub axis: [f64; 3],
    pub limit: Option<JointLimit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Origin {
    pub xyz: [f64; 3],
    pub rpy: [f64; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Visual {
    pub origin: Origin,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Collision {
    pub origin: Origin,
    pub geometry: Geometry,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Mesh { filename: String, scale: [f64; 3] },
    Box { size: [f64; 3] },
    Cylinder { radius: f64, length: f64 },
    Sphere { radius: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JointType {
    Revolute,
    Continuous,
    Prismatic,
    Fixed,
    Floating,
    Planar,
}

impl JointType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "revolute" => Some(Self::Revolute),
            "continuous" => Some(Self::Continuous),
            "prismatic" => Some(Self::Prismatic),
            "fixed" => Some(Self::Fixed),
            "floating" => Some(Self::Floating),
            "planar" => Some(Self::Planar),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Revolute => "revolute",
            Self::Continuous => "continuous",
            Self::Prismatic => "prismatic",
            Self::Fixed => "fixed",
            Self::Floating => "floating",
            Self::Planar => "planar",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JointLimit {
    pub lower: f64,
    pub upper: f64,
    pub effort: f64,
    pub velocity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Inertial {
    pub origin: Origin,
    pub mass: f64,
    pub inertia: InertiaMatrix,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InertiaMatrix {
    pub ixx: f64,
    pub ixy: f64,
    pub ixz: f64,
    pub iyy: f64,
    pub iyz: f64,
    pub izz: f64,
}

#[derive(Debug, Clone)]
pub struct MergeOptions {
    pub attach_to: Option<String>,
    pub orient: [u16; 3],
    pub offset: Origin,
    pub tool_prefix: String,
    pub output_dir: PathBuf,
    pub robot_urdf_dir: PathBuf,
    pub tool_urdf_dir: PathBuf,
}

impl Default for MergeOptions {
    fn default() -> Self {
        Self {
            attach_to: None,
            orient: [0, 0, 0],
            offset: Origin::default(),
            tool_prefix: "tool__".to_string(),
            output_dir: PathBuf::from("."),
            robot_urdf_dir: PathBuf::from("."),
            tool_urdf_dir: PathBuf::from("."),
        }
    }
}
