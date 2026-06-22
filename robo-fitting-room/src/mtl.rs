use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::error::FittingRoomError;

#[derive(Debug, Clone, Default)]
pub struct Material {
    /// Diffuse colour (Kd). Maps to UsdPreviewSurface `diffuseColor`. Defaults to mid-grey.
    pub diffuse: [f64; 3],
    /// Specular colour (Ks). Used to estimate metallic/roughness via heuristics.
    pub specular: [f64; 3],
    /// Specular exponent (Ns). Higher = sharper highlight.
    pub shininess: f64,
    /// Opacity (`d`). 1.0 = fully opaque.
    pub opacity: f64,
}

impl Material {
    pub fn placeholder() -> Self {
        Self {
            diffuse: [0.7, 0.7, 0.7],
            specular: [0.5, 0.5, 0.5],
            shininess: 10.0,
            opacity: 1.0,
        }
    }

    /// Crude PBR estimate: stronger Ks + higher Ns → more metallic, less rough.
    /// Good enough for a viewport preview without proper PBR data in the source.
    pub fn metallic(&self) -> f64 {
        let ks_avg = (self.specular[0] + self.specular[1] + self.specular[2]) / 3.0;
        ks_avg.clamp(0.0, 1.0)
    }

    pub fn roughness(&self) -> f64 {
        // Ns ranges 0..1000+; map inversely so high Ns → low roughness.
        let r = (1.0 / (1.0 + self.shininess * 0.05)).clamp(0.05, 1.0);
        r
    }
}

pub fn parse_mtl(path: &Path) -> Result<HashMap<String, Material>, FittingRoomError> {
    let file = File::open(path).map_err(|e| {
        FittingRoomError::UsdWrite(format!("open {} for MTL parse: {}", path.display(), e))
    })?;
    let reader = BufReader::new(file);

    let mut map: HashMap<String, Material> = HashMap::new();
    let mut current_name: Option<String> = None;

    for line in reader.lines() {
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
            "newmtl" => {
                if let Some(name) = parts.next() {
                    current_name = Some(name.to_string());
                    map.entry(name.to_string())
                        .or_insert_with(Material::placeholder);
                }
            }
            "Kd" | "Ka" | "Ks" => {
                let v: Vec<f64> = parts.filter_map(|p| p.parse::<f64>().ok()).collect();
                if v.len() >= 3 {
                    if let Some(name) = &current_name {
                        if let Some(m) = map.get_mut(name) {
                            let triple = [v[0], v[1], v[2]];
                            match tag {
                                "Kd" => m.diffuse = triple,
                                "Ks" => m.specular = triple,
                                _ => {}
                            }
                        }
                    }
                }
            }
            "Ns" => {
                if let Some(val) = parts.next().and_then(|p| p.parse::<f64>().ok()) {
                    if let Some(name) = &current_name {
                        if let Some(m) = map.get_mut(name) {
                            m.shininess = val;
                        }
                    }
                }
            }
            "d" | "Tr" => {
                if let Some(val) = parts.next().and_then(|p| p.parse::<f64>().ok()) {
                    if let Some(name) = &current_name {
                        if let Some(m) = map.get_mut(name) {
                            // `d` = opacity, `Tr` = transparency (1 - d).
                            m.opacity = if tag == "d" { val } else { 1.0 - val };
                        }
                    }
                }
            }
            _ => {}
        }
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_single_material() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.mtl");
        let mut f = File::create(&p).unwrap();
        writeln!(f, "newmtl RAL9005").unwrap();
        writeln!(f, "Kd 0.1 0.1 0.12").unwrap();
        writeln!(f, "Ks 0.5 0.5 0.5").unwrap();
        writeln!(f, "Ns 25.0").unwrap();
        writeln!(f, "d 0.95").unwrap();

        let mats = parse_mtl(&p).unwrap();
        let m = mats.get("RAL9005").unwrap();
        assert!((m.diffuse[0] - 0.1).abs() < 1e-9);
        assert!((m.shininess - 25.0).abs() < 1e-9);
        assert!((m.opacity - 0.95).abs() < 1e-9);
    }

    #[test]
    fn parses_multiple_materials() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.mtl");
        let mut f = File::create(&p).unwrap();
        writeln!(f, "newmtl A").unwrap();
        writeln!(f, "Kd 1 0 0").unwrap();
        writeln!(f, "newmtl B").unwrap();
        writeln!(f, "Kd 0 1 0").unwrap();

        let mats = parse_mtl(&p).unwrap();
        assert_eq!(mats.len(), 2);
        assert!((mats["A"].diffuse[0] - 1.0).abs() < 1e-9);
        assert!((mats["B"].diffuse[1] - 1.0).abs() < 1e-9);
    }
}
