use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use crate::config::{CsvFormat, EmptyCellPolicy};
use crate::error::R3Error;

/// Header-derived layout of a wide capture: which robots it covers and where
/// each robot's six axis columns sit. Cheap to build (header-only) and the
/// input that drives [`frame_reader`].
#[derive(Debug, Clone, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct CsvSchema {
    /// Robot IDs discovered from the column headers, in first-seen order.
    pub robots: Vec<String>,
    pub axes_per_robot: usize,
    /// robot_id -> per-axis column index (None = that axis column is absent).
    pub column_index: BTreeMap<String, [Option<usize>; 6]>,
    /// Index of the time column (`t_profinet_s`).
    pub time_col: usize,
    pub row_count: Option<usize>,
    pub t_min: Option<f64>,
    pub t_max: Option<f64>,
}

/// Parse a wide-format column name `<ROBOT_ID>_A<n>_deg` into (robot_id, axis
/// index 0..5). Returns None for the time column or any unrecognised header.
fn parse_wide_column(name: &str) -> Option<(String, usize)> {
    let stem = name.strip_suffix("_deg")?;
    let (robot, axis) = stem.rsplit_once("_A")?;
    let n: usize = axis.parse().ok()?;
    if !(1..=6).contains(&n) || robot.is_empty() {
        return None;
    }
    Some((robot.to_string(), n - 1))
}

fn open_reader(path: &Path) -> Result<csv::Reader<std::fs::File>, R3Error> {
    csv::ReaderBuilder::new()
        .flexible(true)
        .from_path(path)
        .map_err(|e| R3Error::Csv(format!("open {}: {}", path.display(), e)))
}

// FR-100, FR-104. Build the schema from the header, then stream the rows once
// to fill in row_count + time span. Never holds more than one row in memory.
pub fn inspect_csv(path: &Path, format: CsvFormat) -> Result<CsvSchema, R3Error> {
    if format == CsvFormat::Long {
        // FR-105 / BL-01: long-form intake is deferred.
        return Err(R3Error::Csv(
            "long-form CSV intake not yet supported (BL-01)".to_string(),
        ));
    }

    let mut rdr = open_reader(path)?;
    let header = rdr
        .headers()
        .map_err(|e| R3Error::Csv(format!("read header: {}", e)))?
        .clone();

    let mut time_col = 0usize;
    let mut robots: Vec<String> = Vec::new();
    let mut column_index: BTreeMap<String, [Option<usize>; 6]> = BTreeMap::new();
    for (i, col) in header.iter().enumerate() {
        if col == "t_profinet_s" {
            time_col = i;
            continue;
        }
        if let Some((robot, axis)) = parse_wide_column(col) {
            if !column_index.contains_key(&robot) {
                robots.push(robot.clone());
            }
            column_index.entry(robot).or_insert([None; 6])[axis] = Some(i);
        }
    }
    if column_index.is_empty() {
        return Err(R3Error::Csv(format!(
            "{}: no `<robot>_A<n>_deg` columns found",
            path.display()
        )));
    }

    // One streaming pass for counts + time span.
    let mut row_count = 0usize;
    let mut t_min: Option<f64> = None;
    let mut t_max: Option<f64> = None;
    let mut rec = csv::StringRecord::new();
    while rdr
        .read_record(&mut rec)
        .map_err(|e| R3Error::Csv(format!("read row: {}", e)))?
    {
        if let Some(t) = rec.get(time_col).and_then(|s| s.trim().parse::<f64>().ok()) {
            t_min.get_or_insert(t);
            t_max = Some(t);
        }
        row_count += 1;
    }

    Ok(CsvSchema {
        robots,
        axes_per_robot: 6,
        column_index,
        time_col,
        row_count: Some(row_count),
        t_min,
        t_max,
    })
}

/// One resolved timeline frame: a timestamp and each robot's six axis values
/// in degrees (None = no value for that axis under the active policy).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub t_s: f64,
    pub joints_deg: BTreeMap<String, [Option<f64>; 6]>,
}

/// Streaming frame iterator. Owns the CSV reader and (for HoldLast) a per-robot
/// last-value cache, so memory is O(robots × 6) regardless of row count.
pub struct FrameIter {
    rdr: csv::Reader<std::fs::File>,
    schema: CsvSchema,
    policy: EmptyCellPolicy,
    cache: BTreeMap<String, [Option<f64>; 6]>,
    rec: csv::StringRecord,
}

impl Iterator for FrameIter {
    type Item = Result<Frame, R3Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.rdr.read_record(&mut self.rec) {
            Ok(false) => None,
            Err(e) => Some(Err(R3Error::Csv(format!("read row: {}", e)))),
            Ok(true) => {
                let t_s = match self
                    .rec
                    .get(self.schema.time_col)
                    .and_then(|s| s.trim().parse::<f64>().ok())
                {
                    Some(t) => t,
                    None => return Some(Err(R3Error::Csv("row missing t_profinet_s".into()))),
                };

                let mut joints_deg: BTreeMap<String, [Option<f64>; 6]> = BTreeMap::new();
                for (robot, cols) in &self.schema.column_index {
                    // Parse this row's present values for the robot.
                    let mut row_vals: [Option<f64>; 6] = [None; 6];
                    for axis in 0..6 {
                        if let Some(ci) = cols[axis] {
                            if let Some(v) =
                                self.rec.get(ci).and_then(|s| s.trim().parse::<f64>().ok())
                            {
                                row_vals[axis] = Some(v);
                            }
                        }
                    }

                    let resolved = match self.policy {
                        EmptyCellPolicy::Skip => row_vals,
                        EmptyCellPolicy::HoldLast => {
                            let cached = self.cache.entry(robot.clone()).or_insert([None; 6]);
                            for axis in 0..6 {
                                if let Some(v) = row_vals[axis] {
                                    cached[axis] = Some(v);
                                }
                            }
                            *cached
                        }
                    };
                    joints_deg.insert(robot.clone(), resolved);
                }

                Some(Ok(Frame { t_s, joints_deg }))
            }
        }
    }
}

// FR-101, FR-102, FR-103. Open a streaming iterator over the capture's frames.
pub fn frame_reader(
    path: &Path,
    schema: &CsvSchema,
    format: CsvFormat,
    policy: EmptyCellPolicy,
) -> Result<FrameIter, R3Error> {
    if format == CsvFormat::Long {
        return Err(R3Error::Csv(
            "long-form CSV intake not yet supported (BL-01)".to_string(),
        ));
    }
    let mut rdr = open_reader(path)?;
    // Advance past the header so read_record yields data rows.
    let _ = rdr
        .headers()
        .map_err(|e| R3Error::Csv(format!("read header: {}", e)))?;
    Ok(FrameIter {
        rdr,
        schema: schema.clone(),
        policy,
        cache: BTreeMap::new(),
        rec: csv::StringRecord::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_csv(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cap.csv");
        std::fs::File::create(&p).unwrap().write_all(body.as_bytes()).unwrap();
        (dir, p)
    }

    #[test]
    fn parse_wide_column_extracts_robot_and_axis() {
        assert_eq!(
            parse_wide_column("113190R01_A1_deg"),
            Some(("113190R01".to_string(), 0))
        );
        assert_eq!(
            parse_wide_column("113230R02_A6_deg"),
            Some(("113230R02".to_string(), 5))
        );
        assert_eq!(parse_wide_column("t_profinet_s"), None);
    }

    // FR-100/104: header → schema; counts + time span from a streaming pass.
    #[test]
    fn inspect_reports_robots_axes_and_span() {
        let (_d, p) = write_csv(
            "t_profinet_s,R1_A1_deg,R1_A2_deg,R1_A3_deg,R1_A4_deg,R1_A5_deg,R1_A6_deg\n\
             0.0,1,2,3,4,5,6\n\
             0.1,7,8,9,10,11,12\n",
        );
        let s = inspect_csv(&p, CsvFormat::Wide).unwrap();
        assert_eq!(s.robots, vec!["R1".to_string()]);
        assert_eq!(s.axes_per_robot, 6);
        assert_eq!(s.row_count, Some(2));
        assert_eq!(s.t_min, Some(0.0));
        assert_eq!(s.t_max, Some(0.1));
    }

    // FR-102: hold-last keeps the prior value when a cell is blank.
    #[test]
    fn hold_last_carries_value_forward() {
        let (_d, p) = write_csv(
            "t_profinet_s,R1_A1_deg,R1_A2_deg,R1_A3_deg,R1_A4_deg,R1_A5_deg,R1_A6_deg\n\
             0.0,10,,,,,\n\
             0.1,,,,,,\n",
        );
        let s = inspect_csv(&p, CsvFormat::Wide).unwrap();
        let frames: Vec<_> = frame_reader(&p, &s, CsvFormat::Wide, EmptyCellPolicy::HoldLast)
            .unwrap()
            .map(|f| f.unwrap())
            .collect();
        assert_eq!(frames[0].joints_deg["R1"][0], Some(10.0));
        // Second row's A1 is blank → held at 10.
        assert_eq!(frames[1].joints_deg["R1"][0], Some(10.0));
    }

    // FR-103: skip policy yields None for blank cells (no carry-forward).
    #[test]
    fn skip_policy_does_not_carry_forward() {
        let (_d, p) = write_csv(
            "t_profinet_s,R1_A1_deg,R1_A2_deg,R1_A3_deg,R1_A4_deg,R1_A5_deg,R1_A6_deg\n\
             0.0,10,,,,,\n\
             0.1,,,,,,\n",
        );
        let s = inspect_csv(&p, CsvFormat::Wide).unwrap();
        let frames: Vec<_> = frame_reader(&p, &s, CsvFormat::Wide, EmptyCellPolicy::Skip)
            .unwrap()
            .map(|f| f.unwrap())
            .collect();
        assert_eq!(frames[0].joints_deg["R1"][0], Some(10.0));
        assert_eq!(frames[1].joints_deg["R1"][0], None);
    }
}
