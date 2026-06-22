use std::path::Path;

use crate::error::DresserError;
use crate::mesh_reader;
use crate::types::{Collision, Geometry, Joint, Link, MergeOptions, Origin, UrdfRobot, Visual};

/// Merge a tool URDF into a base robot URDF.
///
// FR-020, FR-020a, FR-021a, FR-021b, FR-023, FR-025, FR-026, FR-027
pub fn merge(
    base: &UrdfRobot,
    tool: &UrdfRobot,
    opts: &MergeOptions,
) -> Result<UrdfRobot, DresserError> {
    validate_orient(opts.orient)?;

    let flange_link_name = resolve_flange(base, opts.attach_to.as_deref())?;

    // Find the joint that drives the flange link (to get its axis)
    let flange_joint = base.joints.iter().find(|j| j.child == flange_link_name);
    let flange_axis = flange_joint.map(|j| j.axis).unwrap_or([1.0, 0.0, 0.0]);

    // Compute flange face offset: the outermost point of the flange link's mesh along the joint axis
    let flange_face_offset = compute_flange_face_offset(base, flange_link_name, flange_axis, &opts.robot_urdf_dir);

    // Auto-rotate: if orient is 0/0/0, find the rotation around the flange axis
    // that causes no intersection with the robot's bounding box
    let orient = if opts.orient == [0, 0, 0] {
        find_best_orientation(base, tool, flange_axis, flange_face_offset, &opts.robot_urdf_dir, &opts.tool_urdf_dir)
    } else {
        opts.orient
    };

    // Compute tool mounting transform: align tool to flange face
    let tool_transform = compute_tool_transform(tool, flange_axis, flange_face_offset, &opts.tool_urdf_dir, orient);

    // Collect all tool visuals and collisions, applying the computed transform
    let tool_geom = collect_tool_geometry_transformed(tool, &tool_transform);

    // Set up mesh output directories
    let meshes_dir = opts.output_dir.join("meshes");
    let robot_meshes_dir = meshes_dir.join("robot");
    let tool_meshes_dir = meshes_dir.join("tool");
    std::fs::create_dir_all(&robot_meshes_dir).map_err(DresserError::Io)?;
    std::fs::create_dir_all(&tool_meshes_dir).map_err(DresserError::Io)?;

    // Build output links — inject tool geometry into the flange link
    let mut links = base.links.clone();
    for link in &mut links {
        copy_and_rewrite_link_meshes(link, &opts.robot_urdf_dir, &robot_meshes_dir, "meshes/robot")?;

        if link.name == flange_link_name {
            // Inject tool geometry into the flange link
            let mut tool_vis = tool_geom.visuals.clone();
            let mut tool_col = tool_geom.collisions.clone();

            for v in &mut tool_vis {
                if let Geometry::Mesh { ref mut filename, .. } = v.geometry {
                    *filename = copy_mesh_file(filename, &opts.tool_urdf_dir, &tool_meshes_dir, "meshes/tool")?;
                }
            }
            for c in &mut tool_col {
                if let Geometry::Mesh { ref mut filename, .. } = c.geometry {
                    *filename = copy_mesh_file(filename, &opts.tool_urdf_dir, &tool_meshes_dir, "meshes/tool")?;
                }
            }

            link.visuals.extend(tool_vis);
            link.collisions.extend(tool_col);
        }
    }

    // Joints: unchanged from base robot
    let joints = base.joints.clone();

    // Derive composite name
    let base_name = if base.name.is_empty() { "robot" } else { &base.name };
    let tool_name = derive_tool_name(tool);
    let name = format!("{}__{}", base_name, tool_name);

    Ok(UrdfRobot {
        name,
        links,
        joints,
    })
}

// --- Flange Detection ---

fn resolve_flange<'a>(robot: &'a UrdfRobot, attach_to: Option<&str>) -> Result<&'a str, DresserError> {
    if let Some(name) = attach_to {
        if robot.links.iter().any(|l| l.name == name) {
            return Ok(robot.links.iter().find(|l| l.name == name).unwrap().name.as_str());
        }
        return Err(DresserError::UrdfParse {
            path: String::new(),
            detail: format!("attach-to link '{}' not found in robot", name),
        });
    }

    // Auto-detect: deepest leaf link in kinematic chain
    let children: std::collections::HashSet<&str> =
        robot.joints.iter().map(|j| j.child.as_str()).collect();
    let parents: std::collections::HashSet<&str> =
        robot.joints.iter().map(|j| j.parent.as_str()).collect();

    // Leaf = appears as child but never as parent
    let leaves: Vec<&str> = children
        .iter()
        .filter(|c| !parents.contains(*c))
        .copied()
        .collect();

    if leaves.is_empty() {
        return Err(DresserError::UrdfParse {
            path: String::new(),
            detail: "cannot detect flange: no leaf links found".to_string(),
        });
    }

    let mut best_leaf = leaves[0];
    let mut best_depth = 0;

    for leaf in &leaves {
        let depth = chain_depth(leaf, &robot.joints);
        if depth > best_depth || (depth == best_depth && *leaf < best_leaf) {
            best_depth = depth;
            best_leaf = leaf;
        }
    }

    Ok(best_leaf)
}

fn chain_depth(leaf: &str, joints: &[Joint]) -> usize {
    let mut current = leaf;
    let mut depth = 0;
    loop {
        if let Some(joint) = joints.iter().find(|j| j.child == current) {
            current = &joint.parent;
            depth += 1;
        } else {
            break;
        }
    }
    depth
}

// --- Flange Face Computation ---

/// Compute how far the flange link's mesh extends along the joint axis.
/// This gives us the offset from the flange link's origin to its outermost face.
fn compute_flange_face_offset(
    base: &UrdfRobot,
    flange_link_name: &str,
    flange_axis: [f64; 3],
    robot_urdf_dir: &Path,
) -> f64 {
    let flange_link = match base.links.iter().find(|l| l.name == flange_link_name) {
        Some(l) => l,
        None => return 0.0,
    };

    // Collect all vertices from the flange link's visual meshes
    let mut all_vertices = Vec::new();
    for visual in &flange_link.visuals {
        if let Geometry::Mesh { ref filename, .. } = visual.geometry {
            let clean = filename.trim_start_matches("./");
            let mesh_path = robot_urdf_dir.join(clean);
            if let Ok(verts) = mesh_reader::read_mesh_vertices(&mesh_path) {
                // Apply visual origin transform to vertices
                let transformed = apply_origin_to_vertices(&verts, &visual.origin);
                all_vertices.extend(transformed);
            }
        }
    }

    if all_vertices.is_empty() {
        return 0.0;
    }

    // Project onto joint axis — the max projection is the flange face distance
    let (_min, max) = mesh_reader::project_onto_axis(&all_vertices, flange_axis);
    max
}

/// Compute the transform needed to place the tool at the flange face.
///
/// The tool's origin (0,0,0) is its docking interface — that point gets placed
/// at the flange face. Rotation is applied around the tool's origin.
fn compute_tool_transform(
    _tool: &UrdfRobot,
    flange_axis: [f64; 3],
    flange_face_offset: f64,
    _tool_urdf_dir: &Path,
    orient: [u16; 3],
) -> Origin {
    let orient_rpy = orient_to_rpy(orient);

    // Place tool origin at the flange face (outermost point of end-effector along axis)
    Origin {
        xyz: [
            flange_axis[0] * flange_face_offset,
            flange_axis[1] * flange_face_offset,
            flange_axis[2] * flange_face_offset,
        ],
        rpy: orient_rpy,
    }
}

/// 3-stage auto-orientation: tries all 64 rotation candidates and picks the best
/// using normal direction, centroid direction, and AABB intersection checks.
fn find_best_orientation(
    base: &UrdfRobot,
    tool: &UrdfRobot,
    flange_axis: [f64; 3],
    flange_face_offset: f64,
    robot_urdf_dir: &Path,
    tool_urdf_dir: &Path,
) -> [u16; 3] {
    let robot_aabb = compute_robot_aabb(base, robot_urdf_dir);
    let flange_outward = compute_flange_outward(base);
    let tool_mounting_normal = compute_tool_mounting_normal(tool, tool_urdf_dir);

    // Pre-load all tool mesh vertices into RAM once
    let preloaded = PreloadedMeshes::load(tool, tool_urdf_dir);
    log::info!("Auto-orient: loaded {} mesh components into RAM", preloaded.entries.len());

    let flange_pos = [
        flange_axis[0] * flange_face_offset,
        flange_axis[1] * flange_face_offset,
        flange_axis[2] * flange_face_offset,
    ];

    let steps: [u16; 4] = [0, 90, 180, 270];

    struct Candidate {
        orient: [u16; 3],
        normal_dot: f64,
        centroid_outward: f64,
        intersections: usize,
    }

    let mut candidates: Vec<Candidate> = Vec::with_capacity(64);

    for &rx in &steps {
        for &ry in &steps {
            for &rz in &steps {
                let orient = [rx, ry, rz];
                let transform = compute_tool_transform(
                    tool, flange_axis, flange_face_offset, tool_urdf_dir, orient,
                );
                let rot = rotation_matrix_from_rpy(transform.rpy[0], transform.rpy[1], transform.rpy[2]);

                // Stage 1: Rotate the tool mounting normal and check alignment with flange outward
                let rotated_normal = [
                    rot[0][0] * tool_mounting_normal[0] + rot[0][1] * tool_mounting_normal[1] + rot[0][2] * tool_mounting_normal[2],
                    rot[1][0] * tool_mounting_normal[0] + rot[1][1] * tool_mounting_normal[1] + rot[1][2] * tool_mounting_normal[2],
                    rot[2][0] * tool_mounting_normal[0] + rot[2][1] * tool_mounting_normal[1] + rot[2][2] * tool_mounting_normal[2],
                ];
                let normal_dot = rotated_normal[0] * flange_outward[0]
                    + rotated_normal[1] * flange_outward[1]
                    + rotated_normal[2] * flange_outward[2];

                // Stage 2: Check if tool centroid extends outward (from preloaded data)
                let tool_centroid = preloaded.compute_centroid_transformed(&transform);
                let centroid_outward = dot3(&tool_centroid, &flange_outward) - dot3(&flange_pos, &flange_outward);

                // Stage 3: AABB intersection count (from preloaded data)
                let intersections = preloaded.count_intersections(&transform, &robot_aabb);

                candidates.push(Candidate { orient, normal_dot, centroid_outward, intersections });
            }
        }
    }

    // Scoring priority:
    // 1. Primary: highest normal_dot (tool direction aligns with flange outward)
    // 2. Tiebreak among top normal_dot candidates: prefer centroid_outward > 0
    // 3. Final tiebreak: lowest intersection count
    //
    // We pick the candidate with the best normal_dot. If multiple are within
    // 0.05 of the best, prefer the one with positive centroid_outward and
    // fewest intersections.
    candidates.sort_by(|a, b| b.normal_dot.partial_cmp(&a.normal_dot).unwrap());

    let best_dot = candidates[0].normal_dot;
    let top_candidates: Vec<&Candidate> = candidates.iter()
        .filter(|c| c.normal_dot >= best_dot - 0.05)
        .collect();

    let best = if top_candidates.len() > 1 {
        // Among near-equal normal_dot candidates, prefer outward centroid + low intersection
        top_candidates.iter()
            .filter(|c| c.centroid_outward > 0.0)
            .min_by_key(|c| c.intersections)
            .unwrap_or(&top_candidates[0])
    } else {
        &top_candidates[0]
    };

    log::info!(
        "Auto-orient: picked {:?} (normal_dot={:.3}, centroid_outward={:.3}, intersections={})",
        best.orient, best.normal_dot, best.centroid_outward, best.intersections
    );
    best.orient
}

/// Compute the outward direction of the flange at home pose (all joints = 0).
/// Walks the kinematic chain accumulating transforms (translation + rotation)
/// to determine which direction the last link actually points.
/// The flange outward = the last joint's axis rotated into world frame.
fn compute_flange_outward(base: &UrdfRobot) -> [f64; 3] {
    let mut rot = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut last_axis = [1.0, 0.0, 0.0f64];

    let mut current_link = base.links.iter()
        .find(|l| l.name == "World")
        .or_else(|| base.links.first())
        .map(|l| l.name.as_str())
        .unwrap_or("");

    for _ in 0..20 {
        if let Some(joint) = base.joints.iter().find(|j| j.parent == current_link) {
            let joint_rot = rotation_matrix_from_rpy(
                joint.origin.rpy[0], joint.origin.rpy[1], joint.origin.rpy[2],
            );
            rot = mat3_multiply(&rot, &joint_rot);
            last_axis = joint.axis;
            current_link = &joint.child;
        } else {
            break;
        }
    }

    // The flange outward direction is the last joint's axis transformed into world frame
    let dir = [
        rot[0][0] * last_axis[0] + rot[0][1] * last_axis[1] + rot[0][2] * last_axis[2],
        rot[1][0] * last_axis[0] + rot[1][1] * last_axis[1] + rot[1][2] * last_axis[2],
        rot[2][0] * last_axis[0] + rot[2][1] * last_axis[1] + rot[2][2] * last_axis[2],
    ];

    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if len > 1e-12 {
        [dir[0] / len, dir[1] / len, dir[2] / len]
    } else {
        [1.0, 0.0, 0.0]
    }
}

/// Compute the tool's principal direction: vector from origin (0,0,0) toward
/// the centroid of all tool geometry. This is the direction the tool "points".
///
/// Uses visual origin xyz positions directly (weighted equally per visual element).
/// This is more reliable than reading mesh vertices since all tools define visual
/// origins in the URDF regardless of mesh file availability.
fn compute_tool_mounting_normal(tool: &UrdfRobot, _tool_urdf_dir: &Path) -> [f64; 3] {
    let mut sum = [0.0f64; 3];
    let mut count = 0usize;

    for link in &tool.links {
        for visual in &link.visuals {
            // Use the visual origin xyz directly — this is where each mesh component is placed
            sum[0] += visual.origin.xyz[0];
            sum[1] += visual.origin.xyz[1];
            sum[2] += visual.origin.xyz[2];
            count += 1;
        }
    }

    if count == 0 {
        return [1.0, 0.0, 0.0];
    }

    // Direction from origin (0,0,0) to centroid of visual positions = tool's pointing direction
    let cx = sum[0] / count as f64;
    let cy = sum[1] / count as f64;
    let cz = sum[2] / count as f64;

    log::debug!("Tool direction vector (from origin to visual centroid): [{:.4}, {:.4}, {:.4}]", cx, cy, cz);

    let len = (cx * cx + cy * cy + cz * cz).sqrt();
    if len > 1e-12 {
        [cx / len, cy / len, cz / len]
    } else {
        [1.0, 0.0, 0.0]
    }
}

/// Pre-loaded mesh data for a tool — vertices loaded once, reused for all 64 orientation candidates.
struct PreloadedMeshes {
    /// Each entry: (visual origin, vertices)
    entries: Vec<(Origin, Vec<[f64; 3]>)>,
}

impl PreloadedMeshes {
    fn load(tool: &UrdfRobot, tool_urdf_dir: &Path) -> Self {
        let mut entries = Vec::new();
        for link in &tool.links {
            for visual in &link.visuals {
                if let Geometry::Mesh { ref filename, .. } = visual.geometry {
                    let clean = filename.trim_start_matches("./");
                    let mesh_path = tool_urdf_dir.join(clean);
                    if let Ok(verts) = mesh_reader::read_mesh_vertices(&mesh_path) {
                        if !verts.is_empty() {
                            entries.push((visual.origin, verts));
                        }
                    }
                }
            }
        }
        Self { entries }
    }

    fn compute_centroid_transformed(&self, transform: &Origin) -> [f64; 3] {
        let pivot = [0.0, 0.0, 0.0];
        let rot = rotation_matrix_from_rpy(transform.rpy[0], transform.rpy[1], transform.rpy[2]);
        let mut sum = [0.0f64; 3];
        let mut count = 0usize;

        for (origin, verts) in &self.entries {
            let final_origin = rotate_origin_around_pivot(origin, &pivot, &rot, transform.xyz);
            let transformed = apply_origin_to_vertices(verts, &final_origin);
            for v in &transformed {
                sum[0] += v[0];
                sum[1] += v[1];
                sum[2] += v[2];
                count += 1;
            }
        }

        if count == 0 { return [0.0; 3]; }
        [sum[0] / count as f64, sum[1] / count as f64, sum[2] / count as f64]
    }

    fn count_intersections(&self, transform: &Origin, robot_aabb: &([f64; 3], [f64; 3])) -> usize {
        let (aabb_min, aabb_max) = robot_aabb;
        let pivot = [0.0, 0.0, 0.0];
        let rot = rotation_matrix_from_rpy(transform.rpy[0], transform.rpy[1], transform.rpy[2]);
        let mut count = 0;

        for (origin, verts) in &self.entries {
            let final_origin = rotate_origin_around_pivot(origin, &pivot, &rot, transform.xyz);
            let transformed = apply_origin_to_vertices(verts, &final_origin);

            for v in &transformed {
                if v[0] >= aabb_min[0] && v[0] <= aabb_max[0]
                    && v[1] >= aabb_min[1] && v[1] <= aabb_max[1]
                    && v[2] >= aabb_min[2] && v[2] <= aabb_max[2]
                {
                    count += 1;
                }
            }
        }

        count
    }
}

fn dot3(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Compute AABB of the entire robot (all links' meshes combined).
fn compute_robot_aabb(base: &UrdfRobot, robot_urdf_dir: &Path) -> ([f64; 3], [f64; 3]) {
    let mut all_vertices = Vec::new();

    for link in &base.links {
        for visual in &link.visuals {
            if let Geometry::Mesh { ref filename, .. } = visual.geometry {
                let clean = filename.trim_start_matches("./");
                let mesh_path = robot_urdf_dir.join(clean);
                if let Ok(verts) = mesh_reader::read_mesh_vertices(&mesh_path) {
                    let transformed = apply_origin_to_vertices(&verts, &visual.origin);
                    all_vertices.extend(transformed);
                }
            }
        }
    }

    mesh_reader::compute_aabb(&all_vertices)
}

/// Apply an Origin transform (xyz translation + rpy rotation) to vertices.
fn apply_origin_to_vertices(vertices: &[[f64; 3]], origin: &Origin) -> Vec<[f64; 3]> {
    if origin.rpy == [0.0, 0.0, 0.0] && origin.xyz == [0.0, 0.0, 0.0] {
        return vertices.to_vec();
    }

    let rot = rotation_matrix_from_rpy(origin.rpy[0], origin.rpy[1], origin.rpy[2]);

    vertices
        .iter()
        .map(|v| {
            // Rotate then translate
            let rx = rot[0][0] * v[0] + rot[0][1] * v[1] + rot[0][2] * v[2];
            let ry = rot[1][0] * v[0] + rot[1][1] * v[1] + rot[1][2] * v[2];
            let rz = rot[2][0] * v[0] + rot[2][1] * v[1] + rot[2][2] * v[2];
            [rx + origin.xyz[0], ry + origin.xyz[1], rz + origin.xyz[2]]
        })
        .collect()
}

/// Build a 3x3 rotation matrix from roll-pitch-yaw (ZYX convention).
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

// --- Tool Geometry Collection ---

struct ToolGeometry {
    visuals: Vec<Visual>,
    collisions: Vec<Collision>,
}

/// Collect tool geometry and apply a rigid-body transform to all elements.
/// All visuals/collisions are rotated around the tool's origin (0,0,0) as a group,
/// then translated to the flange face position.
fn collect_tool_geometry_transformed(tool: &UrdfRobot, transform: &Origin) -> ToolGeometry {
    // Rotation pivot is the tool's origin (0,0,0) — the docking interface point
    let pivot = [0.0, 0.0, 0.0];
    let rot = rotation_matrix_from_rpy(transform.rpy[0], transform.rpy[1], transform.rpy[2]);

    let mut visuals = Vec::new();
    let mut collisions = Vec::new();

    for link in &tool.links {
        for v in &link.visuals {
            let mut visual = v.clone();
            visual.origin = rotate_origin_around_pivot(&visual.origin, &pivot, &rot, transform.xyz);
            visuals.push(visual);
        }
        for c in &link.collisions {
            let mut collision = c.clone();
            collision.origin = rotate_origin_around_pivot(&collision.origin, &pivot, &rot, transform.xyz);
            collisions.push(collision);
        }
    }

    ToolGeometry { visuals, collisions }
}


/// Rotate a single element's origin around a shared pivot point, then translate.
///
/// For a rigid-body group rotation of all elements:
/// - Each visual's mesh vertex v in world frame = R_element * v + t_element
/// - After group rotation R_group around pivot P with translation T:
///   new_world_v = R_group * (R_element * v + t_element - P) + P + T
///              = (R_group * R_element) * v + R_group * (t_element - P) + P + T
/// - So: new_rpy = rpy_from(R_group * R_element)
///        new_xyz = R_group * (t_element - P) + P + T
fn rotate_origin_around_pivot(
    element: &Origin,
    pivot: &[f64; 3],
    rot_group: &[[f64; 3]; 3],
    translation: [f64; 3],
) -> Origin {
    // Rotate the position around pivot
    let rel_x = element.xyz[0] - pivot[0];
    let rel_y = element.xyz[1] - pivot[1];
    let rel_z = element.xyz[2] - pivot[2];

    let rx = rot_group[0][0] * rel_x + rot_group[0][1] * rel_y + rot_group[0][2] * rel_z;
    let ry = rot_group[1][0] * rel_x + rot_group[1][1] * rel_y + rot_group[1][2] * rel_z;
    let rz = rot_group[2][0] * rel_x + rot_group[2][1] * rel_y + rot_group[2][2] * rel_z;

    let final_xyz = [
        rx + pivot[0] + translation[0],
        ry + pivot[1] + translation[1],
        rz + pivot[2] + translation[2],
    ];

    // Compose rotations via matrix multiplication: R_new = R_group * R_element
    let rot_element = rotation_matrix_from_rpy(element.rpy[0], element.rpy[1], element.rpy[2]);
    let rot_combined = mat3_multiply(rot_group, &rot_element);
    let final_rpy = rpy_from_rotation_matrix(&rot_combined);

    Origin {
        xyz: final_xyz,
        rpy: final_rpy,
    }
}

fn mat3_multiply(a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut c = [[0.0f64; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            c[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    c
}

fn rpy_from_rotation_matrix(rot: &[[f64; 3]; 3]) -> [f64; 3] {
    let pitch = (-rot[2][0]).clamp(-1.0, 1.0).asin();
    let roll;
    let yaw;

    if pitch.cos().abs() > 1e-6 {
        roll = rot[2][1].atan2(rot[2][2]);
        yaw = rot[1][0].atan2(rot[0][0]);
    } else {
        roll = 0.0;
        yaw = (-rot[0][1]).atan2(rot[1][1]);
    }

    [roll, pitch, yaw]
}

// --- Mesh Copying ---

fn copy_and_rewrite_link_meshes(
    link: &mut Link,
    source_dir: &Path,
    dest_dir: &Path,
    relative_prefix: &str,
) -> Result<(), DresserError> {
    for visual in &mut link.visuals {
        if let Geometry::Mesh { ref mut filename, .. } = visual.geometry {
            *filename = copy_mesh_file(filename, source_dir, dest_dir, relative_prefix)?;
        }
    }
    for collision in &mut link.collisions {
        if let Geometry::Mesh { ref mut filename, .. } = collision.geometry {
            *filename = copy_mesh_file(filename, source_dir, dest_dir, relative_prefix)?;
        }
    }
    Ok(())
}

fn copy_mesh_file(
    original_path: &str,
    source_dir: &Path,
    dest_dir: &Path,
    relative_prefix: &str,
) -> Result<String, DresserError> {
    let clean_path = original_path.trim_start_matches("./");
    let src = source_dir.join(clean_path);

    let file_name = src
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let dest = dest_dir.join(&file_name);

    if src.exists() && !dest.exists() {
        std::fs::copy(&src, &dest).map_err(DresserError::Io)?;
    }

    // Also copy the accompanying .mtl material file if it exists
    let mtl_src = src.with_extension("mtl");
    if mtl_src.exists() {
        let mtl_name = mtl_src.file_name().unwrap().to_string_lossy().to_string();
        let mtl_dest = dest_dir.join(&mtl_name);
        if !mtl_dest.exists() {
            let _ = std::fs::copy(&mtl_src, &mtl_dest);
        }
    }

    Ok(format!("./{}/{}", relative_prefix, file_name))
}

// --- Helpers ---

fn validate_orient(orient: [u16; 3]) -> Result<(), DresserError> {
    for (i, deg) in orient.iter().enumerate() {
        if *deg % 90 != 0 || *deg >= 360 {
            let axis = ["X", "Y", "Z"][i];
            return Err(DresserError::OrientationInvalid(format!(
                "{}-axis must be 0, 90, 180, or 270 — got {}",
                axis, deg
            )));
        }
    }
    Ok(())
}

fn orient_to_rpy(orient: [u16; 3]) -> [f64; 3] {
    orient.map(|deg| (deg as f64).to_radians())
}

fn derive_tool_name(tool: &UrdfRobot) -> String {
    if !tool.name.is_empty() {
        return tool.name.clone();
    }
    tool.links
        .iter()
        .find(|l| l.name != "World")
        .map(|l| l.name.clone())
        .unwrap_or_else(|| "tool".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_urdf;
    use std::path::Path;

    #[test]
    fn test_validate_orient() {
        assert!(validate_orient([0, 0, 0]).is_ok());
        assert!(validate_orient([90, 180, 270]).is_ok());
        assert!(validate_orient([45, 0, 0]).is_err());
        assert!(validate_orient([360, 0, 0]).is_err());
    }

    #[test]
    fn test_orient_to_rpy() {
        let rpy = orient_to_rpy([0, 90, 180]);
        assert!((rpy[0] - 0.0).abs() < 1e-10);
        assert!((rpy[1] - std::f64::consts::FRAC_PI_2).abs() < 1e-10);
        assert!((rpy[2] - std::f64::consts::PI).abs() < 1e-10);
    }

    fn find_first_urdf(dir: &Path) -> Option<std::path::PathBuf> {
        std::fs::read_dir(dir).ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().map(|x| x == "urdf").unwrap_or(false))
    }

    fn find_first_tool_urdf(dir: &Path) -> Option<std::path::PathBuf> {
        std::fs::read_dir(dir).ok()?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .find_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                let urdf = e.path().join(format!("{}.urdf", name));
                urdf.exists().then_some(urdf)
            })
    }

    #[test]
    fn test_flange_detection() {
        let robot_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-robot-collection");
        let urdf_path = match find_first_urdf(&robot_dir) {
            Some(p) => p,
            None => return,
        };
        let robot = parse_urdf(&urdf_path).unwrap();
        let flange = resolve_flange(&robot, None).unwrap();

        let depth = chain_depth(flange, &robot.joints);
        assert!(depth >= 5, "Flange should be deep in the kinematic chain");

        let is_parent = robot.joints.iter().any(|j| j.parent == flange);
        assert!(!is_parent, "Flange link should be a leaf (no children)");
    }

    #[test]
    fn test_merge_robot_with_tool() {
        let robot_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-robot-collection");
        let tool_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-tool-collection");
        let robot_path = match find_first_urdf(&robot_dir) {
            Some(p) => p,
            None => return,
        };
        let tool_path = match find_first_tool_urdf(&tool_dir) {
            Some(p) => p,
            None => return,
        };

        let base = parse_urdf(&robot_path).unwrap();
        let tool = parse_urdf(&tool_path).unwrap();
        let opts = MergeOptions::default();

        let dressed = merge(&base, &tool, &opts).unwrap();

        // Same number of links as the base robot (tool geometry injected, not added as new links)
        assert_eq!(dressed.links.len(), base.links.len());

        // Same joints as base robot (no extra joints)
        assert_eq!(dressed.joints.len(), base.joints.len());

        // Flange link should now have more visuals than before (tool injected)
        let flange_name = resolve_flange(&base, None).unwrap();
        let base_flange = base.links.iter().find(|l| l.name == flange_name).unwrap();
        let dressed_flange = dressed.links.iter().find(|l| l.name == flange_name).unwrap();
        assert!(
            dressed_flange.visuals.len() > base_flange.visuals.len(),
            "Flange link should have tool visuals injected"
        );

        // Composite name
        assert!(dressed.name.contains("__"));
    }
}
