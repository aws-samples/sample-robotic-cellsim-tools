# CLI reference

`robo-ros-runner` is a one-shot CLI plus an optional long-running server. Every
operation is one library function the CLI and REST both call.

## `run` (alias `play`)

Replay a capture onto the selected sink.

| Flag | Default | Meaning |
|------|---------|---------|
| `--config <path>` | — | Run-config YAML (durable inputs + mapping). |
| `--cell <path>` | — | Cell manifest YAML (overrides `config.cell_yaml`). |
| `--csv <path>` | — | Capture CSV (overrides `config.csv`). |
| `--rate <f64>` | `1.0` | Playback speed multiplier (capture rate × this). |
| `--loop-play` | off | Loop the timeline until Ctrl-C / cancel. |
| `--start <f64>` | — | Skip frames before this time (seconds). |
| `--end <f64>` | — | Stop after this time. |
| `--robots <ids>` | all | Comma-separated robot_id filter. |
| `--once` | off | Publish a single frame (at `--start`, or the first) and exit. |
| `--sink <kind>` | `dds` | `dds` (real ROS 2 publish, default), `stdout` (preview print), or `zenoh` (cross-host via a bridge). |
| `--ros-domain-id <u32>` | `0` | DDS domain for the `dds` sink. |
| `--zenoh-connect <ep>` | — | Zenoh connect endpoint, e.g. `tcp/10.0.0.5:7447` (repeatable). `--sink zenoh` only. |
| `--zenoh-key-prefix <s>` | `r3` | Zenoh key-expr prefix; MUST match the bridge's `--key-prefix`. `--sink zenoh` only. |
| `--clamp` | off | Force clamp-to-URDF-limits on (overrides config). |

Either `--config`, or both `--cell` and `--csv`, are required. CLI flags override
config-file values.

Each robot's `dressed_urdf` (from the cell manifest) must exist on disk: R3 reads it for
the joint names and limits that go into every command. See [concepts.md](concepts.md#the-dressed-urdf-must-currently-be-present).

```sh
robo-ros-runner run --config r3-run.example.yaml                 # publishes real DDS (default)
robo-ros-runner run --cell cell.yaml --csv cap.csv --rate 2 --robots robot_01
robo-ros-runner run --config r3-run.example.yaml --ros-domain-id 0
robo-ros-runner run --cell cell.yaml --csv cap.csv --sink stdout  # preview, no DDS
robo-ros-runner run --config r3-run.example.yaml \
    --sink zenoh --zenoh-connect tcp/10.0.1.50:7447                # cross-host via robo-zenoh-bridge
```

The `zenoh` sink reaches Isaac across hosts / a VPC where DDS multicast can't: R3 publishes
TCP unicast to `--zenoh-connect`, where [`robo-zenoh-bridge`](../../robo-zenoh-bridge)
(running on the Isaac host) republishes onto Isaac's local DDS. See
[concepts.md](concepts.md#the-zenoh-cross-host-path).

## `dry-run`

Same flags as `run`, but forces the `StdoutSink` regardless of `--sink`. Prints one
`ros2 topic pub --once <topic> sensor_msgs/msg/JointState "{...}"` line per published
frame — proves the topics + joint names + radian positions before touching DDS.

## `validate`

Resolve scene + CSV + URDF + mapping without publishing. Exits 0 and lists the
resolved topics when inputs are consistent; non-zero with a diagnostic otherwise.

```sh
robo-ros-runner validate --cell cell.yaml --csv cap.csv
robo-ros-runner validate --config r3-run.example.yaml
```

## `topics`

Print the resolved per-robot `joint_command` topics for a cell. With `--csv`, each
topic is flagged `[has data]` or `[NO CSV DATA]`.

```sh
robo-ros-runner topics --cell cell.yaml --csv cap.csv
```

## `inspect-csv`

Print a capture's robots, axes per robot, time span, and row count.

```sh
robo-ros-runner inspect-csv --csv cap.csv            # wide format (default)
robo-ros-runner inspect-csv --csv cap.csv --format long   # long format (backlog)
```

## `serve`

Boot the REST API. See [api.md](api.md).

| Flag | Default | Meaning |
|------|---------|---------|
| `--host <host>` | `127.0.0.1` | Bind host (`0.0.0.0` for all interfaces). |
| `--port <u16>` | `3050` | TCP port. |
| `--asset-root <dir>` | — | Sandbox root for all API paths. |

## Logging

`RUST_LOG=info` (default) / `debug` / `warn`. Out-of-range mapping warnings are at
`warn`.
