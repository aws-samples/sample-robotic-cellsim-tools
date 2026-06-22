use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

const BEFORE_HELP: &str = "\
urdf-robo-dresser — robot+tool URDF merger (CLI + REST server)
==============================================================
Two ways to use this tool:

  CLI (one-shot):    dress / batch / validate / list against local files.
  Server (long-run): host the same operations as a versioned REST API.

Both transports call the same library functions — there is no logic
behind the server that the CLI cannot also reach.";

const AFTER_HELP: &str = "\
Quick start
-----------
  # one-shot CLI
  urdf-robo-dresser list --catalogue ../assets
  urdf-robo-dresser dress \\
      --robot ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf \\
      --tool  ../assets/urdf-tool-collection/GREIFER_URW1_OP3210_R01G1/GREIFER_URW1_OP3210_R01G1.urdf \\
      --output /tmp/dressed.urdf

  # long-running server (default build includes it)
  urdf-robo-dresser serve --catalogue ../assets
  open http://localhost:3030/         # landing page
  open http://localhost:3030/docs     # Swagger UI

The default build ships both modes. Pass `--no-default-features` to
cargo to produce the slim CLI-only binary without axum/tokio/utoipa.
See `<command> --help` for full per-command flag listings.";

#[derive(Parser)]
#[command(
    name = "urdf-robo-dresser",
    version,
    about = "Programmatic URDF robot+tool merger (CLI + REST server)",
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

    /// [CLI] Merge ONE robot URDF with ONE tool URDF and write the dressed URDF to disk.
    #[command(long_about = "\
[CLI] Merge ONE robot URDF with ONE tool URDF and write the dressed URDF to disk.

Auto-detects the flange link, generates convex-hull collisions for
both inputs, and re-writes mesh paths so the output is self-contained.
No catalogue assumed — pure file-in / file-out.

Examples:
  # Auto-orient a gripper onto a KR240
  urdf-robo-dresser dress \\
      --robot ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf \\
      --tool  ../assets/urdf-tool-collection/GREIFER_URW1_OP3210_R01G1/GREIFER_URW1_OP3210_R01G1.urdf \\
      --output /tmp/kr240_gripper.urdf

  # Force a specific flange link + 90° rotation about the tool Z axis
  urdf-robo-dresser dress \\
      --robot ./robot.urdf --tool ./tool.urdf --output /tmp/out.urdf \\
      --attach-to flange_link --orient 0,0,90 --hull max")]
    Dress {
        /// Path to the robot's base URDF.
        #[arg(long)]
        robot: PathBuf,
        /// Path to the tool URDF to attach.
        #[arg(long)]
        tool: PathBuf,
        /// Output path for the dressed URDF (mesh outputs land beside it).
        #[arg(long)]
        output: PathBuf,
        /// Override the auto-detected flange link.
        #[arg(long)]
        attach_to: Option<String>,
        /// Tool orientation in 90° steps as `x,y,z` degrees, e.g. `0,90,180`.
        #[arg(long, default_value = "0,0,0")]
        orient: String,
        /// Collision hull buffer: `tight` / `medium` (default) / `max`.
        #[arg(long, default_value = "medium")]
        hull: String,
        /// After dressing, open the result in `urdf-viz` (must be on PATH).
        #[arg(long)]
        view: bool,
    },

    /// [CLI] Permutation-merge every robot × every tool in a catalogue.
    #[command(long_about = "\
[CLI] Permutation-merge every robot × every tool in a catalogue.

Walks `<catalogue>/urdf-robot-collection/` × `urdf-tool-collection/`
and writes one dressed URDF per pair under `<output>/<robot>__<tool>/`.
Continues past individual failures and prints a final tally.

Examples:
  # Dress every catalogue permutation, default hull
  urdf-robo-dresser batch --catalogue ../assets --output /tmp/all-dressed

  # Tighter hulls (smaller bbox; more PhysX cost)
  urdf-robo-dresser batch --catalogue ../assets --output /tmp/all-tight --hull tight")]
    Batch {
        /// Catalogue root containing `urdf-robot-collection/` + `urdf-tool-collection/`.
        #[arg(long)]
        catalogue: PathBuf,
        /// Output directory; one subdirectory per `<robot>__<tool>` permutation.
        #[arg(long)]
        output: PathBuf,
        /// Collision hull buffer: `tight` / `medium` (default) / `max`.
        #[arg(long, default_value = "medium")]
        hull: String,
    },

    /// [CLI] Statically validate a URDF file.
    #[command(long_about = "\
[CLI] Statically validate a URDF file.

Exits 0 on clean, non-zero with a list of structured diagnostics
otherwise. No filesystem write. Use `--tool` to enable the extra
origin-alignment checks specific to tool URDFs.

Examples:
  # Validate as a robot (default)
  urdf-robo-dresser validate ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf

  # Validate as a tool (extra origin checks)
  urdf-robo-dresser validate \\
      ../assets/urdf-tool-collection/GREIFER_URW1_OP3210_R01G1/GREIFER_URW1_OP3210_R01G1.urdf \\
      --tool")]
    Validate {
        /// Path to the URDF file.
        path: PathBuf,
        /// Validate as a tool URDF (extra origin-alignment checks).
        #[arg(long)]
        tool: bool,
    },

    /// [CLI] List robots + tools discoverable in a catalogue.
    ///
    /// Useful for piping into other tools — pass `--json` for a stable
    /// machine-readable shape.
    List {
        /// Catalogue root.
        #[arg(long)]
        catalogue: PathBuf,
        /// Emit JSON instead of the default table.
        #[arg(long)]
        json: bool,
    },

    // -------------------------------------------------------------
    // Server mode — long-running HTTP service (gated on `server` feature)
    // -------------------------------------------------------------

    /// [SERVER] Boot the HTTP server: REST API at `/api/v1`, Swagger UI at `/docs`.
    #[cfg(feature = "server")]
    #[command(long_about = "\
[SERVER] Boot the HTTP server: REST API at `/api/v1`, Swagger UI at `/docs`.

Runs until killed. The dresser is headless — there is no SPA; `/`
serves a small landing page linking to `/docs`. Always available in
default builds; only absent if the binary was built with
`--no-default-features` (the slim CLI-only profile).

Routes at a glance:
  GET  /                                landing page
  GET  /docs                            Swagger UI
  GET  /openapi.json                    OpenAPI 3.x spec
  GET  /healthz, /readyz                k8s-shaped probes
  GET  /api/v1/catalogue                catalogue listing
  POST /api/v1/dress                    merge robot + tool by id
  POST /api/v1/validate                 validate a URDF by path
  POST /api/v1/batch  →  GET /jobs/:id  long-running batch")]
    Serve {
        /// TCP port.
        #[arg(long, default_value_t = 3030)]
        port: u16,
        /// Catalogue root the server scans + sandboxes API requests under.
        #[arg(long)]
        catalogue: PathBuf,
    },
}

fn parse_orient(s: &str) -> Result<[u16; 3], String> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 3 {
        return Err("orient must be x,y,z (e.g. '0,90,180')".to_string());
    }
    let mut result = [0u16; 3];
    for (i, part) in parts.iter().enumerate() {
        let deg: u16 = part.trim().parse().map_err(|_| format!("invalid degree value: {}", part))?;
        if deg % 90 != 0 || deg >= 360 {
            return Err(format!("orientation must be 0, 90, 180, or 270 — got {}", deg));
        }
        result[i] = deg;
    }
    Ok(result)
}

fn main() {
    env_logger::init();
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Command::Dress { robot, tool, output, attach_to, orient, hull, view } => {
            let orient = match parse_orient(&orient) {
                Ok(o) => o,
                Err(e) => {
                    eprintln!("error: {}", e);
                    process::exit(1);
                }
            };
            let hull_buffer = match urdf_robo_dresser::collision::HullBuffer::from_str(&hull) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("error: {}", e);
                    process::exit(1);
                }
            };
            cmd_dress(&robot, &tool, &output, attach_to, orient, hull_buffer, view)
        }
        Command::Batch { catalogue, output, hull } => {
            let hull_buffer = match urdf_robo_dresser::collision::HullBuffer::from_str(&hull) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("error: {}", e);
                    process::exit(1);
                }
            };
            cmd_batch(&catalogue, &output, hull_buffer)
        }
        Command::Validate { path, tool } => cmd_validate(&path, tool),
        Command::List { catalogue, json } => cmd_list(&catalogue, json),
        #[cfg(feature = "server")]
        Command::Serve { port, catalogue } => cmd_serve(port, &catalogue),
    };

    process::exit(exit_code);
}

fn cmd_dress(robot: &PathBuf, tool: &PathBuf, output: &PathBuf, attach_to: Option<String>, orient: [u16; 3], hull_buffer: urdf_robo_dresser::collision::HullBuffer, view: bool) -> i32 {
    use urdf_robo_dresser::{parse_urdf, validate, merge, serialize_urdf, MergeOptions, Origin};
    use urdf_robo_dresser::collision::generate_hulls;

    let base = match parse_urdf(robot) {
        Ok(r) => r,
        Err(e) => { eprintln!("error: {}", e); return 1; }
    };
    let mut tool_urdf = match parse_urdf(tool) {
        Ok(r) => r,
        Err(e) => { eprintln!("error: {}", e); return 1; }
    };

    let base_diags = validate(&base, robot, false);
    let tool_diags = validate(&tool_urdf, tool, true);

    let has_errors = print_diagnostics(&base_diags) || print_diagnostics(&tool_diags);
    if has_errors {
        return 1;
    }

    let output_dir = output.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let robot_urdf_dir = robot.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();
    let tool_urdf_dir = tool.parent().unwrap_or(std::path::Path::new(".")).to_path_buf();

    // Generate convex hull collisions for tool (replaces existing)
    if let Err(e) = generate_hulls(&mut tool_urdf, &tool_urdf_dir, &output_dir, "tool", hull_buffer) {
        eprintln!("warning: tool collision hull generation failed: {}", e);
    }

    // Generate convex hull collisions for robot (replaces existing)
    let mut base_with_collision = base.clone();
    if let Err(e) = generate_hulls(&mut base_with_collision, &robot_urdf_dir, &output_dir, "robot", hull_buffer) {
        eprintln!("warning: robot collision hull generation failed: {}", e);
    }

    let opts = MergeOptions {
        attach_to,
        orient,
        offset: Origin::default(),
        tool_prefix: "tool__".to_string(),
        output_dir: output_dir.clone(),
        robot_urdf_dir: robot_urdf_dir.clone(),
        tool_urdf_dir,
    };

    let dressed = match merge(&base_with_collision, &tool_urdf, &opts) {
        Ok(d) => d,
        Err(e) => { eprintln!("error: {}", e); return 1; }
    };

    let xml = serialize_urdf(&dressed);
    if let Err(e) = std::fs::write(output, &xml) {
        eprintln!("error writing output: {}", e);
        return 1;
    }

    println!("Dressed URDF written to {}", output.display());

    // Open native viewer if requested
    if view {
        println!("Opening urdf-viz...");
        let _ = std::process::Command::new("urdf-viz")
            .arg(output)
            .status();
    }

    0
}

fn cmd_validate(path: &PathBuf, is_tool: bool) -> i32 {
    use urdf_robo_dresser::{parse_urdf, validate};

    let robot = match parse_urdf(path) {
        Ok(r) => r,
        Err(e) => { eprintln!("error: {}", e); return 1; }
    };

    let diags = validate(&robot, path, is_tool);
    if print_diagnostics(&diags) {
        1
    } else {
        println!("Valid URDF: {} links, {} joints", robot.links.len(), robot.joints.len());
        0
    }
}

fn cmd_batch(catalogue: &PathBuf, output: &PathBuf, hull_buffer: urdf_robo_dresser::collision::HullBuffer) -> i32 {
    use urdf_robo_dresser::{parse_urdf, merge, serialize_urdf, MergeOptions, Origin};
    use urdf_robo_dresser::collision::generate_hulls;

    let robot_dir = catalogue.join("urdf-robot-collection");
    let tool_dir = catalogue.join("urdf-tool-collection");

    if !robot_dir.exists() {
        eprintln!("error: robot collection not found at {}", robot_dir.display());
        return 1;
    }
    if !tool_dir.exists() {
        eprintln!("error: tool collection not found at {}", tool_dir.display());
        return 1;
    }

    // Discover robots (each in its own subdirectory: <name>/<name>.urdf)
    let robots: Vec<_> = std::fs::read_dir(&robot_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let urdf = e.path().join(format!("{}.urdf", name));
            urdf.exists().then_some(urdf)
        })
        .collect();

    // Discover tools (each in its own subdirectory)
    let tools: Vec<_> = std::fs::read_dir(&tool_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let urdf = e.path().join(format!("{}.urdf", name));
            urdf.exists().then_some(urdf)
        })
        .collect();

    if robots.is_empty() {
        eprintln!("error: no robot URDFs found in {}", robot_dir.display());
        return 1;
    }
    if tools.is_empty() {
        eprintln!("error: no tool URDFs found in {}", tool_dir.display());
        return 1;
    }

    println!("Found {} robot(s) and {} tool(s)", robots.len(), tools.len());

    let mut success = 0;
    let mut failed = 0;

    for robot_path in &robots {
        let base = match parse_urdf(robot_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("  skip robot {}: {}", robot_path.display(), e);
                failed += 1;
                continue;
            }
        };
        let robot_name = robot_path.file_stem().unwrap().to_string_lossy().to_string();

        for tool_path in &tools {
            let tool = match parse_urdf(tool_path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("  skip tool {}: {}", tool_path.display(), e);
                    failed += 1;
                    continue;
                }
            };
            let tool_name = tool_path.parent().unwrap().file_name().unwrap().to_string_lossy().to_string();
            let combo_name = format!("{}__{}", robot_name, tool_name);
            let combo_dir = output.join(&combo_name);

            std::fs::create_dir_all(&combo_dir).unwrap();

            let tool_urdf_dir = tool_path.parent().unwrap().to_path_buf();
            let robot_urdf_dir = robot_path.parent().unwrap().to_path_buf();

            // Generate convex hull collisions for tool
            let mut tool_with_hulls = tool.clone();
            if let Err(e) = generate_hulls(&mut tool_with_hulls, &tool_urdf_dir, &combo_dir, "tool", hull_buffer) {
                eprintln!("  warning: tool hull generation failed for {}: {}", tool_name, e);
            }

            // Generate convex hull collisions for robot
            let mut base_with_hulls = base.clone();
            if let Err(e) = generate_hulls(&mut base_with_hulls, &robot_urdf_dir, &combo_dir, "robot", hull_buffer) {
                eprintln!("  warning: robot hull generation failed for {}: {}", robot_name, e);
            }

            let opts = MergeOptions {
                attach_to: None,
                orient: [0, 0, 0],
                offset: Origin::default(),
                tool_prefix: "tool__".to_string(),
                output_dir: combo_dir.clone(),
                robot_urdf_dir,
                tool_urdf_dir,
            };

            match merge(&base_with_hulls, &tool_with_hulls, &opts) {
                Ok(dressed) => {
                    let xml = serialize_urdf(&dressed);
                    let out_file = combo_dir.join(format!("{}.urdf", combo_name));
                    std::fs::write(&out_file, &xml).unwrap();
                    println!("  {} -> {}", combo_name, out_file.display());
                    success += 1;
                }
                Err(e) => {
                    eprintln!("  FAILED {}: {}", combo_name, e);
                    failed += 1;
                }
            }
        }
    }

    println!("\nBatch complete: {} success, {} failed", success, failed);
    if failed > 0 { 1 } else { 0 }
}

fn cmd_list(catalogue: &PathBuf, json: bool) -> i32 {
    use urdf_robo_dresser::catalogue;

    let cat = match catalogue::scan(catalogue) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error scanning catalogue: {}", e);
            return 1;
        }
    };

    if cat.robots.is_empty() && cat.tools.is_empty() {
        eprintln!("No assets found in {}", catalogue.display());
        return 1;
    }

    if json {
        print!("{}", cat.to_json());
    } else {
        cat.print_table();
    }

    0
}

#[cfg(feature = "server")]
fn cmd_serve(port: u16, catalogue: &PathBuf) -> i32 {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: tokio runtime build: {}", e);
            return 1;
        }
    };
    match rt.block_on(urdf_robo_dresser::server::run(
        "127.0.0.1",
        port,
        catalogue.clone(),
    )) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("error: {}", e);
            1
        }
    }
}

fn print_diagnostics(diags: &[urdf_robo_dresser::Diagnostic]) -> bool {
    use urdf_robo_dresser::Severity;
    let mut has_errors = false;
    for d in diags {
        let prefix = match d.severity {
            Severity::Error => { has_errors = true; "error" }
            Severity::Warning => "warning",
        };
        let loc = match &d.location {
            Some(l) => format!(" [{}:{}]", l.file.display(), l.element),
            None => String::new(),
        };
        eprintln!("{} [{}]{}: {}", prefix, d.code, loc, d.message);
    }
    has_errors
}
