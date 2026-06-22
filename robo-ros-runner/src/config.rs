use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::R3Error;
use crate::mapping::MappingConfig;

/// Wide vs long capture layout. Wide is the per-robot-column
/// `joint_angles_timeline.csv`; long is the tidy `combined_timeseries.csv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CsvFormat {
    #[default]
    Wide,
    Long,
}

/// What to publish when a robot's cell is blank for the current timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmptyCellPolicy {
    /// Keep the last non-empty value per axis (default). A robot with no
    /// sample yet stays unpublished until its first real value arrives.
    #[default]
    HoldLast,
    /// Treat blanks as "no value this frame"; publish a robot only when all
    /// six axes are present.
    Skip,
}

/// Which transport the publish goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinkKind {
    /// Real DDS publish via ros2-client — the default transport.
    #[default]
    Dds,
    /// Print the `ros2 topic pub` equivalent instead of publishing — the
    /// dry-run / preview path (no live ROS graph needed).
    Stdout,
}

impl SinkKind {
    pub fn parse(s: &str) -> Result<Self, R3Error> {
        match s.to_ascii_lowercase().as_str() {
            "stdout" | "dry-run" | "dryrun" => Ok(Self::Stdout),
            "dds" | "ros2" => Ok(Self::Dds),
            other => Err(R3Error::Config(format!(
                "unknown sink '{}' (expected 'stdout' or 'dds')",
                other
            ))),
        }
    }
}

fn default_rate() -> f64 {
    1.0
}

/// A full R3 run specification. Durable inputs + the joint mapping live here;
/// per-invocation run knobs come from CLI flags (see [`CliOverrides`]).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunConfig {
    /// robo-fitting-room cell manifest (resolves robot IDs + dressed URDFs).
    pub cell_yaml: PathBuf,
    /// Joint-angle capture to replay.
    pub csv: PathBuf,
    #[serde(default)]
    pub csv_format: CsvFormat,
    #[serde(default = "default_rate")]
    pub rate: f64,
    #[serde(default)]
    pub loop_play: bool,
    #[serde(default)]
    pub start_s: Option<f64>,
    #[serde(default)]
    pub end_s: Option<f64>,
    /// `None` = all robots; otherwise only these robot_ids are published.
    #[serde(default)]
    pub robots: Option<Vec<String>>,
    #[serde(default)]
    pub empty_cell: EmptyCellPolicy,
    /// robot_id -> full topic string (bypasses the derived topic).
    #[serde(default)]
    pub topic_overrides: BTreeMap<String, String>,
    /// Override the cell namespace segment (defaults to the manifest name).
    #[serde(default)]
    pub cell_ns_override: Option<String>,
    #[serde(default)]
    pub mapping: MappingConfig,
}

impl RunConfig {
    /// Synthesize a default config from just a cell + CSV (CLI-only runs with
    /// no config file). Identity mapping, capture-rate playback, hold-last.
    pub fn from_paths(cell_yaml: PathBuf, csv: PathBuf) -> Self {
        Self {
            cell_yaml,
            csv,
            csv_format: CsvFormat::default(),
            rate: 1.0,
            loop_play: false,
            start_s: None,
            end_s: None,
            robots: None,
            empty_cell: EmptyCellPolicy::default(),
            topic_overrides: BTreeMap::new(),
            cell_ns_override: None,
            mapping: MappingConfig::default(),
        }
    }
}

// FR-800, FR-801. Load a RunConfig from YAML.
pub fn load_run_config(path: &Path) -> Result<RunConfig, R3Error> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| R3Error::Config(format!("read {}: {}", path.display(), e)))?;
    let mut cfg: RunConfig = serde_yaml::from_str(&content)
        .map_err(|e| R3Error::Config(format!("parse {}: {}", path.display(), e)))?;
    // Resolve cell_yaml / csv relative to the config file's directory so a
    // config can use paths relative to itself rather than the CWD.
    if let Some(dir) = path.parent() {
        if cfg.cell_yaml.is_relative() {
            cfg.cell_yaml = dir.join(&cfg.cell_yaml);
        }
        if cfg.csv.is_relative() {
            cfg.csv = dir.join(&cfg.csv);
        }
    }
    Ok(cfg)
}

/// CLI flags that override config-file values. `None` = leave config as-is.
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub cell_yaml: Option<PathBuf>,
    pub csv: Option<PathBuf>,
    pub csv_format: Option<CsvFormat>,
    pub rate: Option<f64>,
    pub loop_play: Option<bool>,
    pub start_s: Option<f64>,
    pub end_s: Option<f64>,
    pub robots: Option<Vec<String>>,
    pub empty_cell: Option<EmptyCellPolicy>,
    /// Force clamp-to-URDF-limits on regardless of the config's mapping block.
    pub clamp: Option<bool>,
}

// FR-803, FR-605. Layer CLI flags over a config (flags win).
pub fn apply_cli_overrides(mut cfg: RunConfig, ov: CliOverrides) -> RunConfig {
    if let Some(v) = ov.cell_yaml {
        cfg.cell_yaml = v;
    }
    if let Some(v) = ov.csv {
        cfg.csv = v;
    }
    if let Some(v) = ov.csv_format {
        cfg.csv_format = v;
    }
    if let Some(v) = ov.rate {
        cfg.rate = v;
    }
    if let Some(v) = ov.loop_play {
        cfg.loop_play = v;
    }
    if ov.start_s.is_some() {
        cfg.start_s = ov.start_s;
    }
    if ov.end_s.is_some() {
        cfg.end_s = ov.end_s;
    }
    if ov.robots.is_some() {
        cfg.robots = ov.robots;
    }
    if let Some(v) = ov.empty_cell {
        cfg.empty_cell = v;
    }
    if let Some(true) = ov.clamp {
        cfg.mapping.clamp_to_limits = true;
    }
    cfg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_format_defaults_to_wide() {
        assert_eq!(CsvFormat::default(), CsvFormat::Wide);
    }

    #[test]
    fn cli_overrides_win_over_config() {
        let base = RunConfig::from_paths(PathBuf::from("a.yaml"), PathBuf::from("a.csv"));
        let ov = CliOverrides {
            rate: Some(2.0),
            loop_play: Some(true),
            clamp: Some(true),
            ..Default::default()
        };
        let merged = apply_cli_overrides(base, ov);
        assert_eq!(merged.rate, 2.0);
        assert!(merged.loop_play);
        assert!(merged.mapping.clamp_to_limits);
    }
}
