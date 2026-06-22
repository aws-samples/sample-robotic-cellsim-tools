// FR-403. The core ROS 2 transport: pure-Rust ros2-client (RustDDS). Speaks
// RTPS over a shared ROS_DOMAIN_ID straight to Isaac Sim's ROS 2 bridge — no
// rcl / ROS install, no native deps.
//
// Written against ros2-client 0.8 (rustdds re-exported as `ros2_client::rustdds`).
// The 0.7.x line resolves a broken rustdds 0.11.8 combination, so the crate is
// pinned to 0.8 in Cargo.toml.

use std::collections::HashMap;

use ros2_client::rustdds::{policy, Duration, QosPolicies, QosPolicyBuilder};
use ros2_client::{Context, ContextOptions, MessageTypeName, Name, Node, NodeName, NodeOptions};

use crate::error::R3Error;
use crate::topics::ResolvedTopic;
use crate::transport::jointstate::{JointStateMsg, Time};
use crate::transport::{CommandSink, JointCommand};

type JointPublisher = ros2_client::Publisher<JointStateMsg>;

pub struct Ros2ClientSink {
    node: Node,
    publishers: HashMap<String, JointPublisher>,
    advertised: bool,
}

impl Ros2ClientSink {
    pub fn new(domain_id: u32) -> Result<Self, R3Error> {
        let context = Context::with_options(ContextOptions::new().domain_id(domain_id as u16))
            .map_err(|e| R3Error::Transport(format!("ros2 context: {:?}", e)))?;
        let node = context
            .new_node(
                NodeName::new("/rustdds", "robo_ros_runner")
                    .map_err(|e| R3Error::Transport(format!("node name: {:?}", e)))?,
                NodeOptions::new().enable_rosout(false),
            )
            .map_err(|e| R3Error::Transport(format!("create node: {:?}", e)))?;
        Ok(Self {
            node,
            publishers: HashMap::new(),
            advertised: false,
        })
    }

    // Reliable + keep-last(1): a joint command is a "latest target wins"
    // stream; matches the Isaac bridge subscriber's default profile.
    fn qos() -> QosPolicies {
        QosPolicyBuilder::new()
            .reliability(policy::Reliability::Reliable {
                max_blocking_time: Duration::from_millis(100),
            })
            .history(policy::History::KeepLast { depth: 1 })
            .durability(policy::Durability::Volatile)
            .build()
    }

    fn publisher_for(&mut self, topic: &str) -> Result<&JointPublisher, R3Error> {
        if !self.publishers.contains_key(topic) {
            let qos = Self::qos();
            let topic_obj = self
                .node
                .create_topic(
                    &Name::parse(topic)
                        .map_err(|e| R3Error::Transport(format!("topic name {}: {:?}", topic, e)))?,
                    MessageTypeName::new("sensor_msgs", "JointState"),
                    &qos,
                )
                .map_err(|e| R3Error::Transport(format!("create topic {}: {:?}", topic, e)))?;
            let pubr = self
                .node
                .create_publisher::<JointStateMsg>(&topic_obj, Some(qos))
                .map_err(|e| R3Error::Transport(format!("create publisher {}: {:?}", topic, e)))?;
            self.publishers.insert(topic.to_string(), pubr);
        }
        Ok(self.publishers.get(topic).unwrap())
    }
}

impl CommandSink for Ros2ClientSink {
    fn advertise(&mut self, topics: &[ResolvedTopic]) -> Result<(), R3Error> {
        for t in topics {
            if t.has_csv_data {
                let _ = self.publisher_for(&t.topic)?;
            }
        }
        // Give DDS discovery a moment to match the Isaac-side subscribers
        // before the first publish, so early frames aren't dropped.
        if !topics.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        self.advertised = true;
        Ok(())
    }

    fn publish_joint_command(&mut self, cmd: &JointCommand) -> Result<(), R3Error> {
        let msg = JointStateMsg::position_command(
            Time {
                sec: cmd.stamp.sec,
                nanosec: cmd.stamp.nanosec,
            },
            cmd.names,
            cmd.positions,
        );
        let pubr = self.publisher_for(cmd.topic)?;
        pubr.publish(msg)
            .map_err(|e| R3Error::Transport(format!("publish {}: {:?}", cmd.topic, e)))?;
        Ok(())
    }

    fn kind(&self) -> &'static str {
        "dds"
    }
}
