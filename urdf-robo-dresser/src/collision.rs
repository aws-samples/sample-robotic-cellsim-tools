use std::io::Write;
use std::path::Path;

use parry3d::math::Point;
use parry3d::transformation::convex_hull;

use crate::error::DresserError;
use crate::mesh_reader;
use crate::types::{Collision, Geometry, UrdfRobot};

#[derive(Debug, Clone, Copy)]
pub enum HullBuffer {
    Tight,
    Medium,
    Max,
}

impl HullBuffer {
    pub fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "tight" => Ok(Self::Tight),
            "medium" => Ok(Self::Medium),
            "max" => Ok(Self::Max),
            _ => Err(format!("invalid hull buffer '{}' — use tight, medium, or max", s)),
        }
    }

    fn scale_factor(&self) -> f64 {
        match self {
            Self::Tight => 1.0,
            Self::Medium => 1.05,
            Self::Max => 1.15,
        }
    }
}

// FR-016: Per-component convex hull collision generation
pub fn generate_hulls(
    robot: &mut UrdfRobot,
    urdf_dir: &Path,
    output_dir: &Path,
    subdirectory: &str,
    buffer: HullBuffer,
) -> Result<(), DresserError> {
    let hull_dir = output_dir.join("meshes").join(subdirectory);
    std::fs::create_dir_all(&hull_dir).map_err(DresserError::Io)?;

    let scale = buffer.scale_factor();

    for link in &mut robot.links {
        if link.visuals.is_empty() {
            continue;
        }

        let mut hulls: Vec<Collision> = Vec::new();
        let mut generated = 0usize;

        for (i, visual) in link.visuals.iter().enumerate() {
            if let Geometry::Mesh { ref filename, .. } = visual.geometry {
                let clean = filename.trim_start_matches("./");
                let mesh_path = urdf_dir.join(clean);
                if let Ok(verts) = mesh_reader::read_mesh_vertices(&mesh_path) {
                    if verts.len() < 4 {
                        continue;
                    }

                    let centroid = mesh_reader::compute_centroid(&verts);
                    let points: Vec<Point<f32>> = verts.iter()
                        .map(|v| {
                            Point::new(
                                (centroid[0] + (v[0] - centroid[0]) * scale) as f32,
                                (centroid[1] + (v[1] - centroid[1]) * scale) as f32,
                                (centroid[2] + (v[2] - centroid[2]) * scale) as f32,
                            )
                        })
                        .collect();

                    let (hull_points, hull_indices) = convex_hull(&points);
                    if hull_points.is_empty() || hull_indices.is_empty() {
                        continue;
                    }

                    let obj_filename = format!("{}_hull_{}.obj", sanitize_filename(&link.name), i);
                    let obj_path = hull_dir.join(&obj_filename);
                    if let Err(e) = write_obj(&obj_path, &hull_points, &hull_indices) {
                        log::warn!("Failed to write hull {}: {}", obj_filename, e);
                        continue;
                    }

                    hulls.push(Collision {
                        origin: visual.origin,
                        geometry: Geometry::Mesh {
                            filename: format!("./{}", obj_filename),
                            scale: [1.0, 1.0, 1.0],
                        },
                    });
                    generated += 1;
                }
            }
        }

        if !hulls.is_empty() {
            link.collisions = hulls;
            log::info!("Generated {} collision hulls for link '{}' ({:?})", generated, link.name, buffer);
        }
    }

    Ok(())
}

fn write_obj(
    path: &Path,
    vertices: &[Point<f32>],
    triangles: &[[u32; 3]],
) -> Result<(), DresserError> {
    let mut file = std::fs::File::create(path).map_err(DresserError::Io)?;

    for v in vertices {
        writeln!(file, "v {} {} {}", v.x, v.y, v.z).map_err(DresserError::Io)?;
    }

    for tri in triangles {
        writeln!(file, "f {} {} {}", tri[0] + 1, tri[1] + 1, tri[2] + 1).map_err(DresserError::Io)?;
    }

    Ok(())
}

fn sanitize_filename(name: &str) -> String {
    name.replace(|c: char| !c.is_alphanumeric() && c != '_' && c != '-', "_")
}
