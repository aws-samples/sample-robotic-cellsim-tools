use serde::{Deserialize, Serialize};

// FR-402. `sensor_msgs/msg/JointState` mirrored as a CDR-serializable struct.
// Field order/types match the ROS IDL so Isaac's ROS2SubscribeJointState
// deserializes it. velocity/effort are sent empty (a position-only command;
// the Isaac drive owns velocity/effort). The matching DDS type name is
// `sensor_msgs::msg::dds_::JointState_`.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Time {
    pub sec: i32,
    pub nanosec: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub stamp: Time,
    pub frame_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointStateMsg {
    pub header: Header,
    pub name: Vec<String>,
    pub position: Vec<f64>,
    pub velocity: Vec<f64>,
    pub effort: Vec<f64>,
}

impl JointStateMsg {
    // FR-404. Build a position-only command. frame_id is empty; velocity and
    // effort are empty vectors.
    pub fn position_command(stamp: Time, names: &[String], positions: &[f64]) -> Self {
        Self {
            header: Header {
                stamp,
                frame_id: String::new(),
            },
            name: names.to_vec(),
            position: positions.to_vec(),
            velocity: Vec::new(),
            effort: Vec::new(),
        }
    }
}
