# Manifest schema

Scene definitions are YAML. Positions in metres, orientations in
degrees (ABC convention = ZYX-Euler: A=yaw/Z, B=pitch/Y, C=roll/X).
URDF and floor-plan paths are resolved relative to the manifest file.

## Full example

```yaml
name: "Demo Cell"
description: "8-robot KUKA KR240 production cell"

# Optional. Subtracted from every placement and the floor-plan offset
# so the composed scene is centred near the world origin (FR-106).
scene_origin: { x: 68.3955, y: 29.7115, z: 0.0 }

robots:
  - robot_id: "robot_01"
    dressed_urdf: "../urdf-dressed-robots/kr240r2900__gripper_tool/kr240r2900__gripper_tool.urdf"
    position: { x: 69.498, y: 28.397, z: 0.0 }
    orientation: { a: 0.0, b: 0.0, c: 0.0 }
    home_pose_deg: [0, -90, 90, 0, 0, 0]   # optional; per-robot home

  - robot_id: "robot_02"
    dressed_urdf: "../urdf-dressed-robots/kr240r2900__rivet_tool/kr240r2900__rivet_tool.urdf"
    position: { x: 67.398, y: 27.297, z: 0.0 }
    orientation: { a: 90.0, b: 0.0, c: 0.0 }

fixtures:
  - id: "station_fixture_a"
    urdf: "../urdf-fixture-collection/station_fixture_a/station_fixture_a.urdf"
    position: { x: 65.803, y: 30.494, z: 0.390 }
    orientation: { a: 90.0, b: 0.0, c: 0.0 }

floor_plan:
  geometry_file: "../floor-plans/demo-cell-floor-plan.obj"
  offset: { x: 68.3955, y: 29.7115, z: 0.0 }
  scale: 1.0
```

## Field reference

### Top level

| Field | Type | Required? | Notes |
|---|---|---|---|
| `name` | string | yes | Human-readable cell name. **Load-bearing**: doubles as `cell_id` in every emitted ROS 2 topic (`/<cell_id>/<robot_id>/...`), so renaming a manifest is a runtime-visible event |
| `description` | string | no | Free-text |
| `scene_origin` | `{x,y,z}` | no | Subtracted from every placement and floor-plan offset before USDA emission. Use to centre a real-world cell at the USD world origin |
| `robots` | array | usually | One entry per articulated robot in the cell |
| `fixtures` | array | no | Static cell infrastructure (clamping fixtures, conveyors, fences) |
| `floor_plan` | object | no | 2D CAD reference geometry — parsed but not yet emitted into the USDA (BL-09) |

### `robots[]`

| Field | Type | Required? | Notes |
|---|---|---|---|
| `robot_id` | string | yes | Stable ID. Forms the prim path (`/World/robots/<robot_id>`) and the second segment of the ROS 2 topic namespace. Sanitised via `sanitize_ros_segment` (digit-leading IDs get an `r_` prefix to satisfy ROS 2 token rules) |
| `dressed_urdf` | path | yes | Path to a dressed URDF (output of `urdf-robo-dresser dress`). Relative to the manifest file |
| `position` | `{x,y,z}` | yes | World-space metres. Subject to `scene_origin` subtraction |
| `orientation` | `{a,b,c}` | yes | Degrees, ABC = ZYX-Euler |
| `home_pose_deg` | `[float, …]` | no | One angle per actuated joint, URDF declaration order. Length must match the URDF's actuated joint count. Drives both the FK bind pose and per-joint `targetPosition`. Omit for the default KUKA stowed pose `[0, -90, 90, 0, 0, 0]` |

### `fixtures[]`

| Field | Type | Required? | Notes |
|---|---|---|---|
| `id` | string | yes | Stable ID for the prim path (`/World/fixtures/<id>`) |
| `urdf` | path | yes | Path to a fixture URDF. **Must contain only `fixed` joints** — non-fixed joints fail at compose time |
| `position` | `{x,y,z}` | yes | World-space metres. Subject to `scene_origin` subtraction |
| `orientation` | `{a,b,c}` | yes | Degrees, ABC = ZYX-Euler |

### `floor_plan`

| Field | Type | Required? | Notes |
|---|---|---|---|
| `geometry_file` | path | yes | OBJ file with line geometry only (`v` + `l`, no `f`) |
| `offset` | `{x,y,z}` | no | World-space offset applied to the OBJ-local coordinates |
| `cell_origin_dxf` | `{x,y,z}` | no | Companion to `offset`. When set without `offset`, parse_scene derives `effective_offset = scene_origin? - cell_origin_dxf` so the chosen DXF point lands at the world origin. When both are set, `offset` wins |
| `scale` | float | no | Default `1.0`. Multiplied into the OBJ-local coords |

#### Floor-plan OBJ requirements

1. Only `v` (vertex) and `l` (line) elements — no faces
2. All geometry at `z = 0` (2D plan view)
3. Coordinates in metres
4. Footprint layers identified by `o E9_121051_<…>` group directives
   OR `# Layer: E9_121051_<…>` comments — the parser handles both

`floor_plan` is parsed and the segment count is reported in the
compose result, but **not emitted into the USDA today** — Hydra
CAR-811 prevented BasisCurves and flat-mesh fallbacks from rendering.
Re-enabling is tracked as BL-09.

## Validation errors

`robo-fitting-room validate <manifest>` runs every check below. Each
failure prints the relevant `robot_id` / `fixture_id` so problems are
locatable in large manifests.

| Error | Meaning | Fix |
|---|---|---|
| `URDF not found: <path>` | A `dressed_urdf` or `fixture.urdf` path doesn't resolve | Check the path is relative to the manifest file's directory |
| `<path>: file does not contain a <robot> element` | The path resolves but isn't a URDF | Pass a valid URDF (output of `urdf-robo-dresser dress`) |
| `Robot '<id>': non-finite position or orientation` | `NaN` / `Inf` in `position` or `orientation` | Replace with finite values |
| `Robot '<id>': home_pose_deg has N entries but URDF has M actuated joints` | Length mismatch | Either fix the array length or remove the field to use the default stowed pose |
| `Fixture '<id>' has non-fixed joint '<joint>'` | Fixture URDF has a non-`fixed` joint | Convert to `fixed` or model as a robot, not a fixture |

## The `name` field is load-bearing

Beyond display, the manifest's `name` becomes the `cell_id` segment in
every ROS 2 topic the composed USDA publishes:

```
/<cell_id>/<robot_id>/joint_states
/<cell_id>/<robot_id>/joint_command
/<cell_id>/<robot_id>/contacts/<link>
/<cell_id>/clock
```

Renaming a manifest after operators have built dashboards / consumers
keyed on the topic names is a **runtime-visible event** — they will
need to retarget. The `name` is sanitised via `sanitize_ros_segment`
before being used as a topic segment (digit-leading values get an
`r_` prefix; non-alphanumerics become `_`).
