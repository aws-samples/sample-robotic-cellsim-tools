# Concepts

Domain vocabulary used across the dresser codebase, the catalogue
layout, and the REST API. If you just want to use the tool,
[`cli.md`](cli.md) and [`api.md`](api.md) are more direct. Come here
when you need the ground truth on naming, scoring, or merge
semantics.

## Catalogue layout

The catalogue is a structured directory tree that holds every URDF
the dresser can consume or produce:

```
<catalogue>/
├── urdf-robot-collection/        # input: robot arms
│   └── <robot_id>/
│       ├── <robot_id>.urdf
│       └── meshes/
├── urdf-tool-collection/         # input: end-effector tools
│   └── <tool_id>/
│       ├── <tool_id>.urdf
│       └── meshes/
├── urdf-fixture-collection/      # input: static cell fixtures (consumed by robo-fitting-room)
│   └── <fixture_id>/
│       ├── <fixture_id>.urdf
│       └── meshes/
├── urdf-dressed-robots/          # output: dressed (robot+tool) URDFs
│   └── <robot_id>__<tool_id>/
│       ├── <robot_id>__<tool_id>.urdf
│       └── meshes/
│           ├── robot/
│           └── tool/
├── floor-plans/                  # input: 2D OBJ floor plans (consumed by robo-fitting-room)
└── fitted-robots/                # input: scene manifests (consumed by robo-fitting-room)
```

The dresser only owns the first four directories. The last two are
fitting-room concerns; they live in the same catalogue so a single
`--asset-root` / `--catalogue` argument addresses everything.

Each robot, tool, and fixture lives in its own directory **whose name
matches the URDF stem**. This is enforced by `catalogue::scan`:
`urdf-robot-collection/foo/foo.urdf` is discoverable; a stray
`bar.urdf` inside `foo/` is not. The convention prevents ambiguous
references and makes the catalogue listable in O(direct-listing).

## Robot URDF

The arm. Conventionally a 6-DOF KUKA / ABB / Fanuc industrial-arm
URDF, but the dresser is agnostic — anything with a single root link
and a connected kinematic chain works.

The **flange** is the link the tool will attach to. Default flange
detection picks the deepest leaf link in the kinematic chain; this is
the link the last actuated joint moves. Override with `--attach-to
<link_name>` if the URDF has a non-leaf flange (e.g. a wrist plate
that branches).

## Tool URDF

The end-effector — gripper, riveter, welding torch, etc. Tools are
single-rigid-body URDFs with one link named `World` (legacy
convention) and zero joints. Visual + collision geometry hangs off
that link.

For auto-orient to land the tool correctly, the tool URDF must
satisfy:

1. **Origin at the docking center.** The URDF origin `(0, 0, 0)` must
   be the exact point that mates with the robot's flange. Geometry is
   positioned relative to that point via `<origin xyz=… rpy=…>`.
2. **Outward-pointing geometry.** The tool's geometric centroid
   should be on the side of the origin that's "away from the arm";
   the centroid-outward stage of auto-orient depends on this.
3. **Single root link.** Multi-link tools (with internal articulation)
   are out of scope today. Convert to fixed joints or model as a
   robot, not a tool.

`urdf-robo-dresser validate <tool.urdf> --tool` runs these checks.

## Dressed URDF

The output of `dress`: a single self-contained URDF that merges a
robot with a tool. The flange link's `<visual>` and `<collision>`
elements are extended with the tool geometry, rotated by the chosen
orientation. All meshes (robot + tool) are copied into `meshes/robot/`
and `meshes/tool/` beside the dressed URDF, with mesh paths rewritten
to the relative locations.

**The path-prefix split is load-bearing.** [robo-fitting-room](../../robo-fitting-room/)
detects tool geometry by the `meshes/tool/` prefix when emitting the
USDA stage, so the dresser must never put tool meshes under
`meshes/robot/` or vice versa.

## Auto-orientation algorithm

When `--orient` is omitted (or set to `0,0,0`), the dresser picks one
of 64 candidate rotations (`{0, 90, 180, 270}` per axis) using a
purely geometric three-stage scoring algorithm:

### Stage 1 — direction alignment

Compute the tool's "pointing direction": vector from origin `(0,0,0)`
to the centroid of all visual-mesh vertices.

Compute the robot's "flange outward direction": the last actuated
joint's rotation axis, transformed through the forward-kinematics
chain at home pose.

For each candidate orientation, rotate the tool direction and dot
with the flange outward vector. Higher dot product = better
alignment.

### Stage 2 — centroid-outward check

After rotation, the tool's centroid (in robot-base frame) should be
further from the robot base than the flange point itself. This
ensures the tool extends outward, not back into the arm. Candidates
that fail this are demoted but not rejected outright (some tools
genuinely point back, like a welding torch).

### Stage 3 — AABB intersection tiebreak

Among near-equal candidates (within 0.05 of the best `normal_dot`),
count the number of tool-mesh vertices that fall inside the robot's
axis-aligned bounding box at home pose. Fewer intersections = better.
This breaks ties in favour of orientations that don't bury the tool
in the arm geometry.

### Final pick

1. Sort by highest `normal_dot`
2. Among top candidates within 0.05 of the best, prefer
   `centroid_outward > 0`
3. Final tiebreak: lowest `bbox_intersection_count`

The picked orientation is logged as `[INFO] Auto-orient: picked [X,
Y, Z] (normal_dot=…, centroid_outward=…)` so you can sanity-check
the result.

`RUST_LOG=debug` adds the per-candidate scoring detail.

## Hull buffers

Every visual-mesh component is wrapped in a convex hull for collision
detection. Three buffer modes scale the hull outward from its
centroid before computing the convex envelope:

| Mode | Scale factor | Use case |
|---|---|---|
| `tight` | 1.00× | Exact mesh boundary. Precision clearance checks |
| `medium` | 1.05× | +5% inflation. **Default**. Good balance for general simulation |
| `max` | 1.15× | +15% inflation. Generous safety margin or low-quality input meshes |

The convex envelope itself is computed via the quickhull algorithm
(`parry3d`). Per-component (rather than per-link) hulls keep the
collision geometry tight: a robot link with three separate visual
meshes gets three hulls, not one fat envelope around all three.

## Merge semantics

`merge(robot, tool, opts)` produces the dressed URDF by:

1. **Flange resolution** — `attach_to.unwrap_or_else(|| deepest_leaf(robot))`
2. **Tool prefixing** — every tool link/joint name gets the `tool__`
   prefix to avoid collisions with robot names
3. **Tool injection** — tool's visuals + collisions are appended to
   the flange link's geometry list, with each `<origin>` rotated by
   the chosen orientation and translated by `opts.offset`
4. **Mesh path rewrite** — every `<mesh filename="…"/>` in both robot
   and tool is rewritten to point at `meshes/robot/…` or
   `meshes/tool/…` relative to the output URDF
5. **Mesh copy** — actual `.obj`/`.stl` files are copied into the
   output `meshes/` tree. The dresser never references meshes outside
   the output directory

The result is **deterministic**: identical inputs produce
byte-identical dressed URDFs. This matters for CI snapshots, for
content-addressed deployment, and for reproducibility across
machines.

## Validation rules

`validate(urdf, path, is_tool)` runs these checks:

| Check | Robot mode | Tool mode (`--tool`) |
|---|:---:|:---:|
| Acyclic kinematic tree | ✓ | ✓ |
| Single root link | ✓ | ✓ |
| All parent/child link refs resolve | ✓ | ✓ |
| No duplicate link/joint names | ✓ | ✓ |
| Joint limits consistent (`lower < upper`) | ✓ | ✓ |
| Mesh files exist on disk | ✓ | ✓ |
| Visual elements have at least one collision | warn | warn |
| Origin alignment at `(0, 0, 0)` is the docking center | — | ✓ |
| Bounding box centred near origin | — | warn |

Errors fail the run; warnings print but don't.

## Determinism

Identical inputs produce byte-identical output. Two `dress` runs
against the same robot, same tool, same flags yield identical URDFs
and identical mesh files (NFR-105 mirror in this crate).

Sources of non-determinism the dresser **does not** introduce:

- Filesystem readdir order (canonicalised before iteration)
- HashMap iteration order (sorted before write)
- System time / random number generators (never sampled)
- Build profile (no `#[cfg(debug_assertions)]` in output paths)

## Cross-crate flow

```
urdf-robot-collection/   ─┐
urdf-tool-collection/    ─┼─→  urdf-robo-dresser  ─→  urdf-dressed-robots/
                          │              ↓
                          │     (single + batch)
                          │
urdf-fixture-collection/ ─┴──────────────→  robo-fitting-room  ─→  USDA zip  ─→  Isaac Sim
                                                   ↑
                                       fitted-robots/<scene>.yaml
                                       floor-plans/<plan>.obj
```

The two crates share a catalogue convention but compose at the
filesystem layer — the fitting-room reads dressed URDFs by relative
path, never via a Rust dependency on the dresser. This keeps each
crate independently testable and independently deployable.
