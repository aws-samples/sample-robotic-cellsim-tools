use std::fmt::Write as _;

use crate::error::R3Error;
use crate::transport::{CommandSink, JointCommand};

/// Zero-dependency sink: renders each publish as the equivalent
/// `ros2 topic pub` one-liner. This is the default transport and the "dry run"
/// — it proves the full pipeline (topics, joint names, radian positions)
/// without any DDS stack, and its output is the golden-snapshot source.
pub struct StdoutSink {
    /// Capture buffer for tests; when `None`, lines go to stdout.
    sink: Option<String>,
    count: u64,
}

impl StdoutSink {
    pub fn new() -> Self {
        Self {
            sink: None,
            count: 0,
        }
    }

    /// Construct a capturing sink whose lines accumulate in an internal buffer
    /// (drained via [`StdoutSink::take`]) instead of printing. Used by tests.
    pub fn capturing() -> Self {
        Self {
            sink: Some(String::new()),
            count: 0,
        }
    }

    pub fn take(&mut self) -> String {
        self.sink.take().unwrap_or_default()
    }

    /// Render one command as a deterministic single line. Positions are fixed
    /// to 6 decimals so the golden snapshot is stable across platforms.
    fn render(cmd: &JointCommand) -> String {
        let names = cmd
            .names
            .iter()
            .map(|n| format!("'{}'", n))
            .collect::<Vec<_>>()
            .join(",");
        let positions = cmd
            .positions
            .iter()
            .map(|p| format!("{:.6}", p))
            .collect::<Vec<_>>()
            .join(",");
        let mut line = String::new();
        let _ = write!(
            line,
            "ros2 topic pub --once {} sensor_msgs/msg/JointState \
             \"{{header: {{stamp: {{sec: {}, nanosec: {}}}}}, \
             name: [{}], position: [{}]}}\"",
            cmd.topic, cmd.stamp.sec, cmd.stamp.nanosec, names, positions
        );
        line
    }
}

impl Default for StdoutSink {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandSink for StdoutSink {
    fn publish_joint_command(&mut self, cmd: &JointCommand) -> Result<(), R3Error> {
        let line = Self::render(cmd);
        self.count += 1;
        match &mut self.sink {
            Some(buf) => {
                buf.push_str(&line);
                buf.push('\n');
            }
            None => println!("{}", line),
        }
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "stdout"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Stamp;

    #[test]
    fn renders_deterministic_line() {
        let names = vec!["joint_a1".to_string(), "joint_a2".to_string()];
        let positions = vec![0.0, -1.5707963];
        let cmd = JointCommand {
            topic: "/cell/r_R1/joint_command",
            names: &names,
            positions: &positions,
            stamp: Stamp {
                sec: 2,
                nanosec: 250_000_000,
            },
        };
        let line = StdoutSink::render(&cmd);
        assert_eq!(
            line,
            "ros2 topic pub --once /cell/r_R1/joint_command sensor_msgs/msg/JointState \
             \"{header: {stamp: {sec: 2, nanosec: 250000000}}, \
             name: ['joint_a1','joint_a2'], position: [0.000000,-1.570796]}\""
        );
    }

    #[test]
    fn capturing_sink_accumulates() {
        let mut s = StdoutSink::capturing();
        let names = vec!["joint_a1".to_string()];
        let positions = vec![0.0];
        let cmd = JointCommand {
            topic: "/t/r_R1/joint_command",
            names: &names,
            positions: &positions,
            stamp: Stamp { sec: 0, nanosec: 0 },
        };
        s.publish_joint_command(&cmd).unwrap();
        s.publish_joint_command(&cmd).unwrap();
        assert_eq!(s.take().lines().count(), 2);
    }
}
