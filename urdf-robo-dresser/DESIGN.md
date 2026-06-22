# Design: `urdf-robo-dresser`

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        urdf-robo-dresser                        │
├──────────────────────────┬──────────────────────────────────────┤
│      CLI (main.rs)       │      Server (feature="server")       │
│  clap subcommands        │  axum REST + embedded SPA            │
├──────────────────────────┴──────────────────────────────────────┤
│                      Core Library (lib.rs)                      │
│  ┌──────────┐ ┌───────────┐ ┌────────┐ ┌──────────┐ ┌───────┐   │
│  │ parser   │→│ validator │→│ merger │→│serializer│→│output │   │
│  └──────────┘ └───────────┘ └────────┘ └──────────┘ └───────┘   │
│  ┌───────────┐ ┌───────────────┐                                │
│  │ catalogue │ │ collision_gen │                                │
│  └───────────┘ └───────────────┘                                │
├─────────────────────────────────────────────────────────────────┤
│               Viewer (feature="viewer")                         │
│  CLI: urdf-viz (native window)  │  Server: WebGL (browser)      │
└─────────────────────────────────────────────────────────────────┘
```

---

## Data Flow

```
Input:                                    Output:
  base.urdf ──┐                             dressed.urdf
  tool.urdf ──┼──→ parse → validate ──┐     (robot tree unchanged,
  config    ──┘                       │      tool subtree attached
                                      ▼      at flange center)
                                    merge ──→ serialize ──→ disk
                                      │
                                      ├──→ collision_gen (if missing <collision>)
                                      └──→ viewer (optional --view / web UI)
```

---

## Core Types

```rust
pub struct UrdfRobot {
    pub name: String,
    pub links: Vec<Link>,
    pub joints: Vec<Joint>,
}

pub struct Link {
    pub name: String,
    pub inertial: Option<Inertial>,
    pub visuals: Vec<Visual>,
    pub collisions: Vec<Collision>,
}

pub struct Joint {
    pub name: String,
    pub joint_type: JointType,
    pub parent: String,
    pub child: String,
    pub origin: Origin,
    pub axis: [f64; 3],
    pub limit: Option<JointLimit>,
}

pub struct Origin {
    pub xyz: [f64; 3],
    pub rpy: [f64; 3],
}

pub struct Visual {
    pub origin: Origin,
    pub geometry: Geometry,
}

pub struct Collision {
    pub origin: Origin,
    pub geometry: Geometry,
}

pub enum Geometry {
    Mesh { filename: String, scale: [f64; 3] },
    Box { size: [f64; 3] },
    Cylinder { radius: f64, length: f64 },
    Sphere { radius: f64 },
}

pub enum JointType {
    Revolute, Continuous, Prismatic, Fixed, Floating, Planar,
}

pub struct JointLimit {
    pub lower: f64,
    pub upper: f64,
    pub effort: f64,
    pub velocity: f64,
}

pub struct Inertial {
    pub origin: Origin,
    pub mass: f64,
    pub inertia: InertiaMatrix,
}
```

---

## Module Responsibilities

### `parser.rs`

Streaming XML → `UrdfRobot`. Uses `quick-xml` event reader with state machine.

- Input: `&[u8]` or `&Path`
- Output: `Result<UrdfRobot, DresserError>`
- Preserves all data losslessly (round-trip guarantee)

### `validator.rs`

Checks on a parsed `UrdfRobot`:

1. **Tree structure** — single root, acyclic, all parent/child refs resolve
2. **Duplicates** — no repeated link or joint names
3. **Joint limits** — `lower < upper` for bounded types
4. **Mesh existence** — all `Geometry::Mesh` paths resolve relative to URDF location
5. **Tool origin** — root link centroid within tolerance of (0,0,0) for tool URDFs
6. **Collision coverage** — flag links with visuals but no collision (pre-merge warning)

Returns `Vec<Diagnostic>` (errors halt, warnings don't).

### `merger.rs`

Combines base + tool into dressed output. Invariant: **robot tree is immutable**.

```
merge(base: &UrdfRobot, tool: &UrdfRobot, opts: &MergeOptions) -> Result<UrdfRobot>
```

Steps:
1. Identify flange link (auto-detect leaf or use `opts.attach_to`)
2. Compute attachment origin = flange outermost centerpoint in flange-local frame
3. Apply 90° orientation steps from `opts.orient` (default 0/0/0)
4. Apply optional additional offset from `opts.offset`
5. Discard tool's `World` link, re-parent children
6. Namespace-prefix all tool link/joint names (`tool__` prefix)
7. Rewrite tool mesh paths relative to output location
8. Create fixed joint: parent=flange, child=tool root, origin=computed
9. Concatenate: base links/joints + new joint + tool links/joints
10. Run collision_gen on any links still missing `<collision>`

```rust
pub struct MergeOptions {
    pub attach_to: Option<String>,       // override flange link
    pub orient: [u16; 3],                // degrees, multiples of 90 only
    pub offset: Origin,                  // additional 6-DOF offset
    pub tool_prefix: String,             // default "tool__"
    pub output_dir: PathBuf,             // for mesh path rewriting
}
```

### `collision.rs`

Generates convex hull collision geometry for links that have visuals but no `<collision>`.

- Reads mesh file (OBJ/STL) vertices
- Computes convex hull (gift-wrapping or quickhull)
- Writes hull as new STL file to output directory
- Returns `Collision` element referencing the generated file

### `serializer.rs`

`UrdfRobot` → deterministic XML bytes.

- Direct `write!` to `Vec<u8>` (no intermediate DOM)
- Fixed element ordering: links first, joints second (stable output)
- Numeric formatting: sufficient precision, no trailing zeros

### `catalogue.rs`

Discovers and indexes assets from directory structure.

```
scan(root: &Path) -> Result<Catalogue>
```

```rust
pub struct Catalogue {
    pub robots: Vec<RobotEntry>,
    pub tools: Vec<ToolEntry>,
}

pub struct RobotEntry {
    pub name: String,
    pub path: PathBuf,
    pub flange_link: String,
    pub status: AssetStatus,  // Valid | Incomplete(Vec<String>)
}

pub struct ToolEntry {
    pub name: String,
    pub path: PathBuf,
    pub status: AssetStatus,
}
```

### `server/` (feature-gated)

- `mod.rs` — axum router setup, static file serving, CORS
- `api.rs` — REST handlers delegating to core library functions
- `ui.rs` — `rust-embed` SPA with WebGL viewer

Endpoints:

| Method | Path | Action |
|--------|------|--------|
| GET | `/api/catalogue` | List robots + tools |
| POST | `/api/validate` | Validate a URDF (upload or path) |
| POST | `/api/dress` | Merge robot+tool, return dressed URDF metadata |
| POST | `/api/batch` | Trigger batch export |
| GET | `/api/dressed/:id` | Download dressed URDF file |
| GET | `/api/dressed/:id/meshes/*` | Serve mesh files for WebGL viewer |
| GET | `/` | SPA entry point |

### `viewer/` (feature-gated)

- `native.rs` — spawns `urdf-viz` with the dressed URDF path, blocks until window closed
- `webgl.rs` — Three.js/Babylon.js URDF loader embedded in SPA, renders meshes in browser with joint sliders

---

## Merge Algorithm Detail

The tool's geometry is injected directly into the robot's end-effector (flange) link.
No extra links or joints are created — the tool becomes part of the flange link's geometry.
This produces a simpler URDF (same kinematic tree as the base robot) suitable for simulation.

```
1.  base = parse(robot.urdf)
2.  tool = parse(tool.urdf)
3.  validate(base)  → abort on error
4.  validate(tool)  → abort on error
5.  flange = resolve_flange(base, opts.attach_to)
6.  flange_axis = joint_driving_flange.axis
7.  flange_face_offset = max_projection(flange_link_mesh_vertices, flange_axis)
8.  generate_hulls(tool, tool_urdf_dir, "tool", hull_buffer)   // per-component convex hulls
9.  generate_hulls(base, robot_urdf_dir, "robot", hull_buffer)
10. ── AUTO-ORIENTATION (3-stage, when --orient not specified) ──
    For each of 64 candidate rotations (0°/90°/180°/270° × 3 axes):
      a) normal_dot: dot(rotate(tool_direction, candidate), flange_outward)
      b) centroid_outward: tool centroid further from base than flange point
      c) intersection_count: tool vertices inside robot AABB
    Pick: highest normal_dot, tiebreak by centroid_outward > 0, then lowest intersection
11. tool_transform = Origin {
       xyz: flange_axis * flange_face_offset,
       rpy: orient_to_rpy(best_orientation),
    }
12. Collect tool visuals/collisions, rotate as rigid body around origin (0,0,0)
    using tool_transform (matrix composition for rotation, pivot at tool origin)
13. Inject transformed tool visuals/collisions into flange link
14. Copy all meshes (robot + tool + hulls + MTL files) to output/meshes/{robot,tool}/
15. Rewrite all mesh paths to relative ./meshes/{robot,tool}/{filename}
16. dressed = UrdfRobot {
       name: "{base.name}__{tool.name}",
       links: base.links (with tool geometry in flange link),
       joints: base.joints (unchanged),
    }
17. output_bytes = serialize(dressed)
```

---

## Flange Detection

When no explicit `--attach-to` is given:

1. Build adjacency: for each joint, `parent → child`
2. Find all leaf links (links that appear as child but never as parent in any joint)
3. For each leaf, compute chain depth (hops to root)
4. Flange = leaf with maximum depth (deepest in kinematic chain)
5. Tie-break: alphabetical link name (deterministic)

---

## 90° Orientation

Only multiples of 90 accepted. Internally stored as `[u16; 3]` in degrees.

Conversion to rpy radians for the attachment joint:

```rust
fn orient_to_rpy(orient: [u16; 3]) -> [f64; 3] {
    orient.map(|deg| (deg as f64).to_radians())
}
```

Validation rejects values not in `{0, 90, 180, 270}`.

---

## Auto-Orientation (Pre-Merge)

Automatically determines the correct tool mounting orientation. Tries all 64 axis-aligned rotations
(0°/90°/180°/270° on each of X/Y/Z) and picks the best using a 3-stage scoring pipeline.

Runs when `--orient` is not specified (or set to 0,0,0). Skipped when user provides explicit orientation.

### Stage 1: Direction Alignment (primary score)

The tool's "pointing direction" (vector from origin 0,0,0 to centroid of all visual origin positions)
is rotated by each candidate rotation, then dotted with the flange outward direction.

- **Flange outward** = last joint's rotation axis transformed through the FK chain at home pose
- **Tool direction** = normalized centroid of visual origin xyz values
- **Score** = `dot(rotated_tool_direction, flange_outward)` → higher is better

### Stage 2: Centroid Outward Check (filter)

After candidate rotation, the tool's centroid should be further from the robot base than the flange point.

- `centroid_outward = dist(base, rotated_tool_centroid) - dist(base, flange_point)`
- Positive → tool extends outward (correct)
- Negative → tool folds back into arm (wrong)

### Stage 3: AABB Intersection (tiebreaker)

Count how many tool mesh vertices (after candidate rotation + flange offset) fall inside the robot's
axis-aligned bounding box. Fewer intersections = better.

### Scoring Logic

```
1. Sort all 64 candidates by highest normal_dot
2. Among top candidates (within 0.05 of best):
   - Prefer centroid_outward > 0
   - Final tiebreak: lowest intersection count
3. If no candidate passes stages 1+2: fall back to highest normal_dot overall
```

### Key Design Choices

- Rotation pivot is tool origin (0,0,0) — the docking flange center
- Rotation uses matrix multiplication (not Euler angle addition) for correct rigid-body behavior
- All tool visuals/collisions rotate as one group around the shared pivot
- No user input required — fully automatic for well-formed tool URDFs

---

## Convex Hull Generation

Generated for every visual mesh component in both robot and tool. Replaces all existing
collision geometry with per-component convex hulls. Runs before merge.

1. For each visual element in each link, read OBJ mesh vertices
2. Scale vertices outward from centroid by buffer factor (tight=1.0, medium=1.05, max=1.15)
3. Compute convex hull using quickhull algorithm (parry3d)
4. Write hull as OBJ file to `{output_dir}/meshes/{robot|tool}/{link}_hull_{i}.obj`
5. Replace link's `<collision>` elements with hull references (same origin as visual)

Buffer modes controlled via `--hull` CLI flag:
- `tight` — exact mesh boundary (scale 1.0×)
- `medium` — 5% inflation (default)
- `max` — 15% inflation for safety margins

---

## Incremental Build (Batch Mode)

Each combination keyed by: `SHA256(robot_urdf_bytes || tool_urdf_bytes || merge_options_json)`.

State file: `{output_dir}/.dresser-state.json`

```json
{
  "kr240__gripper_tool": {
    "hash": "a1b2c3...",
    "output": "kr240__gripper_tool.urdf",
    "timestamp": "2026-05-26T12:00:00Z"
  }
}
```

On batch run: skip entries whose hash matches current inputs.

---

## Feature Flags

```toml
[features]
default = []
server = ["tokio", "axum", "tower-http", "rust-embed"]
viewer = ["urdf-viz"]
full = ["server", "viewer"]
```

Core library compiles with zero optional deps. `cargo build` gives CLI-only binary.
`cargo build --features full` gives CLI + server + native viewer.

---

## Error Strategy

All public functions return `Result<T, DresserError>`. Validation returns `Vec<Diagnostic>`:

```rust
pub struct Diagnostic {
    pub severity: Severity,  // Error | Warning
    pub code: &'static str,  // e.g. "E011" maps to FR-011
    pub message: String,
    pub location: Option<DiagLocation>,
}

pub struct DiagLocation {
    pub file: PathBuf,
    pub element: String,  // e.g. "joint 'joint_a3'"
}
```

CLI formats diagnostics as colored terminal output. Server returns them as JSON array.
