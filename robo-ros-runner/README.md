# robo-ros-runner (R3)

Replay a captured joint-angle timeline onto ROS 2 `joint_command` topics, driving
a running Isaac Sim cell that was composed by [`robo-fitting-room`](../robo-fitting-room).

R3 is the producer for the topics Isaac's ROS 2 bridge subscribes to. It reuses the
same cell manifest and dressed URDFs the scene was built from, resolves the **exact**
per-robot `joint_command` topics Isaac listens on, converts captured degrees into
commanded radians through a configurable per-axis mapping, and publishes
`sensor_msgs/msg/JointState`.

```
capture CSV ──▶ frame stream ──▶ deg→rad mapping ──▶ JointState ──▶ joint_command topics
   (degrees)     (per timestamp)   (+clamp/affine)     (radians)      Isaac Sim bridge
```

## Why this exists

The cell is composed by `robo-fitting-room` into a USD stage whose OmniGraph nodes
**subscribe** to `sensor_msgs/msg/JointState` on `/<cell>/<robot>/joint_command`.
Until R3, the only way to drive a robot was hand-rolled `ros2 topic pub` calls. R3
replays a whole real capture (`assets/data-captures/joint_angles_timeline.csv` — a
KUKA PROFINET capture, 7 robots × 6 axes, ~83 Hz, ~420 s) onto those topics.

## Quick start

```sh
# what's in a capture?
robo-ros-runner inspect-csv --csv ../assets/data-captures/joint_angles_timeline.csv

# which topics will a cell drive?
robo-ros-runner topics \
    --cell ../assets/fitted-robots/demo-cell.yaml \
    --csv  ../assets/data-captures/joint_angles_timeline.csv

# preview without a live ROS graph (prints the ros2-topic-pub equivalents)
robo-ros-runner dry-run \
    --cell ../assets/fitted-robots/demo-cell.yaml \
    --csv  ../assets/data-captures/joint_angles_timeline.csv

# real replay to Isaac Sim (DDS is the default transport)
robo-ros-runner run --config r3-run.example.yaml --ros-domain-id 0

# long-running server
robo-ros-runner serve --asset-root ../assets
open http://localhost:3050/docs        # Swagger UI
```

## Build

```sh
cargo build           # the whole tool: CLI + ROS 2 / DDS transport + REST server
```

There are **no build-mode feature flags** — a plain `cargo build` produces everything.
The tool is pure Rust with **no native deps**: the ROS 2 transport is
[`ros2-client`](https://crates.io/crates/ros2-client) over RustDDS (no rcl / ROS
install; speaks RTPS straight to Isaac's FastDDS bridge over a shared `ROS_DOMAIN_ID`).
`run` publishes real DDS by default; `dry-run` (or `--sink stdout`) prints the
`ros2 topic pub` equivalent instead, for previewing without a live ROS graph.

## The degrees→radians problem

The capture is raw KUKA axis readings in **degrees**, and some exceed the URDF joint
limits (e.g. `joint_a2` reads ~−200° vs its `[-2.71, 0.61]` rad bound — a ~180° zero
offset between the KUKA convention and the URDF model frame). A naive deg→rad would
command poses PhysX clamps at the joint stops, jamming the arm.

R3 ships a configurable per-axis affine map `rad = sign · deg · π/180 + offset_rad`
(default identity) plus an optional `clamp_to_limits` that pins out-of-range targets to
the URDF bound and warns. Tune the real KUKA→URDF `sign`/`offset` per axis in the run
config. See [docs/concepts.md](docs/concepts.md).

## Subcommands

| Command | What it does |
|---------|--------------|
| `run` (alias `play`) | Replay a capture onto the selected sink |
| `dry-run` | Replay, forcing the StdoutSink (print, never publish) |
| `validate` | Resolve scene + CSV + URDF + mapping, no publish, exit code |
| `topics` | Print the resolved per-robot `joint_command` topics |
| `inspect-csv` | Print a capture's robots, axes, time span, row count |
| `serve` | Boot the REST API (port 3050) |

## Documentation

- [docs/cli.md](docs/cli.md) — full CLI flag reference
- [docs/api.md](docs/api.md) — REST API surface
- [docs/concepts.md](docs/concepts.md) — topic resolution, the deg→rad mapping, end-to-end verification against Isaac
- [REQUIREMENTS.md](REQUIREMENTS.md) — FR/NFR tracker (single source of progress)

## Tests

```sh
cargo test                                  # implemented FRs green; backlog FRs red (todo!())
cargo test --features real-assets           # also runs against ../assets
```

`cargo test` lists every requirement as a pass/fail row: green = implemented, red =
pending (an intentional `todo!()`). The backlog `todo!()` failures are expected.
