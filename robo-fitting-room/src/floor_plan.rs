use std::path::Path;

use crate::error::FittingRoomError;

#[derive(Debug, Clone)]
pub struct FloorPlan {
    pub segments: Vec<[[f64; 3]; 2]>,
    pub footprint_segments: Vec<[[f64; 3]; 2]>,
}

// FR-105
pub fn load_floor_plan(
    obj_path: &Path,
    offset: [f64; 3],
    scale: f64,
) -> Result<FloorPlan, FittingRoomError> {
    let content = std::fs::read_to_string(obj_path).map_err(|e| {
        FittingRoomError::FloorPlanNotFound(format!("{}: {}", obj_path.display(), e))
    })?;

    let (segments, footprint_segments) = parse_obj_grouped(&content, offset, scale);

    Ok(FloorPlan {
        segments,
        footprint_segments,
    })
}

fn parse_obj_grouped(
    content: &str,
    offset: [f64; 3],
    scale: f64,
) -> (Vec<[[f64; 3]; 2]>, Vec<[[f64; 3]; 2]>) {
    let mut vertices: Vec<[f64; 3]> = Vec::new();
    let mut floor_segments: Vec<[[f64; 3]; 2]> = Vec::new();
    let mut footprint_segments: Vec<[[f64; 3]; 2]> = Vec::new();
    let mut in_footprint_group = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // DXF→OBJ exports often encode layer info as `# Layer: <name>`
        // comments (no `o` group directives). Treat those identically.
        if let Some(rest) = trimmed.strip_prefix("# Layer:") {
            let layer_name = rest.trim().to_lowercase();
            in_footprint_group =
                layer_name.contains("e9_121051") || layer_name.contains("footprint");
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        match parts[0] {
            "o" => {
                let group_name = if parts.len() > 1 { parts[1] } else { "" };
                let name_lower = group_name.to_lowercase();
                in_footprint_group =
                    name_lower.contains("e9_121051") || name_lower.contains("footprint");
            }
            "v" => {
                if parts.len() < 4 {
                    continue;
                }
                let x = match parts[1].parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let y = match parts[2].parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let z = match parts[3].parse::<f64>() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                vertices.push([
                    x * scale + offset[0],
                    y * scale + offset[1],
                    z * scale + offset[2],
                ]);
            }
            "l" => {
                if parts.len() < 3 {
                    continue;
                }
                let idx1 = match parts[1].parse::<usize>() {
                    Ok(0) => continue,
                    Ok(v) => v - 1,
                    Err(_) => continue,
                };
                let idx2 = match parts[2].parse::<usize>() {
                    Ok(0) => continue,
                    Ok(v) => v - 1,
                    Err(_) => continue,
                };
                if idx1 >= vertices.len() || idx2 >= vertices.len() {
                    continue;
                }
                if in_footprint_group {
                    footprint_segments.push([vertices[idx1], vertices[idx2]]);
                } else {
                    floor_segments.push([vertices[idx1], vertices[idx2]]);
                }
            }
            _ => {}
        }
    }

    (floor_segments, footprint_segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_segments() {
        let content = "v 1.0 2.0 0.0\nv 4.0 5.0 0.0\nl 1 2\n";
        let (floor, footprints) = parse_obj_grouped(content, [0.0; 3], 1.0);
        assert_eq!(floor.len(), 1);
        assert_eq!(floor[0][0], [1.0, 2.0, 0.0]);
        assert_eq!(floor[0][1], [4.0, 5.0, 0.0]);
        assert!(footprints.is_empty());
    }

    #[test]
    fn parse_with_scale_and_offset() {
        let content = "v 1.0 2.0 0.0\nv 3.0 4.0 0.0\nl 1 2\n";
        let (floor, _) = parse_obj_grouped(content, [10.0, 20.0, 0.0], 2.0);
        assert_eq!(floor[0][0], [12.0, 24.0, 0.0]);
        assert_eq!(floor[0][1], [16.0, 28.0, 0.0]);
    }

    #[test]
    fn parse_footprint_layer_comment() {
        let content = "\
# Layer: 0
v 0.0 0.0 0.0
v 1.0 0.0 0.0
l 1 2
# Layer: E9_121051
v 2.0 0.0 0.0
v 3.0 0.0 0.0
l 3 4
";
        let (floor, footprints) = parse_obj_grouped(content, [0.0; 3], 1.0);
        assert_eq!(floor.len(), 1);
        assert_eq!(footprints.len(), 1);
        assert_eq!(footprints[0][0], [2.0, 0.0, 0.0]);
    }

    #[test]
    fn parse_footprint_group() {
        let content = "\
v 0.0 0.0 0.0
v 1.0 0.0 0.0
l 1 2
o E9_121051_footprint
v 2.0 0.0 0.0
v 3.0 0.0 0.0
l 3 4
";
        let (floor, footprints) = parse_obj_grouped(content, [0.0; 3], 1.0);
        assert_eq!(floor.len(), 1);
        assert_eq!(footprints.len(), 1);
        assert_eq!(footprints[0][0], [2.0, 0.0, 0.0]);
    }

    #[test]
    fn skips_comments_and_malformed() {
        let content = "# comment\nv 1.0 2.0\nv 1.0 2.0 3.0\nv 4.0 5.0 6.0\nl 1 2\n";
        let (floor, _) = parse_obj_grouped(content, [0.0; 3], 1.0);
        assert_eq!(floor.len(), 1);
    }

    #[test]
    fn skips_out_of_bounds_indices() {
        let content = "v 1.0 2.0 0.0\nv 3.0 4.0 0.0\nl 1 5\n";
        let (floor, _) = parse_obj_grouped(content, [0.0; 3], 1.0);
        assert!(floor.is_empty());
    }

    #[test]
    fn empty_input() {
        let (floor, footprints) = parse_obj_grouped("", [0.0; 3], 1.0);
        assert!(floor.is_empty());
        assert!(footprints.is_empty());
    }
}
