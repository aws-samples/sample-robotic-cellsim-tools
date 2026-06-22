use std::path::{Path, PathBuf};

use crate::error::DresserError;
use crate::parser::parse_urdf;
use crate::types::UrdfRobot;
use crate::validator::validate;

#[derive(Debug, Clone)]
pub enum AssetStatus {
    Valid,
    Incomplete { issues: Vec<String> },
    Error(String),
}

#[derive(Debug, Clone)]
pub struct RobotEntry {
    pub name: String,
    pub path: PathBuf,
    pub flange_link: String,
    pub joint_count: usize,
    pub link_count: usize,
    pub status: AssetStatus,
}

#[derive(Debug, Clone)]
pub struct ToolEntry {
    pub name: String,
    pub path: PathBuf,
    pub visual_count: usize,
    pub status: AssetStatus,
}

#[derive(Debug, Clone)]
pub struct Catalogue {
    pub robots: Vec<RobotEntry>,
    pub tools: Vec<ToolEntry>,
}

// FR-001, FR-002, FR-003: Catalogue discovery and indexing
/// Scan a catalogue directory and index all robots and tools.
///
/// Expected structure:
/// ```text
/// catalogue/
/// ├── urdf-robot-collection/
/// │   └── <robot_name>/<robot_name>.urdf
/// └── urdf-tool-collection/
///     └── <tool_name>/<tool_name>.urdf
/// ```
pub fn scan(root: &Path) -> Result<Catalogue, DresserError> {
    let robot_dir = root.join("urdf-robot-collection");
    let tool_dir = root.join("urdf-tool-collection");

    let robots = if robot_dir.exists() {
        scan_robots(&robot_dir)
    } else {
        Vec::new()
    };

    let tools = if tool_dir.exists() {
        scan_tools(&tool_dir)
    } else {
        Vec::new()
    };

    Ok(Catalogue { robots, tools })
}

fn scan_robots(dir: &Path) -> Vec<RobotEntry> {
    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let urdf_path = path.join(format!("{}.urdf", name));

        if !urdf_path.exists() {
            continue;
        }

        let entry = match parse_urdf(&urdf_path) {
            Ok(robot) => {
                let diags = validate(&robot, &urdf_path, false);
                let errors: Vec<String> = diags.iter()
                    .filter(|d| d.is_error())
                    .map(|d| d.message.clone())
                    .collect();
                let warnings: Vec<String> = diags.iter()
                    .filter(|d| !d.is_error())
                    .map(|d| d.message.clone())
                    .collect();

                let flange_link = find_flange_link(&robot);
                let status = if !errors.is_empty() {
                    AssetStatus::Incomplete { issues: errors }
                } else if !warnings.is_empty() {
                    AssetStatus::Incomplete { issues: warnings }
                } else {
                    AssetStatus::Valid
                };

                RobotEntry {
                    name,
                    path: urdf_path,
                    flange_link,
                    joint_count: robot.joints.len(),
                    link_count: robot.links.len(),
                    status,
                }
            }
            Err(e) => RobotEntry {
                name,
                path: urdf_path,
                flange_link: String::new(),
                joint_count: 0,
                link_count: 0,
                status: AssetStatus::Error(e.to_string()),
            },
        };

        entries.push(entry);
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn scan_tools(dir: &Path) -> Vec<ToolEntry> {
    let mut entries = Vec::new();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return entries,
    };

    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let urdf_path = path.join(format!("{}.urdf", name));

        if !urdf_path.exists() {
            continue;
        }

        let entry = match parse_urdf(&urdf_path) {
            Ok(tool) => {
                let diags = validate(&tool, &urdf_path, true);
                let errors: Vec<String> = diags.iter()
                    .filter(|d| d.is_error())
                    .map(|d| d.message.clone())
                    .collect();
                let warnings: Vec<String> = diags.iter()
                    .filter(|d| !d.is_error())
                    .map(|d| d.message.clone())
                    .collect();

                let visual_count: usize = tool.links.iter().map(|l| l.visuals.len()).sum();
                let status = if !errors.is_empty() {
                    AssetStatus::Incomplete { issues: errors }
                } else if !warnings.is_empty() {
                    AssetStatus::Incomplete { issues: warnings }
                } else {
                    AssetStatus::Valid
                };

                ToolEntry {
                    name,
                    path: urdf_path,
                    visual_count,
                    status,
                }
            }
            Err(e) => ToolEntry {
                name,
                path: urdf_path,
                visual_count: 0,
                status: AssetStatus::Error(e.to_string()),
            },
        };

        entries.push(entry);
    }

    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

fn find_flange_link(robot: &UrdfRobot) -> String {
    let children: std::collections::HashSet<&str> =
        robot.joints.iter().map(|j| j.child.as_str()).collect();
    let parents: std::collections::HashSet<&str> =
        robot.joints.iter().map(|j| j.parent.as_str()).collect();

    let mut best_leaf = "";
    let mut best_depth = 0;

    for leaf in children.iter().filter(|c| !parents.contains(*c)) {
        let mut current = *leaf;
        let mut depth = 0;
        loop {
            if let Some(joint) = robot.joints.iter().find(|j| j.child == current) {
                current = &joint.parent;
                depth += 1;
            } else {
                break;
            }
        }
        if depth > best_depth || (depth == best_depth && *leaf < best_leaf) {
            best_depth = depth;
            best_leaf = leaf;
        }
    }

    best_leaf.to_string()
}

impl Catalogue {
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n  \"robots\": [\n");
        for (i, r) in self.robots.iter().enumerate() {
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"flange\": \"{}\", \"joints\": {}, \"links\": {}, \"status\": \"{}\"}}",
                r.name, r.flange_link, r.joint_count, r.link_count, status_str(&r.status)
            ));
            if i < self.robots.len() - 1 { out.push(','); }
            out.push('\n');
        }
        out.push_str("  ],\n  \"tools\": [\n");
        for (i, t) in self.tools.iter().enumerate() {
            out.push_str(&format!(
                "    {{\"name\": \"{}\", \"visuals\": {}, \"status\": \"{}\"}}",
                t.name, t.visual_count, status_str(&t.status)
            ));
            if i < self.tools.len() - 1 { out.push(','); }
            out.push('\n');
        }
        out.push_str("  ]\n}\n");
        out
    }

    pub fn print_table(&self) {
        println!("ROBOTS ({}):", self.robots.len());
        println!("  {:<30} {:<30} {:>6} {:>6}  {}", "Name", "Flange", "Joints", "Links", "Status");
        println!("  {}", "-".repeat(90));
        for r in &self.robots {
            println!("  {:<30} {:<30} {:>6} {:>6}  {}",
                r.name, r.flange_link, r.joint_count, r.link_count, status_str(&r.status));
        }
        println!();
        println!("TOOLS ({}):", self.tools.len());
        println!("  {:<40} {:>8}  {}", "Name", "Visuals", "Status");
        println!("  {}", "-".repeat(65));
        for t in &self.tools {
            println!("  {:<40} {:>8}  {}", t.name, t.visual_count, status_str(&t.status));
        }
    }
}

fn status_str(status: &AssetStatus) -> &str {
    match status {
        AssetStatus::Valid => "valid",
        AssetStatus::Incomplete { .. } => "incomplete",
        AssetStatus::Error(_) => "error",
    }
}
