use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::FittingRoomError;

#[derive(Debug, Default, Clone)]
pub struct ObjMesh {
    pub points: Vec<[f64; 3]>,
    /// Flat list of triangle vertex indices (0-based).
    pub face_vertex_indices: Vec<u32>,
    /// `face_vertex_counts[i]` = number of vertices for face i (always 3 in our triangulated output).
    pub face_vertex_counts: Vec<u32>,
    /// MTL filename referenced by `mtllib` (relative to the OBJ file). None if absent.
    pub mtllib: Option<String>,
    /// Material name from the first `usemtl` directive. Our dressed OBJs use one material
    /// per file; if a file had multiple `usemtl` switches we'd capture only the first.
    pub usemtl: Option<String>,
}

/// Parse an OBJ file. Triangulates polygons. Tolerant of common malformations
/// (skips bad lines with a warning rather than aborting).
pub fn parse_obj(path: &Path) -> Result<ObjMesh, FittingRoomError> {
    let file = File::open(path).map_err(|e| {
        FittingRoomError::UsdWrite(format!("open {} for OBJ parse: {}", path.display(), e))
    })?;
    let reader = BufReader::new(file);

    let mut mesh = ObjMesh::default();

    for (lineno, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let tag = match parts.next() {
            Some(t) => t,
            None => continue,
        };
        match tag {
            "mtllib" => {
                if mesh.mtllib.is_none() {
                    if let Some(name) = parts.next() {
                        mesh.mtllib = Some(name.to_string());
                    }
                }
            }
            "usemtl" => {
                if mesh.usemtl.is_none() {
                    if let Some(name) = parts.next() {
                        mesh.usemtl = Some(name.to_string());
                    }
                }
            }
            "v" => {
                let coords: Vec<f64> = parts.filter_map(|p| p.parse::<f64>().ok()).collect();
                if coords.len() < 3 {
                    log::warn!("{}:{} bad vertex, skipping", path.display(), lineno + 1);
                    continue;
                }
                mesh.points.push([coords[0], coords[1], coords[2]]);
            }
            "f" => {
                // Each token may be `i`, `i/j`, `i/j/k`, or `i//k`. Take the first part (vertex index).
                let indices: Vec<i64> = parts
                    .filter_map(|p| {
                        let v = p.split('/').next()?;
                        v.parse::<i64>().ok()
                    })
                    .collect();
                if indices.len() < 3 {
                    continue;
                }
                let n = mesh.points.len() as i64;
                let resolve = |i: i64| -> Option<u32> {
                    let idx0 = if i > 0 {
                        i - 1
                    } else if i < 0 {
                        n + i
                    } else {
                        return None;
                    };
                    if idx0 < 0 || idx0 >= n {
                        None
                    } else {
                        Some(idx0 as u32)
                    }
                };
                // Fan-triangulate
                let v0 = match resolve(indices[0]) {
                    Some(v) => v,
                    None => continue,
                };
                for w in indices[1..].windows(2) {
                    if let (Some(b), Some(c)) = (resolve(w[0]), resolve(w[1])) {
                        mesh.face_vertex_indices.extend_from_slice(&[v0, b, c]);
                        mesh.face_vertex_counts.push(3);
                    }
                }
            }
            _ => {
                // Ignore vt, vn, mtllib, usemtl, o, g, s, etc.
            }
        }
    }

    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_triangle() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tri.obj");
        let mut f = File::create(&p).unwrap();
        writeln!(f, "v 0 0 0").unwrap();
        writeln!(f, "v 1 0 0").unwrap();
        writeln!(f, "v 0 1 0").unwrap();
        writeln!(f, "f 1 2 3").unwrap();
        let m = parse_obj(&p).unwrap();
        assert_eq!(m.points.len(), 3);
        assert_eq!(m.face_vertex_indices, vec![0, 1, 2]);
        assert_eq!(m.face_vertex_counts, vec![3]);
    }

    #[test]
    fn triangulates_quad() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("quad.obj");
        let mut f = File::create(&p).unwrap();
        for x in 0..4 {
            writeln!(f, "v {} 0 0", x).unwrap();
        }
        writeln!(f, "f 1 2 3 4").unwrap();
        let m = parse_obj(&p).unwrap();
        assert_eq!(m.face_vertex_counts, vec![3, 3]);
        assert_eq!(m.face_vertex_indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn handles_face_with_uv_and_normal() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("uvn.obj");
        let mut f = File::create(&p).unwrap();
        writeln!(f, "v 0 0 0").unwrap();
        writeln!(f, "v 1 0 0").unwrap();
        writeln!(f, "v 0 1 0").unwrap();
        writeln!(f, "f 1/1/1 2/2/2 3/3/3").unwrap();
        let m = parse_obj(&p).unwrap();
        assert_eq!(m.face_vertex_indices, vec![0, 1, 2]);
    }

    #[test]
    fn skips_short_vertex() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.obj");
        let mut f = File::create(&p).unwrap();
        writeln!(f, "v 1 2").unwrap();
        writeln!(f, "v 0 0 0").unwrap();
        writeln!(f, "v 1 0 0").unwrap();
        writeln!(f, "v 0 1 0").unwrap();
        writeln!(f, "f 1 2 3").unwrap();
        let m = parse_obj(&p).unwrap();
        assert_eq!(m.points.len(), 3);
    }
}
