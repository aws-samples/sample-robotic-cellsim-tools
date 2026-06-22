#![forbid(unsafe_code)]

pub mod error;
pub mod types;
pub mod parser;
pub mod serializer;
pub mod validator;
pub mod merger;
pub mod mesh_reader;
pub mod collision;
pub mod catalogue;

#[cfg(feature = "server")]
pub mod server;

pub use error::{DresserError, Diagnostic, Severity};
pub use types::{UrdfRobot, Link, Joint, Origin, Visual, Collision, Geometry, JointType, JointLimit, Inertial, InertiaMatrix, MergeOptions};
pub use parser::{parse_urdf, parse_urdf_bytes};
pub use serializer::serialize_urdf;
pub use validator::validate;
pub use merger::merge;
