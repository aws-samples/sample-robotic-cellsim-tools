use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

const BEFORE_HELP: &str = "\
robo-fitting-room — manifest-driven scene composition + 2D scene editor
=======================================================================
Two ways to use this tool:

  CLI (one-shot):    validate / compose a manifest, exit.
  Server (long-run): host the SPA + REST API, drive Isaac Sim.

Both transports call the same library functions — there is no logic
behind the server that the CLI cannot also reach.";

const AFTER_HELP: &str = "\
Quick start
-----------
  # one-shot CLI
  robo-fitting-room validate ../assets/fitted-robots/k8urw11-cell.yaml
  robo-fitting-room compose -m ../assets/fitted-robots/k8urw11-cell.yaml -o /tmp/cell.zip

  # long-running server (default build includes it)
  robo-fitting-room serve --asset-root ../assets
  open http://localhost:3040/         # 2D scene editor (SPA)
  open http://localhost:3040/docs     # Swagger UI

The default build ships both modes. Pass `--no-default-features` to
cargo to produce the slim CLI-only binary without axum/tokio/utoipa
or the embedded SPA bundle.
See `<command> --help` for full per-command flag listings.";

#[derive(Parser)]
#[command(
    name = "robo-fitting-room",
    version,
    about = "Manifest-driven robotic-cell scene composer (CLI + REST server)",
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

    /// [CLI] Validate a scene manifest (YAML structure + URDF paths + placement values).
    ///
    /// Exits 0 on success, non-zero with diagnostics otherwise. No
    /// network or filesystem write — purely a static check. (FR-141.)
    Validate {
        /// Manifest YAML to validate.
        manifest: PathBuf,
    },

    /// [CLI] Compose a manifest into a USDA stage zip ready to load in Isaac Sim.
    ///
    /// Reads the YAML, walks every referenced URDF, emits one inlined
    /// `.usda` + the manifest into a self-contained `.zip` package.
    /// No server, no network. (FR-140.)
    Compose {
        /// Manifest YAML.
        #[arg(short, long)]
        manifest: PathBuf,
        /// Output `.zip` path (will be created/overwritten).
        #[arg(short, long)]
        output: PathBuf,
        /// Target backend. Today only `isaac-sim` is implemented; MuJoCo /
        /// Gazebo are tracked in REQUIREMENTS.md (BL-04 / BL-05).
        #[arg(long, default_value = "isaac-sim")]
        backend: String,
        /// Keep full robot visual meshes. Default `false` drops robot
        /// visuals and surfaces collision hulls in the viewport — leaner
        /// zip, faster Isaac load, identical physics. Fixtures always
        /// emit at full fidelity regardless of this flag.
        #[arg(long, default_value_t = false)]
        full_robots: bool,
        /// Keep full visual meshes for tools (default `true`). Pass
        /// `--full-tools=false` to surface tool hulls only. Tool meshes
        /// are detected by the `meshes/tool/` path prefix that
        /// urdf-robo-dresser writes; independent of `--full-robots`.
        #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
        full_tools: bool,
    },

    // -------------------------------------------------------------
    // Server mode — long-running HTTP service (gated on `server` feature)
    // -------------------------------------------------------------

    /// [SERVER] Boot the HTTP server: 2D editor SPA at `/`, REST API at `/api/v1`, Swagger UI at `/docs`.
    #[cfg(feature = "server")]
    #[command(long_about = "\
[SERVER] Boot the HTTP server: 2D editor SPA at `/`, REST API at `/api/v1`, Swagger UI at `/docs`.

Runs until killed. The SPA, the OpenAPI spec, and every
`/api/v1/*` route call the same library functions the CLI does.
Requires the binary to be built with `--features server`.

Routes at a glance:
  GET  /                    2D scene-layout editor (SPA)
  GET  /docs                Swagger UI
  GET  /openapi.json        OpenAPI 3.x spec
  GET  /healthz, /readyz    k8s-shaped probes
  *    /api/v1/...          versioned REST surface")]
    Serve {
        /// Bind host. Use `0.0.0.0` to listen on all interfaces.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// TCP port.
        #[arg(long, default_value_t = 3040)]
        port: u16,
        /// Root directory containing `urdf-dressed-robots/`,
        /// `urdf-fixture-collection/`, `floor-plans/`,
        /// `fitted-robots/`. Every `?path=` API call is sandboxed
        /// under this — paths with `..` segments are rejected.
        #[arg(long)]
        asset_root: PathBuf,
        /// Override the embedded SPA bundle with files from disk —
        /// useful when iterating on the editor's JS without rebuilding
        /// the binary. Defaults to the bundle baked at compile time.
        #[arg(long)]
        web_root: Option<PathBuf>,
    },
}

fn main() {
    // Default log level = info; users override with RUST_LOG=debug etc.
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();

    match cli.command {
        Command::Validate { manifest } => {
            let scene = match robo_fitting_room::parse_scene(&manifest) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                    process::exit(1);
                }
            };

            println!(
                "Scene: \"{}\" — {} robots, {} fixtures, floor_plan: {}",
                scene.name,
                scene.robots.len(),
                scene.fixtures.len(),
                if scene.floor_plan.is_some() { "yes" } else { "no" },
            );

            let errors = robo_fitting_room::validate_scene(&scene);
            if errors.is_empty() {
                println!("OK: all checks passed");
            } else {
                for e in &errors {
                    eprintln!("  ERROR: {}", e);
                }
                process::exit(1);
            }
        }
        Command::Compose {
            manifest,
            output,
            backend,
            full_robots,
            full_tools,
        } => {
            if backend != "isaac-sim" {
                eprintln!("ERROR: backend '{}' not yet supported (only 'isaac-sim')", backend);
                process::exit(1);
            }

            let scene = match robo_fitting_room::parse_scene(&manifest) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                    process::exit(1);
                }
            };

            let errors = robo_fitting_room::validate_scene(&scene);
            if !errors.is_empty() {
                for e in &errors {
                    eprintln!("  ERROR: {}", e);
                }
                process::exit(1);
            }

            println!(
                "Composing scene \"{}\" → {} ...",
                scene.name,
                output.display()
            );
            for r in &scene.robots {
                println!(
                    "  Robot {}: dressed_urdf = {}",
                    r.robot_id,
                    r.dressed_urdf
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                );
            }
            for f in &scene.fixtures {
                println!(
                    "  Fixture {}: urdf = {}",
                    f.id,
                    f.urdf
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                );
            }

            if full_robots {
                println!("  Robots: full visual fidelity");
            } else {
                println!("  Robots: collision hulls only (use --full-robots for visuals)");
            }
            if full_tools {
                println!("  Tools: full visual fidelity");
            } else {
                println!("  Tools: collision hulls only (--full-tools=false)");
            }

            let opts = robo_fitting_room::ComposeOptions {
                full_robots,
                full_tools,
            };
            match robo_fitting_room::compose_stage_with_options(&scene, &manifest, &output, &opts) {
                Ok(report) => {
                    println!(
                        "Written: {} ({} robots, {} fixtures, {} floor-plan segments)",
                        report.zip_path.display(),
                        report.robot_count,
                        report.fixture_count,
                        report.floor_plan_segments,
                    );
                    println!(
                        "  Inline meshes: {} ({} vertices, {} faces)",
                        report.mesh_count, report.total_vertices, report.total_faces,
                    );
                    println!(
                        "  Zip: {} bytes (usda: {} bytes)",
                        report.zip_bytes, report.usda_bytes,
                    );
                }
                Err(e) => {
                    eprintln!("ERROR: {}", e);
                    process::exit(1);
                }
            }
        }
        #[cfg(feature = "server")]
        Command::Serve {
            host,
            port,
            asset_root,
            web_root,
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
            if let Err(e) = rt.block_on(robo_fitting_room::server::run(
                &host, port, asset_root, web_root,
            )) {
                eprintln!("ERROR: {}", e);
                process::exit(1);
            }
        }
    }
}
