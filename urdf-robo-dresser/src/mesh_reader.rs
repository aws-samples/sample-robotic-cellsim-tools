use std::path::Path;

use crate::error::DresserError;

/// Read vertex positions from an OBJ file.
/// Only parses `v x y z` lines — ignores normals, texture coords, faces.
pub fn read_obj_vertices(path: &Path) -> Result<Vec<[f64; 3]>, DresserError> {
    let content = std::fs::read_to_string(path).map_err(|e| DresserError::UrdfParse {
        path: path.display().to_string(),
        detail: format!("failed to read OBJ file: {}", e),
    })?;

    let mut vertices = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with("v ") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            let x = parts[1].parse::<f64>().unwrap_or(0.0);
            let y = parts[2].parse::<f64>().unwrap_or(0.0);
            let z = parts[3].parse::<f64>().unwrap_or(0.0);
            vertices.push([x, y, z]);
        }
    }

    Ok(vertices)
}

/// Read vertex positions from a binary STL file.
pub fn read_stl_vertices(path: &Path) -> Result<Vec<[f64; 3]>, DresserError> {
    let data = std::fs::read(path).map_err(|e| DresserError::UrdfParse {
        path: path.display().to_string(),
        detail: format!("failed to read STL file: {}", e),
    })?;

    if data.len() < 84 {
        return Ok(Vec::new());
    }

    // Binary STL: 80-byte header + 4-byte triangle count + triangles
    let num_triangles = u32::from_le_bytes([data[80], data[81], data[82], data[83]]) as usize;
    let mut vertices = Vec::with_capacity(num_triangles * 3);

    let mut offset = 84;
    for _ in 0..num_triangles {
        if offset + 50 > data.len() {
            break;
        }
        // Skip normal (12 bytes), read 3 vertices (each 12 bytes), skip attribute (2 bytes)
        offset += 12; // skip normal
        for _ in 0..3 {
            let x = f32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]) as f64;
            let y = f32::from_le_bytes([data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7]]) as f64;
            let z = f32::from_le_bytes([data[offset + 8], data[offset + 9], data[offset + 10], data[offset + 11]]) as f64;
            vertices.push([x, y, z]);
            offset += 12;
        }
        offset += 2; // skip attribute byte count
    }

    Ok(vertices)
}

/// Read vertices from a mesh file (auto-detect OBJ vs STL by extension).
pub fn read_mesh_vertices(path: &Path) -> Result<Vec<[f64; 3]>, DresserError> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("obj") => read_obj_vertices(path),
        Some("stl") | Some("STL") => read_stl_vertices(path),
        _ => read_obj_vertices(path), // fallback to OBJ
    }
}

/// Full OBJ mesh data: vertices, face indices, and computed face normals.
pub struct ObjMesh {
    pub vertices: Vec<[f64; 3]>,
    pub faces: Vec<[usize; 3]>,
    pub face_normals: Vec<[f64; 3]>,
}

/// Read full OBJ mesh: vertices + faces + computed face normals.
pub fn read_obj_mesh(path: &Path) -> Result<ObjMesh, DresserError> {
    let content = std::fs::read_to_string(path).map_err(|e| DresserError::UrdfParse {
        path: path.display().to_string(),
        detail: format!("failed to read OBJ file: {}", e),
    })?;

    let mut vertices = Vec::new();
    let mut faces = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("v ") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 4 {
                let x = parts[1].parse::<f64>().unwrap_or(0.0);
                let y = parts[2].parse::<f64>().unwrap_or(0.0);
                let z = parts[3].parse::<f64>().unwrap_or(0.0);
                vertices.push([x, y, z]);
            }
        } else if line.starts_with("f ") {
            // Parse face: "f v1 v2 v3" or "f v1/vt1/vn1 v2/... v3/..."
            let parts: Vec<&str> = line.split_whitespace().skip(1).collect();
            if parts.len() >= 3 {
                let indices: Vec<usize> = parts.iter()
                    .filter_map(|p| {
                        // Take first number before any '/'
                        p.split('/').next()?.parse::<usize>().ok().map(|i| i.saturating_sub(1))
                    })
                    .collect();
                // Triangulate: fan from first vertex
                for i in 1..indices.len() - 1 {
                    faces.push([indices[0], indices[i], indices[i + 1]]);
                }
            }
        }
    }

    // Compute face normals from cross product
    let face_normals: Vec<[f64; 3]> = faces.iter().map(|f| {
        if f[0] < vertices.len() && f[1] < vertices.len() && f[2] < vertices.len() {
            let v0 = vertices[f[0]];
            let v1 = vertices[f[1]];
            let v2 = vertices[f[2]];
            let e1 = [v1[0] - v0[0], v1[1] - v0[1], v1[2] - v0[2]];
            let e2 = [v2[0] - v0[0], v2[1] - v0[1], v2[2] - v0[2]];
            let nx = e1[1] * e2[2] - e1[2] * e2[1];
            let ny = e1[2] * e2[0] - e1[0] * e2[2];
            let nz = e1[0] * e2[1] - e1[1] * e2[0];
            let len = (nx * nx + ny * ny + nz * nz).sqrt();
            if len > 1e-12 { [nx / len, ny / len, nz / len] } else { [0.0; 3] }
        } else {
            [0.0; 3]
        }
    }).collect();

    Ok(ObjMesh { vertices, faces, face_normals })
}

/// Compute the average normal of faces whose centroid is within `radius` of a given point.
pub fn average_normal_near_point(mesh: &ObjMesh, point: [f64; 3], radius: f64) -> [f64; 3] {
    let radius_sq = radius * radius;
    let mut sum = [0.0f64; 3];
    let mut count = 0usize;

    for (i, face) in mesh.faces.iter().enumerate() {
        if face[0] >= mesh.vertices.len() || face[1] >= mesh.vertices.len() || face[2] >= mesh.vertices.len() {
            continue;
        }
        let v0 = mesh.vertices[face[0]];
        let v1 = mesh.vertices[face[1]];
        let v2 = mesh.vertices[face[2]];
        let cx = (v0[0] + v1[0] + v2[0]) / 3.0;
        let cy = (v0[1] + v1[1] + v2[1]) / 3.0;
        let cz = (v0[2] + v1[2] + v2[2]) / 3.0;

        let dx = cx - point[0];
        let dy = cy - point[1];
        let dz = cz - point[2];
        let dist_sq = dx * dx + dy * dy + dz * dz;

        if dist_sq <= radius_sq {
            sum[0] += mesh.face_normals[i][0];
            sum[1] += mesh.face_normals[i][1];
            sum[2] += mesh.face_normals[i][2];
            count += 1;
        }
    }

    if count == 0 {
        return [0.0; 3];
    }

    let len = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt();
    if len > 1e-12 {
        [sum[0] / len, sum[1] / len, sum[2] / len]
    } else {
        [0.0; 3]
    }
}

/// Compute the axis-aligned bounding box of a set of vertices.
/// Returns (min, max) corners.
pub fn compute_aabb(vertices: &[[f64; 3]]) -> ([f64; 3], [f64; 3]) {
    if vertices.is_empty() {
        return ([0.0; 3], [0.0; 3]);
    }
    let mut min = [f64::MAX; 3];
    let mut max = [f64::MIN; 3];
    for v in vertices {
        for i in 0..3 {
            min[i] = min[i].min(v[i]);
            max[i] = max[i].max(v[i]);
        }
    }
    (min, max)
}

/// Project all vertices onto an axis and return the (min, max) extent.
pub fn project_onto_axis(vertices: &[[f64; 3]], axis: [f64; 3]) -> (f64, f64) {
    if vertices.is_empty() {
        return (0.0, 0.0);
    }
    let mut min_proj = f64::MAX;
    let mut max_proj = f64::MIN;
    for v in vertices {
        let proj = v[0] * axis[0] + v[1] * axis[1] + v[2] * axis[2];
        min_proj = min_proj.min(proj);
        max_proj = max_proj.max(proj);
    }
    (min_proj, max_proj)
}

/// Compute the centroid of a set of vertices.
pub fn compute_centroid(vertices: &[[f64; 3]]) -> [f64; 3] {
    if vertices.is_empty() {
        return [0.0; 3];
    }
    let mut sum = [0.0f64; 3];
    for v in vertices {
        sum[0] += v[0];
        sum[1] += v[1];
        sum[2] += v[2];
    }
    let n = vertices.len() as f64;
    [sum[0] / n, sum[1] / n, sum[2] / n]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_obj_robot_mesh() {
        let mesh_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../assets/urdf-robot-collection/meshes");
        if !mesh_dir.exists() {
            return;
        }
        // Find any .obj file in the mesh directory
        let entry = std::fs::read_dir(&mesh_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().extension().map(|x| x == "obj").unwrap_or(false));
        if let Some(entry) = entry {
            let verts = read_obj_vertices(&entry.path()).unwrap();
            assert!(!verts.is_empty(), "Should have vertices");
        }
    }

    #[test]
    fn test_project_onto_axis() {
        let verts = vec![[1.0, 0.0, 0.0], [2.0, 0.0, 0.0], [3.0, 0.0, 0.0]];
        let (min, max) = project_onto_axis(&verts, [1.0, 0.0, 0.0]);
        assert!((min - 1.0).abs() < 1e-10);
        assert!((max - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_compute_aabb() {
        let verts = vec![[-1.0, -2.0, -3.0], [1.0, 2.0, 3.0], [0.0, 0.0, 0.0]];
        let (min, max) = compute_aabb(&verts);
        assert_eq!(min, [-1.0, -2.0, -3.0]);
        assert_eq!(max, [1.0, 2.0, 3.0]);
    }
}
