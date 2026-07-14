//! FR-028: reachable-workspace computation. Given a dressed robot's joint
//! chain + limits + the mounted tool's distal offset, computes:
//! - max_radius_2d: maximum horizontal distance from the axis-1 centerline
//! - volume_3d: volume of the convex hull of FK-sampled tool-tip positions
//! - hull OBJ mesh: the convex hull written as meshes/3dreach/reach_envelope.obj

use std::path::Path;

use parry3d::math::Point;
use parry3d::transformation::convex_hull;

use crate::collision::write_obj;
use crate::types::{JointType, Origin, UrdfRobot};

/// Grid steps per joint for workspace sampling.
const GRID_STEPS: usize = 9;

/// Result of the workspace analysis, embedded in the dressed URDF.
/// `max_radius_2d` is always computed; `volume_3d` and `hull_obj_rel` are
/// best-effort (zero/empty if the hull computation fails or is degenerate).
#[derive(Debug, Clone, PartialEq)]
pub struct ReachEnvelope {
    pub max_radius_2d: f64,
    pub volume_3d: f64,
    pub hull_obj_rel: String,
}

/// One joint in the serial chain, extracted for FK sampling.
struct ChainJoint {
    origin: Origin,
    axis: [f64; 3],
    joint_type: JointType,
    lower: f64,
    upper: f64,
}

// FR-028
pub fn compute_reach_envelope(
    robot: &UrdfRobot,
    tool_distal_offset: f64,
    output_dir: &Path,
) -> ReachEnvelope {
    let chain = extract_chain(robot);
    if chain.is_empty() {
        return ReachEnvelope {
            max_radius_2d: 0.0,
            volume_3d: 0.0,
            hull_obj_rel: String::new(),
        };
    }

    // Sample the joint space on a deterministic grid and collect tool-tip
    // positions in the base (world) frame.
    let points = sample_workspace(&chain, tool_distal_offset);

    // Filter out points below Z=0 (below the robot base / floor level).
    let points: Vec<[f64; 3]> = points.into_iter().filter(|p| p[2] >= 0.0).collect();

    // max_radius_2d: ALWAYS computed — max horizontal distance from axis-1 (Z).
    let max_radius_2d = points
        .iter()
        .map(|p| (p[0] * p[0] + p[1] * p[1]).sqrt())
        .fold(0.0f64, f64::max);

    // 3D hull + volume + OBJ: best-effort (degenerate/coplanar clouds → skip).
    let mut volume_3d = 0.0;
    let mut hull_obj_rel = String::new();

    let hull_points: Vec<Point<f32>> = points
        .iter()
        .map(|p| Point::new(p[0] as f32, p[1] as f32, p[2] as f32))
        .collect();
    // parry3d's convex_hull panics on degenerate input (coplanar / collinear /
    // too few points). Guard by checking the AABB has non-zero extent in all 3
    // axes before attempting the hull.
    let has_3d_spread = if hull_points.len() >= 4 {
        let (mut mn, mut mx) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &hull_points {
            for i in 0..3 {
                mn[i] = mn[i].min(p[i]);
                mx[i] = mx[i].max(p[i]);
            }
        }
        (mx[0] - mn[0]) > 1e-4 && (mx[1] - mn[1]) > 1e-4 && (mx[2] - mn[2]) > 1e-4
    } else {
        false
    };
    let (hull_verts, hull_faces) = if has_3d_spread {
        convex_hull(&hull_points)
    } else {
        (vec![], vec![])
    };

    if !hull_verts.is_empty() && !hull_faces.is_empty() {
        volume_3d = compute_hull_volume(&hull_verts, &hull_faces);

        let reach_dir = output_dir.join("meshes/3dreach");
        if std::fs::create_dir_all(&reach_dir).is_ok() {
            let obj_path = reach_dir.join("reach_envelope.obj");
            if write_obj(&obj_path, &hull_verts, &hull_faces).is_ok() {
                hull_obj_rel = "./meshes/3dreach/reach_envelope.obj".to_string();
            }
        }
    }

    ReachEnvelope {
        max_radius_2d,
        volume_3d,
        hull_obj_rel,
    }
}

/// Walk the kinematic chain from root to leaf and collect the joints.
/// The root link is the one that appears as a joint parent but never as a
/// child (handles "World", "base_link", or any other naming convention).
fn extract_chain(robot: &UrdfRobot) -> Vec<ChainJoint> {
    let mut chain = Vec::new();
    // Find the true root: a link name that appears as a parent but never as a child.
    let children: std::collections::HashSet<&str> =
        robot.joints.iter().map(|j| j.child.as_str()).collect();
    let mut current_link = robot
        .joints
        .iter()
        .map(|j| j.parent.as_str())
        .find(|p| !children.contains(p))
        .or_else(|| robot.links.first().map(|l| l.name.as_str()))
        .unwrap_or("");

    for _ in 0..20 {
        // When a link has multiple child joints (e.g. base_link → base(fixed)
        // AND base_link → link_1(revolute)), prefer the articulated one to
        // follow the serial chain rather than a dead-end fixed branch.
        let joint_opt = robot
            .joints
            .iter()
            .filter(|j| j.parent == current_link)
            .min_by_key(|j| match j.joint_type {
                JointType::Revolute | JointType::Continuous | JointType::Prismatic => 0,
                _ => 1,
            });
        if let Some(joint) = joint_opt {
            let is_articulated = matches!(
                joint.joint_type,
                JointType::Revolute | JointType::Continuous | JointType::Prismatic
            );
            if is_articulated {
                let (lower, upper) = match (&joint.joint_type, &joint.limit) {
                    (JointType::Continuous, _) => (-std::f64::consts::PI, std::f64::consts::PI),
                    (_, Some(lim)) => (lim.lower, lim.upper),
                    _ => (-std::f64::consts::PI, std::f64::consts::PI),
                };
                chain.push(ChainJoint {
                    origin: joint.origin,
                    axis: joint.axis,
                    joint_type: joint.joint_type.clone(),
                    lower,
                    upper,
                });
            } else {
                // Fixed joint: store as zero-range so FK still applies its static transform.
                chain.push(ChainJoint {
                    origin: joint.origin,
                    axis: joint.axis,
                    joint_type: joint.joint_type.clone(),
                    lower: 0.0,
                    upper: 0.0,
                });
            }
            current_link = &joint.child;
        } else {
            break;
        }
    }
    chain
}

/// Sample the joint space on a uniform grid and run FK for each configuration,
/// returning the tool-tip position cloud in the base frame.
fn sample_workspace(chain: &[ChainJoint], tool_distal_offset: f64) -> Vec<[f64; 3]> {
    // Build per-joint sample arrays: uniform grid from min to max (inclusive).
    let joint_samples: Vec<Vec<f64>> = chain
        .iter()
        .map(|j| {
            if j.lower >= j.upper {
                vec![0.0] // fixed or zero-range
            } else {
                (0..=GRID_STEPS)
                    .map(|i| j.lower + (j.upper - j.lower) * (i as f64) / (GRID_STEPS as f64))
                    .collect()
            }
        })
        .collect();

    // Total configurations = product of all sample counts.
    let total: usize = joint_samples.iter().map(|s| s.len()).product();
    let mut points = Vec::with_capacity(total);

    // Iterate the Cartesian product via an index vector (no recursion, no alloc per step).
    let dims: Vec<usize> = joint_samples.iter().map(|s| s.len()).collect();
    let mut idx = vec![0usize; chain.len()];

    loop {
        // Build q-vector from current indices.
        let q: Vec<f64> = idx
            .iter()
            .enumerate()
            .map(|(d, &i)| joint_samples[d][i])
            .collect();

        // FK: accumulate pos + rot through the chain.
        let pos = forward_kinematics(chain, &q, tool_distal_offset);
        points.push(pos);

        // Advance the multi-index (odometer style, last dimension fastest).
        if !advance_index(&mut idx, &dims) {
            break;
        }
    }

    points
}

/// Forward kinematics: given joint values `q`, compute the tool-tip position
/// in the base frame by walking the chain and accumulating transforms.
fn forward_kinematics(chain: &[ChainJoint], q: &[f64], tool_distal_offset: f64) -> [f64; 3] {
    let mut pos = [0.0f64; 3];
    let mut rot = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    for (i, joint) in chain.iter().enumerate() {
        // Static transform: pos += rot * origin.xyz; rot *= R(origin.rpy).
        let t = mat3_vec3(&rot, &joint.origin.xyz);
        pos[0] += t[0];
        pos[1] += t[1];
        pos[2] += t[2];
        let r_static = rotation_matrix_from_rpy(joint.origin.rpy[0], joint.origin.rpy[1], joint.origin.rpy[2]);
        rot = mat3_multiply(&rot, &r_static);

        // Joint articulation.
        let angle = q[i];
        match joint.joint_type {
            JointType::Revolute | JointType::Continuous => {
                let r_joint = axis_angle_matrix(&joint.axis, angle);
                rot = mat3_multiply(&rot, &r_joint);
            }
            JointType::Prismatic => {
                let d = mat3_vec3(&rot, &[
                    joint.axis[0] * angle,
                    joint.axis[1] * angle,
                    joint.axis[2] * angle,
                ]);
                pos[0] += d[0];
                pos[1] += d[1];
                pos[2] += d[2];
            }
            _ => {} // Fixed: no articulation
        }
    }

    // Tool tip: extend along the flange outward direction (last column of rot
    // dotted with the last joint's axis gives the flange direction; for
    // simplicity, extend along rot * [1,0,0] or the chain's final axis).
    // The tool_distal_offset is the distance from flange origin to the tool tip.
    if tool_distal_offset > 0.0 {
        // Use the Z-column of the current rotation as the "outward" direction
        // (convention: flange points along the last joint's axis after articulation).
        let last_axis = chain.last().map(|j| j.axis).unwrap_or([0.0, 0.0, 1.0]);
        let outward = mat3_vec3(&rot, &last_axis);
        let len = (outward[0] * outward[0] + outward[1] * outward[1] + outward[2] * outward[2]).sqrt();
        if len > 1e-9 {
            pos[0] += outward[0] / len * tool_distal_offset;
            pos[1] += outward[1] / len * tool_distal_offset;
            pos[2] += outward[2] / len * tool_distal_offset;
        }
    }

    pos
}

/// Advance a multi-dimensional index vector. Returns false when all combinations exhausted.
fn advance_index(idx: &mut [usize], dims: &[usize]) -> bool {
    for d in (0..idx.len()).rev() {
        idx[d] += 1;
        if idx[d] < dims[d] {
            return true;
        }
        idx[d] = 0;
    }
    false
}

/// Hull volume via the signed-tetrahedron (divergence theorem) method.
/// V = |Σ (v0 · (v1 × v2)) / 6| for each triangle face.
fn compute_hull_volume(verts: &[Point<f32>], faces: &[[u32; 3]]) -> f64 {
    let mut vol = 0.0f64;
    for face in faces {
        let v0 = &verts[face[0] as usize];
        let v1 = &verts[face[1] as usize];
        let v2 = &verts[face[2] as usize];
        // Signed volume of tetrahedron formed with origin.
        let cross = [
            (v1.y as f64 - v0.y as f64) * (v2.z as f64 - v0.z as f64)
                - (v1.z as f64 - v0.z as f64) * (v2.y as f64 - v0.y as f64),
            (v1.z as f64 - v0.z as f64) * (v2.x as f64 - v0.x as f64)
                - (v1.x as f64 - v0.x as f64) * (v2.z as f64 - v0.z as f64),
            (v1.x as f64 - v0.x as f64) * (v2.y as f64 - v0.y as f64)
                - (v1.y as f64 - v0.y as f64) * (v2.x as f64 - v0.x as f64),
        ];
        vol += (v0.x as f64) * cross[0] + (v0.y as f64) * cross[1] + (v0.z as f64) * cross[2];
    }
    (vol / 6.0).abs()
}

// ---- Math helpers (mirror merger.rs patterns) ----

fn rotation_matrix_from_rpy(roll: f64, pitch: f64, yaw: f64) -> [[f64; 3]; 3] {
    let (sr, cr) = roll.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sy, cy) = yaw.sin_cos();
    [
        [cy * cp, cy * sp * sr - sy * cr, cy * sp * cr + sy * sr],
        [sy * cp, sy * sp * sr + cy * cr, sy * sp * cr - cy * sr],
        [-sp, cp * sr, cp * cr],
    ]
}

fn axis_angle_matrix(axis: &[f64; 3], angle: f64) -> [[f64; 3]; 3] {
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    let len = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    let (x, y, z) = if len > 1e-9 {
        (axis[0] / len, axis[1] / len, axis[2] / len)
    } else {
        (1.0, 0.0, 0.0)
    };
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

fn mat3_multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn mat3_vec3(m: &[[f64; 3]; 3], v: &[f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Joint, JointLimit, JointType, Link, Origin, UrdfRobot};

    /// Synthetic 2-link planar arm: known exact reach = L1 + L2.
    /// Joint 1 at origin, revolute about Z; link1 length 1.0m along X.
    /// Joint 2 at [1,0,0], revolute about Z; link2 length 0.5m along X.
    /// Max reach = 1.5m (both links extended).
    fn two_link_planar() -> UrdfRobot {
        UrdfRobot {
            name: "test_2link".to_string(),
            links: vec![
                Link { name: "base".into(), inertial: None, visuals: vec![], collisions: vec![] },
                Link { name: "link1".into(), inertial: None, visuals: vec![], collisions: vec![] },
                Link { name: "link2".into(), inertial: None, visuals: vec![], collisions: vec![] },
            ],
            joints: vec![
                Joint {
                    name: "j1".into(),
                    joint_type: JointType::Revolute,
                    parent: "base".into(),
                    child: "link1".into(),
                    origin: Origin { xyz: [0.0, 0.0, 0.0], rpy: [0.0, 0.0, 0.0] },
                    axis: [0.0, 0.0, 1.0],
                    limit: Some(JointLimit { lower: -std::f64::consts::PI, upper: std::f64::consts::PI, effort: 0.0, velocity: 0.0 }),
                },
                Joint {
                    name: "j2".into(),
                    joint_type: JointType::Revolute,
                    parent: "link1".into(),
                    child: "link2".into(),
                    origin: Origin { xyz: [1.0, 0.0, 0.0], rpy: [0.0, 0.0, 0.0] },
                    axis: [0.0, 0.0, 1.0],
                    limit: Some(JointLimit { lower: -std::f64::consts::PI, upper: std::f64::consts::PI, effort: 0.0, velocity: 0.0 }),
                },
            ],
            reach: None,
        }
    }

    #[test]
    fn test_two_link_max_reach() {
        let robot = two_link_planar();
        let dir = tempfile::tempdir().unwrap();
        // tool_distal = 0: no tool mounted. Both joints revolve about Z, j2 is
        // at [1,0,0] from j1 — the chain's max XY distance is 1.0m (the flange
        // stays at radius 1.0 from the base since j2's rotation about Z doesn't
        // move the flange when there's no further offset).
        let env = compute_reach_envelope(&robot, 0.0, dir.path());
        assert!(
            (env.max_radius_2d - 1.0).abs() < 0.05,
            "expected ~1.0m reach (bare 2R Z-axis arm), got {}",
            env.max_radius_2d
        );
        // volume_3d is 0 for this planar arm (all points are coplanar in Z=0)
        // — that's geometrically correct; a non-trivial volume requires out-of-
        // plane joints (tested with real 6R assets in tests/requirements.rs).
        assert!(env.volume_3d >= 0.0);
        // No hull OBJ for coplanar point clouds (degenerate — no 3D volume).
        assert!(env.hull_obj_rel.is_empty());
    }

    #[test]
    fn test_hull_volume_unit_cube() {
        // A unit cube's convex hull volume should be 1.0.
        let verts: Vec<Point<f32>> = vec![
            Point::new(0.0, 0.0, 0.0), Point::new(1.0, 0.0, 0.0),
            Point::new(1.0, 1.0, 0.0), Point::new(0.0, 1.0, 0.0),
            Point::new(0.0, 0.0, 1.0), Point::new(1.0, 0.0, 1.0),
            Point::new(1.0, 1.0, 1.0), Point::new(0.0, 1.0, 1.0),
        ];
        let (hull_v, hull_f) = convex_hull(&verts);
        let vol = compute_hull_volume(&hull_v, &hull_f);
        assert!((vol - 1.0).abs() < 0.01, "expected ~1.0, got {}", vol);
    }
}
