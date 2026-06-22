# Concepts

## The data contract

Three layers must line up for a captured pose to drive the right joint in Isaac:

| Layer | Identifier | Example |
|-------|-----------|---------|
| Capture column | `<robot_id>_A<n>_deg` (degrees) | `robot_01_A2_deg` |
| URDF joint | `joint_a<n>` (revolute, radian limits) | `joint_a2`, `[-2.71, 0.61]` |
| ROS topic | `/<cell_ns>/<robot_ns>/joint_command` | `/Demo_Cell/robot_01/joint_command` |

R3 reads the cell manifest with `robo_fitting_room::parse_scene`, the dressed URDFs with
`urdf_robo_dresser::parse_urdf`, and the capture with its own streaming reader, then
publishes `sensor_msgs/msg/JointState` with `name = [joint_a1..joint_a6]` and `position`
in **radians**.

## The dressed URDF must currently be present

Each robot's `dressed_urdf` (referenced from the cell manifest) **must exist on disk** for
ROS commanding today. `build_plan` parses it once per robot to obtain:

1. **The joint names** (`joint_a1..joint_a6`, in URDF declaration order) — these become the
   `JointState.name` array, and Isaac matches a command to joints **by name**, so they must
   be exact.
2. **The radian limits** (`<limit lower/upper>`) — used only when `--clamp` is on, to pin
   out-of-range targets to the joint bound.

If a robot's URDF is missing, `build_plan` errors before anything is published. Note the
*topic string* itself needs no URDF (it is derived purely from the manifest `name` +
`robot_id`) — only the message body (names) and the optional clamp (limits) come from the
URDF.

This is an implementation choice, not an intrinsic requirement: the joint names are
effectively constant (`joint_a1..joint_a6`) for these assets, and limits matter only for
`--clamp`. A URDF-free mode (joint names from config/CLI, clamping off) is feasible but not
yet built. If you want to decouple commanding from the heavy mesh-bearing dressed URDFs,
point `dressed_urdf` at a minimal stub URDF that declares just the six revolute joints
(names + limits, no meshes/inertials) — R3 only reads the joints.

## Topic resolution

Topics must match **byte-for-byte** what `robo-fitting-room` emitted into the USD
ActionGraphs, or Isaac's subscriber never sees the message. R3 re-implements
`sanitize_ros_segment` identically (it is private in fitting-room):

- A segment whose first char is a digit (or that is empty) gets an `r_` prefix
  (so a robot id like `42` becomes `r_42`).
- Every non-`[A-Za-z0-9_]` char becomes `_`.

So cell `Demo Cell` → `Demo_Cell`, robot `robot_01` → `robot_01` (already a valid
token), and the topic is `/Demo_Cell/robot_01/joint_command`. The `real_assets` test
asserts the resolved set equals the authored `assets/data-captures/ros-topics.txt`,
including an 8th robot (`robot_08`) that has a topic but no capture columns (flagged
`has_csv_data = false`).

## The degrees→radians mapping

The capture is raw KUKA axis readings in degrees. Measured across the whole capture,
axes A2/A3/A5 sit **outside** the URDF limits on every robot — A2 in particular reads
around −110°..−225° against a `[-2.71, 0.61]` rad bound, i.e. a ~180° (~π) zero offset
between the KUKA axis convention and the URDF model frame.

A naive deg→rad would hand PhysX a target it clamps at the joint stop, jamming the arm.
R3's mapping is a per-axis affine:

```
rad[i] = sign[i] · deg[i] · (π/180) + offset_rad[i]
```

- **Default** is the identity (`sign = +1`, `offset = 0`) = plain deg→rad. Faithful unit
  conversion, no silent correction.
- **`clamp_to_limits`** (config / `--clamp`) pins an out-of-range target to the URDF
  bound and counts it; `warn_out_of_range` logs each one. A run that reports
  "N clamped" with N large means your offsets are wrong, not that the arm is fine.
- Tune `sign`/`offset_rad` per axis (cell-wide `default_axes`, or `per_robot`) to land
  the trace inside the limits. For this capture an A2 `offset_rad` near `+π` is the
  starting point.

## Playback scheduling

`build_plan` resolves everything once (scene, per-robot URDF limits + axis maps, CSV
schema, topics). `run_playback` then streams frames and sleeps each inter-frame gap
(`(t[i+1] − t[i]) / rate`, capped at 1 s). The clock is injected — a mock clock makes
tests instant and deterministic. Empty CSV cells follow the `hold_last` policy by
default (carry the prior value; a robot with no value yet stays unpublished). Only a
fully-populated six-axis pose is published.

## Transports

The engine writes to a `CommandSink` and never knows the wire:

- **`Ros2ClientSink`** (the default transport): pure-Rust `ros2-client` over RustDDS.
  Speaks RTPS straight to Isaac's FastDDS/Cyclone bridge over a shared `ROS_DOMAIN_ID` —
  no rcl, no ROS install, no native deps. JointState is registered as the DDS type
  `sensor_msgs::msg::dds_::JointState_`. This is what `run` uses.
- **`StdoutSink`** (preview, zero-dep): prints the `ros2 topic pub` equivalent instead of
  publishing. This is `dry-run` (and `--sink stdout`) and the deterministic
  golden-output source — useful when there's no live ROS graph to publish to.

## End-to-end verification against Isaac Sim

1. Compose + load the cell in Isaac Sim (`robo-fitting-room compose ...`), press Play.
   Make sure `isaacsim.ros2.bridge` is enabled and `ROS_DOMAIN_ID` matches.
2. Dry-run first to confirm the topic strings match `ros2 topic list`:
   ```sh
   robo-ros-runner dry-run --cell <cell.yaml> --csv <capture.csv> --once
   ```
3. Real replay (DDS is the default transport):
   ```sh
   robo-ros-runner run --config r3-run.example.yaml --ros-domain-id 0
   ```
4. Confirm from a ROS 2 shell:
   ```sh
   ros2 topic echo /Demo_Cell/robot_01/joint_command   # R3 publishing
   ros2 topic echo /Demo_Cell/robot_01/joint_states    # Isaac tracking
   ros2 topic hz   /Demo_Cell/robot_01/joint_command   # ≈ 83 Hz × rate
   ```
   The arm should track the capture; with `--clamp` the shoulder/elbow stay inside their
   limits instead of jamming.
