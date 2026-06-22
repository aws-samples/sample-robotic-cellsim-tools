pub mod dds;
pub mod jointstate;
pub mod stdout;

pub use jointstate::JointStateMsg;
pub use stdout::StdoutSink;

use crate::config::SinkKind;
use crate::error::R3Error;
use crate::topics::ResolvedTopic;

/// A ROS 2 time stamp split into seconds + nanoseconds (the
/// `builtin_interfaces/Time` shape inside a JointState header).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub sec: i32,
    pub nanosec: u32,
}

impl Stamp {
    /// Decompose a fractional-seconds timestamp (`t_profinet_s`) into a stamp.
    pub fn from_seconds(t: f64) -> Self {
        let sec = t.floor();
        let nanosec = ((t - sec) * 1e9).round();
        Self {
            sec: sec as i32,
            nanosec: nanosec as u32,
        }
    }
}

/// One joint-command publish: the resolved topic, the actuated joint names,
/// the radian positions, and the timestamp. Borrows so the engine reuses its
/// buffers across frames.
#[derive(Debug)]
pub struct JointCommand<'a> {
    pub topic: &'a str,
    pub names: &'a [String],
    pub positions: &'a [f64],
    pub stamp: Stamp,
}

/// The wire abstraction: every transport (stdout dry-run, real DDS, future
/// rosbridge) implements this. The playback engine is written entirely against
/// the trait and never knows which sink it drives.
pub trait CommandSink {
    // FR-400.
    fn publish_joint_command(&mut self, cmd: &JointCommand) -> Result<(), R3Error>;
    /// Pre-create writers / advertise topics. Default is a no-op.
    fn advertise(&mut self, _topics: &[ResolvedTopic]) -> Result<(), R3Error> {
        Ok(())
    }
    fn flush(&mut self) -> Result<(), R3Error> {
        Ok(())
    }
    fn kind(&self) -> &'static str;
}

// FR-401, FR-403, FR-405. Build the requested sink. DDS is the real (default)
// transport; the StdoutSink is the dry-run preview.
pub fn make_sink(kind: SinkKind, domain_id: u32) -> Result<Box<dyn CommandSink>, R3Error> {
    match kind {
        SinkKind::Dds => Ok(Box::new(dds::Ros2ClientSink::new(domain_id)?)),
        SinkKind::Stdout => Ok(Box::new(StdoutSink::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamp_splits_seconds_and_nanos() {
        let s = Stamp::from_seconds(1.5);
        assert_eq!(s.sec, 1);
        assert_eq!(s.nanosec, 500_000_000);
    }
}
