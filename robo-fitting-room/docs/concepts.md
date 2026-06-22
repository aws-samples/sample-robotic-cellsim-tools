# Concepts

This document explains the domain vocabulary used across the
fitting-room codebase, the manifest schema, and the REST API. If you
just want to use the tool, [`cli.md`](cli.md) and [`api.md`](api.md)
are more direct; come here when something in the code or a doc says
"see concepts.md" and you want the ground truth.

## Manifest

A YAML file declaring the contents and layout of one robotic cell:
which dressed robots and fixtures to place, at what world-space poses,
optionally with a 2D floor plan reference. The manifest is the single
input to `compose`; the USDA + zip are the output. See
[`manifests.md`](manifests.md) for the field-by-field reference.

## Dressed URDF

A composite URDF produced by
[`urdf-robo-dresser`](../../urdf-robo-dresser/) that merges a base
robot URDF (e.g. KUKA KR240) with an end-effector tool URDF (e.g. a
gripper or rivet head) into a single file. Tool meshes live under
`meshes/tool/`, robot meshes under `meshes/robot/` — that path-prefix
distinction is how the `--full-tools` flag identifies tool geometry
at compose time.

The fitting-room only consumes dressed URDFs; it never touches a bare
robot or tool URDF directly.

## Fixture URDF

A URDF representing static cell infrastructure (clamping fixtures,
conveyors, safety fences). The composer rejects any fixture URDF
containing a non-`fixed` joint — fixtures are by definition
articulation-free, so this catches authoring mistakes early.

Fixtures are anchored to the world via a `PhysicsFixedJoint
"world_anchor"` whose `localPos0/localRot0` matches the manifest
pose, so PhysX never reports "disjointed body transforms".

## Scene origin

An optional `scene_origin: {x,y,z}` field on the manifest. When set,
its value is subtracted from every robot/fixture `position` and from
the floor-plan `offset` before USDA emission. The point of this is
ergonomic viewport navigation: real cells often live at coordinates
like `(70.5, 27.4, 0)` which puts the camera nowhere near anything;
subtracting `scene_origin` moves the cell to the world origin where
Isaac's default camera framing works.

The subtraction is applied at parse time, so every downstream piece
of the pipeline sees already-centred coordinates. The original
positions are preserved verbatim in the saved manifest YAML — the
edit is non-destructive.

## Home pose

The joint-angle vector applied at scene load. Default is the KUKA
"stowed" pose `[0°, -90°, +90°, 0°, 0°, 0°]` — a folded-upright
configuration that doesn't intersect the floor or the robot's own
base. Per-robot override via the manifest's `home_pose_deg` field.

The same angles are written into two places:

1. The per-link `xformOp:transform` bind pose (forward kinematics
   pre-computed in compose so PhysX's articulation pose initialiser
   doesn't disagree with the per-joint frames)
2. Each joint's `drive:angular:physics:targetPosition`, so the arm
   stays put when Play starts

Both must agree, otherwise PhysX warns "disjointed body transforms"
every frame.

## Articulation root

Every robot in the composed USDA carries `PhysicsArticulationRootAPI`
+ `PhysxArticulationAPI` on its top-level Xform. This tells PhysX to
treat the robot's link tree as a single articulation — a connected
graph of rigid bodies linked by reduced-coordinate joints, solved as
one solver pass per step. The alternative ("disconnected rigid bodies
+ explicit joint constraints") is much less stable for industrial-arm
masses + drive stiffnesses.

The articulation root is the entry point for joint-state queries and
for the `IsaacArticulationController` OmniGraph node.

## Topic schema

All ROS 2 topics emitted by the composed USDA follow the two-segment
namespace contract `/<cell_id>/<robot_id>/<channel>`:

| Channel | Topic | Type | Direction |
|---|---|---|---|
| Joint telemetry | `/<cell_id>/<robot_id>/joint_states` | `sensor_msgs/msg/JointState` | pub |
| Joint command | `/<cell_id>/<robot_id>/joint_command` | `sensor_msgs/msg/JointState` | sub |
| Per-link contact | `/<cell_id>/<robot_id>/contacts/<link>` | `std_msgs/msg/Bool` | pub |
| Sim clock | `/<cell_id>/clock` | `rosgraph_msgs/msg/Clock` | pub |

`cell_id` comes from the manifest's `name` field (sanitised).
`robot_id` from each robot's manifest entry. Both segments are run
through `sanitize_ros_segment` so the result satisfies
`rmw_validate_namespace` / `rmw_validate_topic_name` — digit-leading
values get an `r_` prefix, non-alphanumeric characters become `_`,
empty segments fall back to `r_anon`.

This schema is the same under FastDDS (Isaac default) and Cyclone DDS
(`RMW_IMPLEMENTATION=rmw_cyclonedds_cpp`).

## Contact graphs

Per-link contact telemetry uses the per-robot OmniGraph (we don't
emit a separate `ActionGraph_contacts_<id>`). For every link with at
least one `<collision>` block, compose emits inside the existing
per-robot graph:

```
ContactSensor_<link>   (IsaacReadContactSensor, csPrim → /World/robots/<robot>/<link>/ContactSensor_node)
        │ outputs:execOut
        ▼
PublishContact_<link>  (ROS2Publisher, messageName="Bool", topicName="contacts/<link>")
        │ outputs to ROS 2
        ▼
   /<cell_id>/<robot_id>/contacts/<link>
```

The publisher's exec input is wired to the sensor's `outputs:execOut`
(NOT the playback tick), so the topic stays silent until a contact
event lands. Each collision-bearing link prim also carries
`PhysxContactReportAPI` and a child `IsaacContactSensor` prim with
`enabled = 1` and `radius = -1` (full-body queries).

## Visual fidelity tiers

The compose pass has two independent fidelity flags:

- `--full-robots` (default `false`): keep the robot arm's visual mesh
  prims. Default drops them and surfaces collision hulls instead, for
  a smaller zip and slightly faster Isaac load. Articulation behaviour
  is identical either way.
- `--full-tools` (default `true`): keep tool visual mesh prims. Tools
  are detected by the `meshes/tool/` path prefix the dresser writes.
  Set `--full-tools=false` to drop tool visuals and surface only the
  tool collision hulls.

Both flags only affect the `def Mesh "visual_*"` emissions. Joint
authority, contact sensor wiring, and articulation tuning are
untouched.

See [`cli.md`](cli.md) for the size-vs-fidelity matrix on the
demo cell.

## Determinism

Identical manifests produce byte-identical USDA output (NFR-105).
This matters for CI snapshots, for content-addressed deployment, and
for the `tests/openapi_snapshot` + `tests/real_assets` real-asset
test pattern. The composer never reads the system clock, the build
profile, or any other non-manifest input that could perturb the
output.

## Steel-arm inertia override

KR240 URDFs from many vendors ship with placeholder inertials that
would explode under the stiff drives compose configures. A small
hardcoded table of `(mass, diagonal_inertia)` overrides recognised
KR240 link suffixes (`_base_link`, `_link_1` … `_link_6`); URDF
inertials are used verbatim for any link not in the table.

This is a workaround for incomplete URDF authoring, not a feature.
Generalising to a density-from-bbox path that handles arbitrary
robot geometries is tracked as BL-13.
