use robo_fitting_room::SceneDefinition;
use serde::Serialize;

use crate::config::RunConfig;
use crate::error::R3Error;

// FR-300. Re-implemented byte-for-byte from robo-fitting-room's private
// compose::sanitize_ros_segment so R3 resolves the exact topic strings Isaac
// subscribes to. rmw token contract: leading digit (or empty) gets an `r_`
// prefix; every non-[A-Za-z0-9_] becomes `_`; empty result becomes `r_anon`.
pub fn sanitize_ros_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len() + 2);
    let needs_prefix = segment
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true);
    if needs_prefix {
        out.push_str("r_");
    }
    for c in segment.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "r_anon".to_string();
    }
    out
}

// FR-300. Two-segment ROS 2 namespace `<cell_id>/<robot_id>`, each sanitized.
pub fn ros_namespace(cell_id: &str, robot_id: &str) -> String {
    format!(
        "{}/{}",
        sanitize_ros_segment(cell_id),
        sanitize_ros_segment(robot_id)
    )
}

/// One robot's resolved joint_command topic plus whether the CSV carries data
/// for it (a robot can be in the cell manifest but absent from the capture).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct ResolvedTopic {
    pub robot_id: String,
    pub topic: String,
    pub has_csv_data: bool,
}

// FR-301, FR-303, FR-304. Build the per-robot joint_command topic for every
// robot in the scene. `csv_robots` flags which robots the capture covers;
// `topic_overrides`/`cell_ns_override` from config win over the derived value.
pub fn resolve_topics(
    scene: &SceneDefinition,
    cfg: &RunConfig,
    csv_robots: &[String],
) -> Result<Vec<ResolvedTopic>, R3Error> {
    let cell_ns = match &cfg.cell_ns_override {
        Some(ns) => sanitize_ros_segment(ns),
        None => sanitize_ros_segment(&scene.name),
    };

    let mut out = Vec::with_capacity(scene.robots.len());
    for r in &scene.robots {
        let topic = match cfg.topic_overrides.get(&r.robot_id) {
            Some(t) => t.clone(),
            None => format!(
                "/{}/{}/joint_command",
                cell_ns,
                sanitize_ros_segment(&r.robot_id)
            ),
        };
        out.push(ResolvedTopic {
            robot_id: r.robot_id.clone(),
            topic,
            has_csv_data: csv_robots.iter().any(|c| c == &r.robot_id),
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verifies the sanitizer matches the values authored in ros-topics.txt.
    #[test]
    fn sanitize_matches_capture_contract() {
        assert_eq!(sanitize_ros_segment("113190R01"), "r_113190R01");
        assert_eq!(
            sanitize_ros_segment("K8URW11 Production Cell"),
            "K8URW11_Production_Cell"
        );
        // Letter-leading IDs are not prefixed; only digit-leading ones are.
        assert_eq!(sanitize_ros_segment("R1"), "R1");
        // Empty input yields the bare prefix (matches fitting-room).
        assert_eq!(sanitize_ros_segment(""), "r_");
        assert_eq!(sanitize_ros_segment("joint_a1"), "joint_a1");
    }

    #[test]
    fn namespace_joins_two_segments() {
        assert_eq!(
            ros_namespace("K8URW11 Production Cell", "113190R01"),
            "K8URW11_Production_Cell/r_113190R01"
        );
    }
}
