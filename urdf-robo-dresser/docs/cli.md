# CLI reference

`urdf-robo-dresser` ships four CLI subcommands plus the REST server.
All five share the same library functions — there is no behaviour in
the server that the CLI cannot also reach.

```
$ urdf-robo-dresser --help
urdf-robo-dresser — robot+tool URDF merger (CLI + REST server)

  CLI (one-shot):    dress / batch / validate / list against local files.
  Server (long-run): host the same operations as a versioned REST API.

Commands:
  dress     [CLI] Merge ONE robot URDF with ONE tool URDF and write the dressed URDF to disk
  batch     [CLI] Permutation-merge every robot × every tool in a catalogue
  validate  [CLI] Statically validate a URDF file
  list      [CLI] List robots + tools discoverable in a catalogue
  serve     [SERVER] Boot the HTTP server: REST API at /api/v1, Swagger UI at /docs
```

`urdf-robo-dresser <cmd> --help` drills into per-flag detail with
copy-pasteable examples.

The REST `serve` mode is documented separately in [`api.md`](api.md).

---

## `dress` — merge a robot with a tool

The core operation. Parses both URDFs, auto-detects the flange link,
computes the correct tool orientation, generates convex-hull
collisions, writes a self-contained dressed URDF.

```sh
urdf-robo-dresser dress \
    --robot   <ROBOT_URDF> \
    --tool    <TOOL_URDF> \
    --output  <OUTPUT_URDF> \
    [--orient X,Y,Z] \
    [--hull tight|medium|max] \
    [--attach-to LINK_NAME] \
    [--view]
```

| Flag | Default | Description |
|---|---|---|
| `--robot` | required | Robot base URDF |
| `--tool` | required | Tool URDF to attach |
| `--output` | required | Dressed URDF path; meshes land beside it |
| `--orient` | `0,0,0` (= auto) | Override tool orientation (degrees in 90° steps). When all zero, the auto-orient algorithm runs |
| `--hull` | `medium` | Collision-hull buffer. `tight` = exact, `medium` = +5%, `max` = +15% |
| `--attach-to` | auto | Override flange-link detection. Default = deepest leaf of the kinematic chain |
| `--view` | off | Open `urdf-viz` after dressing for visual inspection |

### Pipeline

1. Parse robot + tool URDFs (typed kinematic trees)
2. Validate both (cyclic graph check, dangling refs, missing meshes)
3. Auto-detect flange link (deepest leaf in the kinematic chain) unless `--attach-to` is set
4. Generate per-component convex hulls for both robot + tool
5. Score 64 candidate orientations; pick the best
6. Inject tool geometry into the flange link with the chosen rotation
7. Copy meshes (visual + hulls) into the output directory
8. Write deterministic URDF with rewritten mesh paths

### Worked examples

```sh
# Single dress, auto-orient
urdf-robo-dresser dress \
    --robot ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf \
    --tool  ../assets/urdf-tool-collection/gripper_tool/gripper_tool.urdf \
    --output /tmp/kr240_gripper.urdf

# Force orientation + flange link
urdf-robo-dresser dress \
    --robot ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf \
    --tool  ../assets/urdf-tool-collection/rivet_tool/rivet_tool.urdf \
    --output /tmp/kr240_riveter.urdf \
    --attach-to like_kr240r2900_link_6 \
    --orient 0,90,0

# Generous collision margin (safety zones / low-quality mesh inputs)
urdf-robo-dresser dress \
    --robot ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf \
    --tool  ../assets/urdf-tool-collection/gripper_tool/gripper_tool.urdf \
    --output /tmp/kr240_gripper_safe.urdf \
    --hull max --view
```

### Output layout

```
/tmp/kr240_gripper.urdf
/tmp/meshes/
    robot/
        like_kr240r2900_*_visuals_*.obj
        like_kr240r2900_*_hull_0.obj
    tool/
        World_*.obj
        World_hull_*.obj
```

All paths in the URDF are relative to the URDF file location, so the
output directory can be moved or zipped without breaking references.

---

## `batch` — every robot × every tool

Walks `<catalogue>/urdf-robot-collection/` × `urdf-tool-collection/`
and writes one dressed URDF per pair under `<output>/<robot>__<tool>/`.
Continues past individual failures and prints a final tally.

```sh
urdf-robo-dresser batch \
    --catalogue <CATALOGUE_DIR> \
    --output    <OUTPUT_DIR> \
    [--hull tight|medium|max]
```

Catalogue layout — each robot and each tool lives in its own
directory matching its URDF stem:

```
<catalogue>/
├── urdf-robot-collection/
│   ├── <robot_a>/
│   │   ├── <robot_a>.urdf
│   │   └── meshes/
│   └── <robot_b>/...
└── urdf-tool-collection/
    ├── <tool_1>/
    │   ├── <tool_1>.urdf
    │   └── meshes/
    └── <tool_2>/...
```

Output:

```
<output>/
├── <robot_a>__<tool_1>/
│   ├── <robot_a>__<tool_1>.urdf
│   └── meshes/
├── <robot_a>__<tool_2>/...
└── <robot_b>__<tool_1>/...
```

### Worked example

```sh
urdf-robo-dresser batch \
    --catalogue ../assets \
    --output    ../assets/urdf-dressed-robots
```

```
Found 1 robot(s) and 4 tool(s)
[INFO] Auto-orient: picked [270, 180, 270] (normal_dot=0.935)
  kr240r2900__gripper_tool_a -> .../kr240r2900__gripper_tool_a.urdf
[INFO] Auto-orient: picked [90, 0, 180] (normal_dot=0.840)
  kr240r2900__gripper_tool_b -> .../kr240r2900__gripper_tool_b.urdf
[INFO] Auto-orient: picked [0, 270, 180] (normal_dot=0.840)
  kr240r2900__gripper_tool_c -> .../kr240r2900__gripper_tool_c.urdf
[INFO] Auto-orient: picked [90, 180, 90] (normal_dot=0.910)
  kr240r2900__gripper_tool_d -> .../kr240r2900__gripper_tool_d.urdf

Batch complete: 4 success, 0 failed
```

This is the recommended way to populate the
`urdf-dressed-robots/` catalogue that
[`robo-fitting-room`](../../robo-fitting-room/) consumes.

---

## `validate` — static URDF check

```sh
urdf-robo-dresser validate <URDF_PATH> [--tool]
```

Exit `0` on clean, non-zero with structured diagnostics otherwise. No
filesystem writes — safe in CI on every URDF change.

| Check | Triggered for |
|---|---|
| Acyclic kinematic tree | both |
| Single root link | both |
| All parent/child refs resolve | both |
| No duplicate link/joint names | both |
| Joint limits consistency (`lower < upper`) | both |
| Mesh-file existence (`.obj` / `.stl`) | both |
| Visual-without-collision warning | both |
| Origin alignment at `0,0,0` (flange center) | tool only (`--tool`) |

```sh
# Robot URDF
urdf-robo-dresser validate \
    ../assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf
# Valid URDF: 8 links, 7 joints

# Tool URDF (extra origin checks)
urdf-robo-dresser validate \
    ../assets/urdf-tool-collection/gripper_tool/gripper_tool.urdf \
    --tool
# Valid URDF: 1 links, 0 joints
```

---

## `list` — discover the catalogue

```sh
urdf-robo-dresser list --catalogue <DIR> [--json]
```

Default output is a table with two sections (robots, tools). With
`--json`, emits a stable shape suitable for piping into other tools:

```json
{
  "robots": [{"id": "kr240r2900", "urdf": "<abs path>", "links": 8, "joints": 7}],
  "tools":  [{"id": "gripper_tool", "urdf": "<abs path>", "links": 1, "joints": 0}]
}
```

Useful for shelling out from CI or another binary that needs to know
what the catalogue contains before issuing dress requests.

---

## `serve` — REST + Swagger UI

```sh
urdf-robo-dresser serve --catalogue <DIR> [--port <PORT>]
```

| Flag | Default | Description |
|---|---|---|
| `--catalogue` | required | Catalogue root the server scans + sandboxes API requests under |
| `--port` | `3030` | TCP port |

Available in default builds (`server` is a default Cargo feature).
Absent only with `--no-default-features`. See [`api.md`](api.md) for
the full REST surface.

---

## Library API

Both transports (CLI and REST) call the same lib functions:

```rust
use urdf_robo_dresser::{
    parse_urdf, validate, merge, serialize_urdf,
    MergeOptions, Origin,
    collision::{generate_hulls, HullBuffer},
};

let mut robot = parse_urdf("robot.urdf".as_ref())?;
let mut tool  = parse_urdf("tool.urdf".as_ref())?;

let _diags_robot = validate(&robot, "robot.urdf".as_ref(), false);
let _diags_tool  = validate(&tool,  "tool.urdf".as_ref(),  true);

let out_dir = std::path::PathBuf::from("/tmp");
generate_hulls(&mut robot, "robot/".as_ref(), &out_dir, "robot", HullBuffer::Medium)?;
generate_hulls(&mut tool,  "tool/".as_ref(),  &out_dir, "tool",  HullBuffer::Medium)?;

let opts = MergeOptions {
    attach_to: None,
    orient: [0, 0, 0],
    offset: Origin::default(),
    tool_prefix: "tool__".to_string(),
    output_dir: out_dir.clone(),
    robot_urdf_dir: "robot/".into(),
    tool_urdf_dir:  "tool/".into(),
};
let dressed = merge(&robot, &tool, &opts)?;
std::fs::write("/tmp/dressed.urdf", serialize_urdf(&dressed))?;
```

## Environment variables

| Variable | Effect |
|---|---|
| `RUST_LOG=info` | Show auto-orient picks and hull generation progress |
| `RUST_LOG=debug` | Show tool direction vectors, flange-detection details |

## Testing

```sh
cargo test                       # parser round-trip, validators, merge correctness
cargo test --features server     # also handler smoke + OpenAPI snapshot
cargo test --test requirements   # FR/NFR mapping
```
