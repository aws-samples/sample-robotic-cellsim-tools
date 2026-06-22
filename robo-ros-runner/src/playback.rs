use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use robo_fitting_room::parse_scene;
use serde::Serialize;

use crate::config::RunConfig;
use crate::csv_intake::{frame_reader, inspect_csv, CsvSchema};
use crate::error::R3Error;
use crate::mapping::{joint_limits_for_robot, map_degrees_to_radians, AxisMap, JointLimits};
use crate::topics::{resolve_topics, ResolvedTopic};
use crate::transport::{CommandSink, JointCommand, Stamp};

/// Largest inter-frame sleep we honor; protects against a huge gap in the
/// capture stalling playback for minutes.
const MAX_FRAME_GAP_S: f64 = 1.0;

/// Injectable clock so playback runs at wall-clock rate in production and
/// instantly + deterministically in tests.
pub trait Clock {
    fn sleep(&self, dur: Duration);
}

/// Real wall-clock: sleeps for the requested duration.
pub struct RealClock;

impl Clock for RealClock {
    fn sleep(&self, dur: Duration) {
        std::thread::sleep(dur);
    }
}

/// Cooperative cancellation flag shared with a server job handle.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Everything resolved once before the replay loop runs: topics, per-robot
/// joint limits + axis maps, the CSV schema, and the config knobs.
pub struct PlaybackPlan {
    pub cfg: RunConfig,
    pub topics: Vec<ResolvedTopic>,
    pub schema: CsvSchema,
    /// robot_id -> (joint limits, six axis maps). Only robots that are in the
    /// CSV, pass the `robots` filter, and have a topic appear here.
    pub robots: BTreeMap<String, RobotPlan>,
}

pub struct RobotPlan {
    pub topic: String,
    pub limits: JointLimits,
    pub axes: [AxisMap; 6],
}

/// Outcome summary of a playback run.
#[derive(Debug, Clone, Default, Serialize)]
#[derive(utoipa::ToSchema)]
pub struct PlaybackReport {
    pub robots: Vec<ResolvedTopic>,
    pub frames_published: u64,
    pub messages_published: u64,
    pub duration_s: f64,
    pub clamped_count: u64,
    pub looped: u32,
}

// FR-500. Resolve scene + URDF limits + CSV schema + topics into a plan. Does
// all parsing/validation up front so the replay loop is pure I/O + math.
pub fn build_plan(cfg: &RunConfig) -> Result<PlaybackPlan, R3Error> {
    let scene = parse_scene(&cfg.cell_yaml)
        .map_err(|e| R3Error::Scene(format!("{}: {}", cfg.cell_yaml.display(), e)))?;
    let schema = inspect_csv(&cfg.csv, cfg.csv_format)?;
    let topics = resolve_topics(&scene, cfg, &schema.robots)?;

    // robot_id -> dressed_urdf, for limit extraction.
    let urdf_by_id: BTreeMap<String, _> = scene
        .robots
        .iter()
        .map(|r| (r.robot_id.clone(), r.dressed_urdf.clone()))
        .collect();
    let topic_by_id: BTreeMap<String, String> = topics
        .iter()
        .map(|t| (t.robot_id.clone(), t.topic.clone()))
        .collect();

    let mut robots = BTreeMap::new();
    for robot_id in &schema.robots {
        // Honor the --robots filter.
        if let Some(filter) = &cfg.robots {
            if !filter.iter().any(|f| f == robot_id) {
                continue;
            }
        }
        let Some(urdf) = urdf_by_id.get(robot_id) else {
            // CSV has data for a robot the cell manifest doesn't place — skip.
            log::warn!(
                "robot {} present in CSV but not in cell manifest; skipping",
                robot_id
            );
            continue;
        };
        let Some(topic) = topic_by_id.get(robot_id) else {
            continue;
        };
        let limits = joint_limits_for_robot(urdf)?;
        let axes = cfg.mapping.axes_for(robot_id);
        robots.insert(
            robot_id.clone(),
            RobotPlan {
                topic: topic.clone(),
                limits,
                axes,
            },
        );
    }

    if robots.is_empty() {
        return Err(R3Error::Config(
            "no robots to play (check the cell manifest, CSV columns, and --robots filter)".into(),
        ));
    }

    Ok(PlaybackPlan {
        cfg: cfg.clone(),
        topics,
        schema,
        robots,
    })
}

/// Publish one frame's worth of joint commands for every robot that has a full
/// set of mapped positions. Returns (messages, clamped) counts.
fn publish_frame(
    plan: &PlaybackPlan,
    joints_deg: &BTreeMap<String, [Option<f64>; 6]>,
    t_s: f64,
    sink: &mut dyn CommandSink,
) -> Result<(u64, u64), R3Error> {
    let stamp = Stamp::from_seconds(t_s);
    let clamp = plan.cfg.mapping.clamp_to_limits;
    let warn = plan.cfg.mapping.warn_out_of_range;
    let mut messages = 0u64;
    let mut clamped = 0u64;

    for (robot_id, rp) in &plan.robots {
        let Some(deg) = joints_deg.get(robot_id) else {
            continue;
        };
        let (mapped, c) = map_degrees_to_radians(deg, &rp.axes, &rp.limits, clamp, warn);
        clamped += c as u64;
        // Only publish when all six axes have a value (a partial pose would
        // command undefined joints).
        if mapped.iter().any(|p| p.is_none()) {
            continue;
        }
        let positions: Vec<f64> = mapped.iter().map(|p| p.unwrap()).collect();
        let names: Vec<String> = rp.limits.names.to_vec();
        sink.publish_joint_command(&JointCommand {
            topic: &rp.topic,
            names: &names,
            positions: &positions,
            stamp,
        })?;
        messages += 1;
    }
    Ok((messages, clamped))
}

// FR-506. Publish exactly one frame (the first frame at/after start_s) and
// return. Used by `--once` to pose the cell and exit.
pub fn publish_snapshot(
    plan: &PlaybackPlan,
    sink: &mut dyn CommandSink,
) -> Result<PlaybackReport, R3Error> {
    sink.advertise(&plan.topics)?;
    let start = plan.cfg.start_s.unwrap_or(f64::NEG_INFINITY);
    let mut report = PlaybackReport {
        robots: plan.topics.clone(),
        ..Default::default()
    };
    for frame in frame_reader(
        &plan.cfg.csv,
        &plan.schema,
        plan.cfg.csv_format,
        plan.cfg.empty_cell,
    )? {
        let frame = frame?;
        if frame.t_s < start {
            continue;
        }
        let (msgs, clamped) = publish_frame(plan, &frame.joints_deg, frame.t_s, sink)?;
        report.frames_published = 1;
        report.messages_published = msgs;
        report.clamped_count = clamped;
        report.duration_s = 0.0;
        break;
    }
    sink.flush()?;
    Ok(report)
}

// FR-501..FR-508. Replay the whole timeline: schedule each frame by the
// capture's own deltas (scaled by `rate`), honoring the start/end window,
// loop, and cancellation. `clock` is injected so tests run instantly.
pub fn run_playback(
    plan: &PlaybackPlan,
    sink: &mut dyn CommandSink,
    clock: &dyn Clock,
    cancel: &CancelFlag,
) -> Result<PlaybackReport, R3Error> {
    sink.advertise(&plan.topics)?;
    let rate = if plan.cfg.rate > 0.0 {
        plan.cfg.rate
    } else {
        1.0
    };
    let start = plan.cfg.start_s.unwrap_or(f64::NEG_INFINITY);
    let end = plan.cfg.end_s.unwrap_or(f64::INFINITY);

    let mut report = PlaybackReport {
        robots: plan.topics.clone(),
        ..Default::default()
    };

    loop {
        let mut prev_t: Option<f64> = None;
        let mut first_t: Option<f64> = None;
        let mut last_t: Option<f64> = None;

        for frame in frame_reader(
            &plan.cfg.csv,
            &plan.schema,
            plan.cfg.csv_format,
            plan.cfg.empty_cell,
        )? {
            if cancel.is_cancelled() {
                sink.flush()?;
                return Ok(report);
            }
            let frame = frame?;
            if frame.t_s < start {
                continue;
            }
            if frame.t_s > end {
                break;
            }

            // Sleep the inter-frame delta (scaled by rate, capped).
            if let Some(pt) = prev_t {
                let gap = ((frame.t_s - pt) / rate).clamp(0.0, MAX_FRAME_GAP_S);
                if gap > 0.0 {
                    clock.sleep(Duration::from_secs_f64(gap));
                }
            }
            prev_t = Some(frame.t_s);
            first_t.get_or_insert(frame.t_s);
            last_t = Some(frame.t_s);

            let (msgs, clamped) = publish_frame(plan, &frame.joints_deg, frame.t_s, sink)?;
            report.frames_published += 1;
            report.messages_published += msgs;
            report.clamped_count += clamped;
        }

        if let (Some(a), Some(b)) = (first_t, last_t) {
            report.duration_s += b - a;
        }

        if plan.cfg.loop_play && !cancel.is_cancelled() {
            report.looped += 1;
            continue;
        }
        break;
    }

    sink.flush()?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test clock that never actually sleeps (records nothing).
    struct MockClock;
    impl Clock for MockClock {
        fn sleep(&self, _dur: Duration) {}
    }

    /// A sink that records every published command for assertions.
    #[derive(Default)]
    struct CapturingSink {
        pub commands: Vec<(String, Vec<String>, Vec<f64>)>,
    }
    impl CommandSink for CapturingSink {
        fn publish_joint_command(&mut self, cmd: &JointCommand) -> Result<(), R3Error> {
            self.commands.push((
                cmd.topic.to_string(),
                cmd.names.to_vec(),
                cmd.positions.to_vec(),
            ));
            Ok(())
        }
        fn kind(&self) -> &'static str {
            "capturing"
        }
    }

    use std::io::Write;

    /// Lay down a tiny cell: one robot, a 6-axis URDF, a 2-row capture, and a
    /// matching RunConfig. Returns (tempdir, config).
    fn tiny_cell() -> (tempfile::TempDir, RunConfig) {
        let dir = tempfile::tempdir().unwrap();

        // 6-axis revolute URDF named with the generic like_ convention.
        let urdf = dir.path().join("r.urdf");
        let mut s = String::from("<?xml version=\"1.0\"?>\n<robot name=\"r\">\n");
        for n in &["World", "base", "l1", "l2", "l3", "l4", "l5", "l6"] {
            s.push_str(&format!("  <link name=\"{}\"/>\n", n));
        }
        s.push_str("  <joint name=\"root_joint\" type=\"fixed\">\n    <parent link=\"World\"/>\n    <child link=\"base\"/>\n  </joint>\n");
        for i in 1..=6 {
            let parent = if i == 1 { "base".to_string() } else { format!("l{}", i - 1) };
            s.push_str(&format!(
                "  <joint name=\"joint_a{}\" type=\"revolute\">\n    <parent link=\"{}\"/>\n    <child link=\"l{}\"/>\n    <axis xyz=\"0 0 1\"/>\n    <limit lower=\"-3.14\" upper=\"3.14\" effort=\"100\" velocity=\"1\"/>\n  </joint>\n",
                i, parent, i
            ));
        }
        s.push_str("</robot>\n");
        std::fs::write(&urdf, s).unwrap();

        // Cell manifest referencing the URDF by relative path. A digit-leading
        // robot_id exercises the `r_` topic-prefix path like real captures.
        let cell = dir.path().join("cell.yaml");
        std::fs::write(
            &cell,
            "name: \"Test Cell\"\nrobots:\n  - robot_id: \"113190R01\"\n    dressed_urdf: \"r.urdf\"\n    position: { x: 0, y: 0, z: 0 }\n    orientation: { a: 0, b: 0, c: 0 }\n",
        )
        .unwrap();

        // 2-row capture, full 6-axis values, 0.1 s apart.
        let csv = dir.path().join("cap.csv");
        std::fs::File::create(&csv)
            .unwrap()
            .write_all(
                b"t_profinet_s,113190R01_A1_deg,113190R01_A2_deg,113190R01_A3_deg,113190R01_A4_deg,113190R01_A5_deg,113190R01_A6_deg\n\
                  0.0,0,0,0,0,0,0\n\
                  0.1,90,0,0,0,0,0\n",
            )
            .unwrap();

        let cfg = RunConfig::from_paths(cell, csv);
        (dir, cfg)
    }

    // FR-500/501/508 + FR-301: build_plan resolves the right topic and replay
    // publishes one message per frame with joint_a1..a6 names + radians.
    #[test]
    fn replay_publishes_per_frame() {
        let (_d, cfg) = tiny_cell();
        let plan = build_plan(&cfg).unwrap();
        assert_eq!(plan.topics.len(), 1);
        assert_eq!(plan.topics[0].topic, "/Test_Cell/r_113190R01/joint_command");
        assert!(plan.topics[0].has_csv_data);

        let mut sink = CapturingSink::default();
        let report = run_playback(&plan, &mut sink, &MockClock, &CancelFlag::new()).unwrap();
        assert_eq!(report.frames_published, 2);
        assert_eq!(report.messages_published, 2);
        assert_eq!(sink.commands.len(), 2);
        // joint names in URDF order.
        assert_eq!(sink.commands[0].1, vec![
            "joint_a1", "joint_a2", "joint_a3", "joint_a4", "joint_a5", "joint_a6"
        ]);
        // Second frame A1 = 90 deg -> pi/2 rad.
        assert!((sink.commands[1].2[0] - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }

    // FR-506: snapshot publishes exactly one frame.
    #[test]
    fn snapshot_publishes_one_frame() {
        let (_d, cfg) = tiny_cell();
        let plan = build_plan(&cfg).unwrap();
        let mut sink = CapturingSink::default();
        let report = publish_snapshot(&plan, &mut sink).unwrap();
        assert_eq!(report.frames_published, 1);
        assert_eq!(sink.commands.len(), 1);
    }

    // FR-503: the start/end window trims frames.
    #[test]
    fn window_trims_frames() {
        let (_d, mut cfg) = tiny_cell();
        cfg.start_s = Some(0.05);
        let plan = build_plan(&cfg).unwrap();
        let mut sink = CapturingSink::default();
        let report = run_playback(&plan, &mut sink, &MockClock, &CancelFlag::new()).unwrap();
        assert_eq!(report.frames_published, 1); // only the 0.1 s frame
    }

    // FR-505: loop replays the timeline; a sink that cancels after the first
    // pass bounds the test (else `loop_play` runs forever).
    #[test]
    fn loop_replays_twice() {
        let (_d, mut cfg) = tiny_cell();
        cfg.loop_play = true;
        let plan = build_plan(&cfg).unwrap();
        let cancel = CancelFlag::new();

        struct CancelAfter<'a> {
            inner: CapturingSink,
            cancel: &'a CancelFlag,
            limit: usize,
        }
        impl<'a> CommandSink for CancelAfter<'a> {
            fn publish_joint_command(&mut self, cmd: &JointCommand) -> Result<(), R3Error> {
                self.inner.publish_joint_command(cmd)?;
                if self.inner.commands.len() >= self.limit {
                    self.cancel.cancel();
                }
                Ok(())
            }
            fn kind(&self) -> &'static str {
                "cancel-after"
            }
        }
        // 2 frames in pass 1; the 3rd message lands in pass 2, then cancel.
        let mut sink = CancelAfter {
            inner: CapturingSink::default(),
            cancel: &cancel,
            limit: 3,
        };
        let report = run_playback(&plan, &mut sink, &MockClock, &cancel).unwrap();
        assert!(report.looped >= 1);
        assert!(sink.inner.commands.len() >= 3);
    }
}
