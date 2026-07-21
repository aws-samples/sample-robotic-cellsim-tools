#![forbid(unsafe_code)]

pub mod config;
pub mod csv_intake;
pub mod error;
pub mod mapping;
pub mod playback;
pub mod server;
pub mod topics;
pub mod transport;

pub use config::{
    apply_cli_overrides, load_run_config, CliOverrides, CsvFormat, EmptyCellPolicy, RunConfig,
    SinkKind, SinkOptions,
};
pub use csv_intake::{frame_reader, inspect_csv, CsvSchema, Frame};
pub use error::R3Error;
pub use mapping::{
    joint_limits_for_robot, map_degrees_to_radians, AxisMap, JointLimits, MappingConfig,
};
pub use playback::{
    build_plan, publish_snapshot, run_playback, CancelFlag, Clock, PlaybackPlan, PlaybackReport,
    RealClock,
};
pub use topics::{resolve_topics, ros_namespace, sanitize_ros_segment, ResolvedTopic};
pub use transport::{make_sink, CommandSink, JointCommand, Stamp};
