#![forbid(unsafe_code)]

pub mod compose;
pub mod error;
pub mod floor_plan;
pub mod math3;
pub mod mtl;
pub mod obj;
pub mod scene;

#[cfg(feature = "server")]
pub mod server;

pub use compose::{
    compose_stage, compose_stage_with_options, compose_urdf_bundle, ComposeOptions, ComposeReport,
    UrdfBundleReport,
};
pub use error::FittingRoomError;
pub use floor_plan::FloorPlan;
pub use scene::{
    parse_scene, parse_scene_from_yaml, validate_scene, FixturePlacement, Pedestal,
    RobotPlacement, SceneDefinition,
};
