// FR-407. Zenoh transport: publish joint commands over Zenoh (TCP unicast) to
// an explicit endpoint, where robo-zenoh-bridge republishes them onto Isaac's
// local ROS 2 / DDS. This is the cross-host / cross-VPC path DDS multicast
// discovery can't take. Pure-Rust (transport_tcp only), driven synchronously
// via the zenoh `Wait` trait — no async runtime.
//
// The external crate is `::zenoh`; this module is `crate::transport::zenoh`.

use serde::{Deserialize, Serialize};
use zenoh::{Config, Session, Wait};

use crate::error::R3Error;
use crate::transport::{CommandSink, JointCommand};

/// Default Zenoh key-expr prefix. Both the sink and the bridge must agree on it.
pub const DEFAULT_KEY_PREFIX: &str = "r3";

/// ROS topic -> Zenoh key expr: `/a/b/c` -> `r3/a/b/c`.
///
/// Reversible and collision-free: every ROS segment R3 emits has already passed
/// `sanitize_ros_segment` (`[A-Za-z0-9_]` only), so there are no `//`, no
/// leading/trailing `/`, and no Zenoh wildcard bytes to escape.
pub fn topic_to_key_expr(prefix: &str, topic: &str) -> String {
    format!("{}/{}", prefix, topic.trim_start_matches('/'))
}

/// Zenoh key expr -> ROS topic: `r3/a/b/c` -> `/a/b/c`. Inverse of
/// [`topic_to_key_expr`].
pub fn key_expr_to_topic(prefix: &str, ke: &str) -> String {
    let body = ke
        .strip_prefix(prefix)
        .and_then(|s| s.strip_prefix('/'))
        .unwrap_or(ke);
    format!("/{}", body)
}

/// The Zenoh wire message. serde_json on the wire so field names travel — no
/// schema drift between the sink and the bridge. Owned (the bridge deserializes
/// into it). The topic is carried here (authoritative) as well as in the key
/// expr.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZenohJointCommand {
    pub topic: String,
    pub names: Vec<String>,
    pub positions: Vec<f64>,
    pub sec: i32,
    pub nanosec: u32,
}

impl ZenohJointCommand {
    pub fn from_command(cmd: &JointCommand) -> Self {
        Self {
            topic: cmd.topic.to_string(),
            names: cmd.names.to_vec(),
            positions: cmd.positions.to_vec(),
            sec: cmd.stamp.sec,
            nanosec: cmd.stamp.nanosec,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, R3Error> {
        serde_json::to_vec(self).map_err(|e| R3Error::Transport(format!("zenoh encode: {}", e)))
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, R3Error> {
        serde_json::from_slice(b).map_err(|e| R3Error::Transport(format!("zenoh decode: {}", e)))
    }
}

/// Publishes joint commands over Zenoh. One session; `session.put` per message
/// (no per-topic publisher cache — the key-expr parse is trivial at replay rate).
pub struct ZenohSink {
    session: Session,
    key_prefix: String,
}

impl ZenohSink {
    // Client mode + explicit connect endpoints, multicast scouting off (EC2 VPCs
    // have no multicast). `zenoh::open` returns a session that connects in the
    // background and retries, so R3 can start before the bridge is up.
    pub fn new(connect: &[String], key_prefix: &str) -> Result<Self, R3Error> {
        let mut config = Config::default();
        config
            .insert_json5("mode", r#""client""#)
            .map_err(|e| R3Error::Transport(format!("zenoh mode: {}", e)))?;
        config
            .insert_json5("scouting/multicast/enabled", "false")
            .map_err(|e| R3Error::Transport(format!("zenoh scouting: {}", e)))?;
        if !connect.is_empty() {
            // serde_json emits a correctly-quoted JSON array: ["tcp/1.2.3.4:7447"].
            let eps = serde_json::to_string(connect)
                .map_err(|e| R3Error::Transport(format!("zenoh endpoints: {}", e)))?;
            config
                .insert_json5("connect/endpoints", &eps)
                .map_err(|e| R3Error::Transport(format!("zenoh connect: {}", e)))?;
        }
        let session = zenoh::open(config)
            .wait()
            .map_err(|e| R3Error::Transport(format!("zenoh open: {}", e)))?;
        Ok(Self {
            session,
            key_prefix: key_prefix.to_string(),
        })
    }
}

impl CommandSink for ZenohSink {
    fn publish_joint_command(&mut self, cmd: &JointCommand) -> Result<(), R3Error> {
        let key = topic_to_key_expr(&self.key_prefix, cmd.topic);
        let bytes = ZenohJointCommand::from_command(cmd).to_bytes()?;
        self.session
            .put(&key, bytes)
            .wait()
            .map_err(|e| R3Error::Transport(format!("zenoh put {}: {}", cmd.topic, e)))?;
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "zenoh"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_expr_round_trips() {
        let topic = "/K8URW11_Production_Cell/r_113190R01/joint_command";
        let ke = topic_to_key_expr("r3", topic);
        assert_eq!(ke, "r3/K8URW11_Production_Cell/r_113190R01/joint_command");
        assert_eq!(key_expr_to_topic("r3", &ke), topic);
    }

    #[test]
    fn wire_message_serde_round_trips() {
        let wire = ZenohJointCommand {
            topic: "/cell/r_x/joint_command".into(),
            names: vec!["joint_a1".into()],
            positions: vec![0.5],
            sec: 1,
            nanosec: 2,
        };
        assert_eq!(
            ZenohJointCommand::from_bytes(&wire.to_bytes().unwrap()).unwrap(),
            wire
        );
    }
}
