use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

use robo_ros_runner::{
    apply_cli_overrides, build_plan, inspect_csv, load_run_config, make_sink, publish_snapshot,
    resolve_topics, run_playback, CliOverrides, CsvFormat, RealClock, RunConfig, SinkKind,
};
use robo_ros_runner::playback::CancelFlag;

const BEFORE_HELP: &str = "\
robo-ros-runner (R3) — replay captured joint timelines onto ROS 2 / Isaac Sim
=============================================================================
Two ways to use this tool:

  CLI (one-shot):    validate / inspect / replay a capture, exit.
  Server (long-run): host the REST API; start/stop playback jobs.

Both transports call the same library functions — there is no logic
behind the server that the CLI cannot also reach.";

const AFTER_HELP: &str = "\
Quick start
-----------
  # what topics will a cell drive?
  robo-ros-runner topics --cell ../assets/fitted-robots/k8urw11-cell.yaml \\
      --csv ../assets/data-captures/joint_angles_timeline.csv

  # what's in a capture?
  robo-ros-runner inspect-csv --csv ../assets/data-captures/joint_angles_timeline.csv

  # preview without a live ROS graph (prints the ros2-topic-pub equivalent)
  robo-ros-runner dry-run --cell <cell.yaml> --csv <capture.csv>

  # real replay to Isaac Sim (DDS is the default transport)
  robo-ros-runner run --config r3-run.yaml --ros-domain-id 0

  # long-running server
  robo-ros-runner serve --asset-root ../assets
  open http://localhost:3050/docs     # Swagger UI

A plain `cargo build` produces the whole tool — CLI, ROS 2 / DDS transport,
and REST server, all on. `run` publishes real DDS by default; `dry-run`
(or `--sink stdout`) previews without touching the network.
See `<command> --help` for full per-command flag listings.";

#[derive(Parser)]
#[command(
    name = "robo-ros-runner",
    version,
    about = "Replay captured joint-angle timelines onto ROS 2 joint_command topics (CLI + REST server)",
    before_help = BEFORE_HELP,
    after_help = AFTER_HELP,
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    // -------------------------------------------------------------
    // CLI mode — one-shot, no server
    // -------------------------------------------------------------
    /// [CLI] Replay a capture onto joint_command topics. (alias: play)
    #[command(alias = "play")]
    Run(RunArgs),

    /// [CLI] Dry run: print the `ros2 topic pub` equivalents, never touch DDS.
    DryRun(RunArgs),

    /// [CLI] Validate config + cell + CSV + URDF mapping without publishing.
    Validate {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        cell: Option<PathBuf>,
        #[arg(long)]
        csv: Option<PathBuf>,
    },

    /// [CLI] Print the resolved per-robot joint_command topics for a cell.
    Topics {
        #[arg(long)]
        cell: PathBuf,
        #[arg(long)]
        csv: Option<PathBuf>,
    },

    /// [CLI] Inspect a capture: robots, axes, time span, row count.
    InspectCsv {
        #[arg(long)]
        csv: PathBuf,
        #[arg(long, default_value = "wide")]
        format: String,
    },

    // -------------------------------------------------------------
    // Server mode — long-running HTTP service
    // -------------------------------------------------------------
    /// [SERVER] Boot the HTTP server: REST API at `/api/v1`, Swagger UI at `/docs`.
    Serve {
        /// Bind host. Use `0.0.0.0` to listen on all interfaces.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// TCP port.
        #[arg(long, default_value_t = 3050)]
        port: u16,
        /// Root directory holding cell manifests + captures. Every `path` in
        /// an API call is sandboxed under this; `..` segments are rejected.
        #[arg(long)]
        asset_root: PathBuf,
    },
}

/// Shared flags for `run` / `dry-run`.
#[derive(clap::Args)]
struct RunArgs {
    /// R3 run-config YAML (durable inputs + joint mapping).
    #[arg(long)]
    config: Option<PathBuf>,
    /// Cell manifest YAML (overrides config.cell_yaml).
    #[arg(long)]
    cell: Option<PathBuf>,
    /// Capture CSV (overrides config.csv).
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Playback speed multiplier (1.0 = capture rate).
    #[arg(long)]
    rate: Option<f64>,
    /// Loop the timeline forever (until Ctrl-C).
    #[arg(long)]
    loop_play: bool,
    /// Start time (seconds) — skip frames before this.
    #[arg(long)]
    start: Option<f64>,
    /// End time (seconds) — stop after this.
    #[arg(long)]
    end: Option<f64>,
    /// Comma-separated robot_id filter (default: all robots in the capture).
    #[arg(long, value_delimiter = ',')]
    robots: Option<Vec<String>>,
    /// Publish a single frame (at --start, or the first) and exit.
    #[arg(long)]
    once: bool,
    /// Transport: `dds` (default, real ROS 2 publish) or `stdout` (preview).
    #[arg(long, default_value = "dds")]
    sink: String,
    /// ROS_DOMAIN_ID for the DDS sink.
    #[arg(long, default_value_t = 0)]
    ros_domain_id: u32,
    /// Force clamp-to-URDF-limits on (overrides config).
    #[arg(long)]
    clamp: bool,
}

/// Resolve a RunConfig from --config (if any) layered with the run flags.
fn config_from_run_args(args: &RunArgs) -> Result<RunConfig, String> {
    let base = match &args.config {
        Some(p) => load_run_config(p).map_err(|e| e.to_string())?,
        None => {
            let cell = args
                .cell
                .clone()
                .ok_or("either --config or --cell + --csv is required")?;
            let csv = args
                .csv
                .clone()
                .ok_or("either --config or --cell + --csv is required")?;
            RunConfig::from_paths(cell, csv)
        }
    };
    let ov = CliOverrides {
        cell_yaml: args.cell.clone(),
        csv: args.csv.clone(),
        csv_format: None,
        rate: args.rate,
        loop_play: if args.loop_play { Some(true) } else { None },
        start_s: args.start,
        end_s: args.end,
        robots: args.robots.clone(),
        empty_cell: None,
        clamp: if args.clamp { Some(true) } else { None },
    };
    Ok(apply_cli_overrides(base, ov))
}

fn cmd_run(args: &RunArgs, force_stdout: bool) -> i32 {
    let cfg = match config_from_run_args(args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return 1;
        }
    };
    let plan = match build_plan(&cfg) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return 1;
        }
    };

    let sink_kind = if force_stdout {
        SinkKind::Stdout
    } else {
        match SinkKind::parse(&args.sink) {
            Ok(k) => k,
            Err(e) => {
                eprintln!("ERROR: {}", e);
                return 1;
            }
        }
    };
    let mut sink = match make_sink(sink_kind, args.ros_domain_id) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return 1;
        }
    };

    let report = if args.once {
        publish_snapshot(&plan, sink.as_mut())
    } else {
        run_playback(&plan, sink.as_mut(), &RealClock, &CancelFlag::new())
    };

    match report {
        Ok(r) => {
            eprintln!(
                "Done: {} frames, {} messages, {} clamped, {} loops, {:.3}s of capture (sink: {})",
                r.frames_published,
                r.messages_published,
                r.clamped_count,
                r.looped,
                r.duration_s,
                sink.kind(),
            );
            0
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            1
        }
    }
}

fn cmd_validate(config: Option<PathBuf>, cell: Option<PathBuf>, csv: Option<PathBuf>) -> i32 {
    let args = RunArgs {
        config,
        cell,
        csv,
        rate: None,
        loop_play: false,
        start: None,
        end: None,
        robots: None,
        once: false,
        sink: "stdout".to_string(),
        ros_domain_id: 0,
        clamp: false,
    };
    let cfg = match config_from_run_args(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return 1;
        }
    };
    match build_plan(&cfg) {
        Ok(plan) => {
            println!(
                "OK: {} robot(s) ready to play from {}",
                plan.robots.len(),
                cfg.csv.display()
            );
            for t in &plan.topics {
                println!(
                    "  {} -> {} ({})",
                    t.robot_id,
                    t.topic,
                    if t.has_csv_data { "has data" } else { "NO CSV DATA" }
                );
            }
            0
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            1
        }
    }
}

fn cmd_topics(cell: PathBuf, csv: Option<PathBuf>) -> i32 {
    let scene = match robo_fitting_room::parse_scene(&cell) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERROR: {}", e);
            return 1;
        }
    };
    // Determine which robots the CSV covers (if a CSV is given).
    let csv_robots: Vec<String> = match &csv {
        Some(p) => match inspect_csv(p, CsvFormat::Wide) {
            Ok(s) => s.robots,
            Err(e) => {
                eprintln!("ERROR: {}", e);
                return 1;
            }
        },
        None => Vec::new(),
    };
    let cfg = RunConfig::from_paths(cell.clone(), csv.unwrap_or_default());
    match resolve_topics(&scene, &cfg, &csv_robots) {
        Ok(topics) => {
            for t in &topics {
                let flag = if csv_robots.is_empty() {
                    ""
                } else if t.has_csv_data {
                    " [has data]"
                } else {
                    " [NO CSV DATA]"
                };
                println!("{}{}", t.topic, flag);
            }
            0
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            1
        }
    }
}

fn cmd_inspect_csv(csv: PathBuf, format: String) -> i32 {
    let fmt = match format.to_ascii_lowercase().as_str() {
        "wide" => CsvFormat::Wide,
        "long" => CsvFormat::Long,
        other => {
            eprintln!("ERROR: unknown format '{}' (expected 'wide' or 'long')", other);
            return 1;
        }
    };
    match inspect_csv(&csv, fmt) {
        Ok(s) => {
            println!(
                "Capture: {} robots, {} axes/robot, {} rows",
                s.robots.len(),
                s.axes_per_robot,
                s.row_count.unwrap_or(0),
            );
            if let (Some(a), Some(b)) = (s.t_min, s.t_max) {
                println!("  Time span: {:.3}s .. {:.3}s ({:.3}s)", a, b, b - a);
            }
            println!("  Robots: {}", s.robots.join(", "));
            0
        }
        Err(e) => {
            eprintln!("ERROR: {}", e);
            1
        }
    }
}

fn main() {
    // Default log level = info; users override with RUST_LOG=debug etc.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Command::Run(args) => cmd_run(&args, false),
        Command::DryRun(args) => cmd_run(&args, true),
        Command::Validate { config, cell, csv } => cmd_validate(config, cell, csv),
        Command::Topics { cell, csv } => cmd_topics(cell, csv),
        Command::InspectCsv { csv, format } => cmd_inspect_csv(csv, format),
        Command::Serve {
            host,
            port,
            asset_root,
        } => {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("ERROR: tokio runtime build: {}", e);
                    process::exit(1);
                }
            };
            match rt.block_on(robo_ros_runner::server::run(&host, port, asset_root)) {
                Ok(()) => 0,
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                    1
                }
            }
        }
    };
    process::exit(exit_code);
}
