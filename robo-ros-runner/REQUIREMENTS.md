# `robo-ros-runner` (R3) Requirements Tracker

> **Status legend**: DONE (linked to source), PARTIAL (linked + remaining gap noted), NOT STARTED, BACKLOG (deferred)

A pure-Rust application that replays a captured joint-angle timeline onto ROS 2 `joint_command` topics, driving a running Isaac Sim cell composed by `robo-fitting-room`. It resolves the same topic namespaces Isaac subscribes to, converts captured degrees into commanded radians through a configurable per-axis mapping, and publishes `sensor_msgs/msg/JointState`. ROS 2 / DDS (pure-Rust ros2-client over RustDDS) and the REST server are core — a plain `cargo build` produces the whole tool with no native deps and no build-mode feature flags. A `StdoutSink` provides a no-DDS `dry-run` preview.

Target: standalone open-source release.

---

## 1. Definitions

| Term | Meaning |
|------|---------|
| **Capture** | A CSV timeline of joint angles. Wide form (`joint_angles_timeline.csv`) has one column per robot+axis (`<robot_id>_A<n>_deg`, degrees) plus a `t_profinet_s` time column. Long form (`combined_timeseries.csv`) is tidy `t,robot,source,signal,value`. |
| **Cell manifest** | The `robo-fitting-room` scene YAML declaring robot IDs and their dressed URDFs. R3 reuses `robo_fitting_room::parse_scene` to read it. |
| **Dressed URDF** | A composite robot+tool URDF; its six actuated (non-Fixed) joints `joint_a1..joint_a6` and their radian limits drive the JointState names + clamp bounds. |
| **Resolved topic** | The per-robot `joint_command` topic `/<cell_ns>/<robot_ns>/joint_command`, where each segment passes through `sanitize_ros_segment` — byte-identical to `robo-fitting-room`'s emission. |
| **Frame** | One timestamp's resolved joint state across all robots, after the empty-cell policy is applied. |
| **Empty-cell policy** | How blank CSV cells are treated: `HoldLast` (carry the prior value) or `Skip` (no value this frame). |
| **Axis map** | Per-axis affine `rad = sign * deg * (pi/180) + offset_rad`. Default identity = plain degrees→radians. Reconciles the captured KUKA zero/sign convention with the URDF model frame. |
| **CommandSink** | The transport abstraction. `Ros2ClientSink` publishes real DDS (the default transport); `StdoutSink` prints the `ros2 topic pub` equivalent for the `dry-run` preview. |
| **Clock** | Injected time source for the replay scheduler. `RealClock` sleeps; a mock clock makes tests deterministic and instant. |
| **Playback plan** | The fully-resolved inputs (topics, per-robot limits + axis maps, CSV schema) built once before the replay loop runs. |

---

## 2. Functional Requirements

### 2.1 CSV Intake

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-100 | The tool SHALL parse a wide-format capture header into a per-robot, per-axis column map. | Must | A header `t_profinet_s,<id>_A1_deg..A6_deg` yields one robot entry with six column indices. | [DONE](src/csv_intake.rs#L48) |
| FR-101 | The tool SHALL stream capture rows without loading the whole file into memory. | Must | A 99k-row capture is replayed with peak memory independent of row count. | [DONE](src/csv_intake.rs#L120) |
| FR-102 | The tool SHALL apply a hold-last-value policy by default: a blank cell keeps the prior value; a robot with no value yet stays unpublished. | Must | Given a blank A1 cell after a populated one, the frame reports the prior A1 value. | [DONE](src/csv_intake.rs#L148) |
| FR-103 | The tool SHALL support a `Skip` empty-cell policy where blanks produce no value for that axis. | Should | A blank cell under `Skip` yields `None`, not the prior value. | [DONE](src/csv_intake.rs#L145) |
| FR-104 | The tool SHALL report a capture's schema: robots, axes per robot, row count, and time span. | Must | `inspect-csv` on the reference capture reports 7 robots, 6 axes, ~99,774 rows, ~420 s. | [DONE](src/csv_intake.rs#L57) |
| FR-105 | The tool SHALL parse the long-form `combined_timeseries.csv` (`D->M\|A<n>_deg`). | Should | Long-form intake produces the same frames as the wide form for the shared robots. | BACKLOG |

### 2.2 Joint Mapping

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-200 | The tool SHALL extract the six actuated joint names + radian limits from each robot's dressed URDF, reusing `parse_urdf`. | Must | A KR-class dressed URDF yields `joint_a1..joint_a6` with their `<limit>` bounds. | [DONE](src/mapping.rs#L92) |
| FR-201 | The tool SHALL apply a configurable per-axis affine `sign * deg * (pi/180) + offset_rad`. | Must | A per-axis offset of +pi pulls a -200.57° A2 sample into the URDF range. | [DONE](src/mapping.rs#L128) |
| FR-202 | The default mapping SHALL be the identity (sign +1, offset 0, clamp off) = plain degrees→radians. | Must | A 90° input with default mapping yields exactly pi/2 rad. | [DONE](src/mapping.rs#L24) |
| FR-203 | The tool SHALL optionally clamp commanded positions to the URDF limits, warning on out-of-range values. | Must | With clamp on, a -200.57° A2 sample is pinned to the joint_a2 lower limit and counted as clamped. | [DONE](src/mapping.rs#L128) |
| FR-204 | The tool SHALL support per-robot mapping overrides. | Should | A `per_robot` entry overrides the cell-wide `default_axes` for that robot. | [DONE](src/mapping.rs#L72) |

### 2.3 Topic Resolution

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-300 | The tool SHALL sanitize ROS segments byte-identically to `robo-fitting-room` (digit/empty-leading → `r_` prefix; non-`[A-Za-z0-9_]` → `_`). | Must | A digit-leading id `42` → `r_42`; `Demo Cell` → `Demo_Cell`. | [DONE](src/topics.rs#L11) |
| FR-301 | The tool SHALL resolve `/<cell_ns>/<robot_ns>/joint_command` for every robot in the cell manifest. | Must | A cell with robot `robot_01` resolves `/Demo_Cell/robot_01/joint_command`. | [DONE](src/topics.rs#L60) |
| FR-302 | The resolved topics SHALL equal the authored `ros-topics.txt` for the reference cell. | Must | `topics` on the reference cell prints the eight `joint_command` topics verbatim. | [DONE](tests/real_assets.rs) |
| FR-303 | The tool SHALL flag robots present in the manifest but absent from the capture. | Must | A robot with no CSV columns is reported with `has_csv_data = false`. | [DONE](src/topics.rs#L78) |
| FR-304 | The tool SHALL accept per-robot topic overrides and a cell-namespace override from config. | Should | A `topic_overrides` entry replaces the derived topic for that robot. | [DONE](src/topics.rs#L66) |

### 2.4 Transport / Publish

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-400 | The tool SHALL abstract the wire behind a `CommandSink` trait so transports are pluggable. | Must | The playback engine drives any `CommandSink` without knowing its kind. | [DONE](src/transport/mod.rs#L48) |
| FR-401 | The default `StdoutSink` SHALL print the `ros2 topic pub` equivalent and require no native deps. | Must | `dry-run` prints one `ros2 topic pub --once <topic> sensor_msgs/msg/JointState ...` line per published frame. | [DONE](src/transport/stdout.rs#L42) |
| FR-402 | The tool SHALL define a `JointStateMsg` matching the `sensor_msgs/msg/JointState` CDR layout. | Must | The struct serializes header/name/position/velocity/effort in IDL order. | [DONE](src/transport/jointstate.rs#L23) |
| FR-403 | The tool SHALL publish real DDS via the pure-Rust ros2-client — the default transport, always built. | Must | `run` (default `--sink dds`) publishes JointState the Isaac bridge receives (live arm-tracking verified manually per docs/concepts.md). | [DONE](src/transport/dds.rs#L84) |
| FR-404 | Published positions SHALL be radians; velocity and effort SHALL be empty. | Must | A published JointState carries six radian positions and empty velocity/effort. | [DONE](src/transport/jointstate.rs#L40) |
| FR-405 | The DDS transport SHALL use a configurable `ROS_DOMAIN_ID`. | Must | `--ros-domain-id N` participates on DDS domain N. | [DONE](src/transport/dds.rs#L29) |
| FR-406 | The tool SHALL provide a rosbridge (websocket) sink for the future WebApp. | Should | A browser client drives playback over a rosbridge websocket. | BACKLOG |
| FR-407 | The tool SHALL provide a Zenoh sink that publishes joint commands over Zenoh (TCP unicast) to a remote bridge, for cross-host / cross-VPC delivery. | Should | `run --sink zenoh --zenoh-connect tcp/<ip>:7447` publishes each frame as a JSON `ZenohJointCommand` on key `<prefix>/<topic-without-leading-slash>`; `kind()=="zenoh"`; the topic↔key-expr mapping round-trips. | [DONE](src/transport/zenoh.rs#L1) |

### 2.5 Playback Scheduling

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-500 | The tool SHALL build a playback plan (scene + URDF limits + CSV schema + topics) once before replay. | Must | `build_plan` validates all inputs and errors before any publish. | [DONE](src/playback.rs#L84) |
| FR-501 | The tool SHALL replay at capture rate using `t_profinet_s` deltas. | Must | Inter-frame sleeps equal the captured time deltas. | [DONE](src/playback.rs#L227) |
| FR-502 | The tool SHALL accept a `--rate` multiplier. | Must | `--rate 2` halves every inter-frame sleep. | [DONE](src/playback.rs#L225) |
| FR-503 | The tool SHALL accept a `--start`/`--end` time window. | Must | `--start 0.05` skips frames before 0.05 s. | [DONE](src/playback.rs#L240) |
| FR-504 | The tool SHALL accept a `--robots` filter. | Should | `--robots <id>` publishes only that robot. | [DONE](src/playback.rs#L108) |
| FR-505 | The tool SHALL support `--loop` to restart the timeline. | Should | `--loop` replays the capture, incrementing the loop counter. | [DONE](src/playback.rs#L277) |
| FR-506 | The tool SHALL support `--once` to publish a single frame and exit. | Must | `--once` publishes exactly one frame. | [DONE](src/playback.rs#L177) |
| FR-507 | A running playback SHALL be cancellable via a shared flag. | Should | Setting the cancel flag stops the loop at the next frame. | [DONE](src/playback.rs#L235) |
| FR-508 | The replay scheduler SHALL accept an injected clock for deterministic tests. | Must | A mock clock replays the whole timeline instantly. | [DONE](src/playback.rs#L22) |

### 2.6 CLI

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-600 | The tool SHALL provide a `run` (alias `play`) subcommand. | Must | `run --cell <c> --csv <c>` replays to the selected sink. | [DONE](src/main.rs#L66) |
| FR-601 | The tool SHALL provide a `validate` subcommand that resolves everything without publishing. | Must | `validate` exits 0 and lists topics when inputs are consistent. | [DONE](src/main.rs#L250) |
| FR-602 | The tool SHALL provide a `topics` subcommand. | Must | `topics --cell <c>` prints the resolved per-robot topics. | [DONE](src/main.rs#L296) |
| FR-603 | The tool SHALL provide an `inspect-csv` subcommand. | Must | `inspect-csv --csv <c>` prints robots, axes, span, row count. | [DONE](src/main.rs#L337) |
| FR-604 | The tool SHALL provide a `dry-run` subcommand that forces the StdoutSink. | Must | `dry-run` prints the publish-equivalents regardless of `--sink`. | [DONE](src/main.rs#L70) |
| FR-605 | CLI flags SHALL override config-file values. | Must | `--rate 2` wins over a config `rate: 1.0`. | [DONE](src/config.rs#L150) |
| FR-606 | The CLI SHALL ship before/after-help banners stating the dual-mode design. | Should | `--help` shows the quick-start + "both transports call the same library functions". | [DONE](src/main.rs#L13) |

### 2.7 Server / Web

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-700 | The tool SHALL provide an axum HTTP server (core, always built) with configurable bind + port (default 3050). | Must | `serve --port 3050 --asset-root <dir>` accepts connections. | [DONE](src/server/mod.rs#L26) |
| FR-701 | `POST /api/v1/topics` SHALL resolve topics for a cell manifest. | Must | Returns the cell name + resolved topics. | [DONE](src/server/handlers/topics.rs#L24) |
| FR-702 | `POST /api/v1/csv/inspect` SHALL return a capture's schema. | Must | Returns robots, axes, row count, time span. | [DONE](src/server/handlers/csv.rs#L22) |
| FR-703 | `POST /api/v1/csv/upload` SHALL accept a capture upload, sandboxed under asset_root. | Should | An uploaded CSV is stored under `<asset_root>/data-captures/`. | BACKLOG |
| FR-704 | `POST /api/v1/playback` SHALL start a playback job and return a job id. | Must | Returns `{job_id, status, topics}`; the job runs off the request thread. | [DONE](src/server/handlers/playback.rs#L56) |
| FR-705 | `DELETE /api/v1/playback/:id` SHALL cancel a running job. | Must | Cancellation flips the job to `cancelled`. | [DONE](src/server/handlers/playback.rs#L118) |
| FR-706 | `GET /api/v1/playback/:id` SHALL return job status. | Must | Returns frames/messages/clamped/loop counts + status. | [DONE](src/server/handlers/playback.rs#L147) |
| FR-707 | The server SHALL expose `/healthz` + `/readyz` probes at the root. | Must | `/healthz` is 200 once bound; `/readyz` is 200 when asset_root canonicalises. | [DONE](src/server/handlers/health.rs#L18) |
| FR-708 | The server SHALL publish `/openapi.json` + Swagger UI at `/docs`, enforced by a checked-in snapshot. | Should | `tests/openapi_snapshot.rs` enforces `openapi.json`. | [DONE](src/server/openapi.rs#L60) |
| FR-709 | Every 4xx/5xx response SHALL serialise the `{code, message, hint?}` error envelope. | Must | An unresolvable path returns 400 with the envelope shape. | [DONE](src/server/error.rs#L13) |
| FR-710 | Asset-root paths SHALL be sandboxed (reject `..` / absolute; resolved path must descend from asset_root). | Must | A `../etc/passwd` path is rejected. | [DONE](src/server/state.rs#L58) |
| FR-711 | The server SHALL stream live playback status + scrub-to-time via SSE/websocket. | Should | A client receives frame progress as the job runs. | BACKLOG |
| FR-712 | The server SHALL embed a web UI to load captures, pick a cell, configure mapping, and start/stop playback. | Should | The SPA drives the REST endpoints above from the browser. | BACKLOG |

### 2.8 Configuration

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-800 | The tool SHALL load a run-config YAML (serde_yaml). | Must | A `r3-run.yaml` parses into a `RunConfig`. | [DONE](src/config.rs#L117) |
| FR-801 | The run-config SHALL reference the cell YAML + CSV + mapping + topic overrides + rate/loop. | Must | All run inputs are expressible in one config file. | [DONE](src/config.rs#L66) |
| FR-802 | The tool SHALL run from CLI flags alone (no config file), synthesizing defaults. | Must | `run --cell <c> --csv <c>` works with identity mapping. | [DONE](src/config.rs#L97) |
| FR-803 | The tool SHALL layer CLI overrides over a loaded config (flags win). | Must | `apply_cli_overrides` replaces only the set fields. | [DONE](src/config.rs#L150) |

---

## 3. Non-Functional Requirements

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| NFR-100 | The tool SHALL be pure Rust with no native deps — including the core ROS 2 / DDS transport (RustDDS, no rcl/ROS install) and the Zenoh transport (TCP-only, no QUIC/TLS/shm C stacks). | Must | `cargo tree -e normal` contains no C-compiled crates: no `cc`/`bindgen` build steps and no `*-sys` crate that compiles or links C sources. Pure-Rust FFI shims binding the always-present system libc (`libc`, `dirs-sys`, `windows-sys`) are permitted. | [DONE](Cargo.toml) |
| NFR-101 | The tool SHALL build with a plain `cargo build` on macOS arm64 and Linux x86_64 — no build-mode feature flags. | Must | `cargo build` (no flags) produces the full CLI + DDS + server binary on the dev box. | [DONE](src/lib.rs#L1) |
| NFR-102 | Replay SHALL be deterministic: same config + mock clock → identical StdoutSink output. | Must | A golden snapshot of the StdoutSink output is byte-stable. | [DONE](tests/requirements.rs) |
| NFR-103 | The tool SHALL sustain ~83 Hz × 7 robots (≈581 msg/s) with negligible CPU. | Should | The hot loop reuses buffers; no per-frame allocation beyond the command vectors. | [DONE](src/playback.rs#L146) |
| NFR-104 | The tool SHALL never modify input CSV/YAML/URDF files. | Must | Inputs are opened read-only; only stdout / DDS is written. | [DONE](src/csv_intake.rs#L42) |
| NFR-105 | Peak memory SHALL be independent of capture row count. | Must | The frame reader holds at most one row + an O(robots×6) cache. | [DONE](src/csv_intake.rs#L121) |
| NFR-106 | Source SHALL contain no asset-specific cell/tool/fixture names — only test fixtures may. | Must | `src/` has no private cell-name, tool-name, or robot-id literals. | [DONE](src/lib.rs#L1) |

---

## 4. Backlog

Items deferred for later implementation. Every item originates from a requirement.

| ID | Description | Origin |
|----|-------------|--------|
| BL-01 | Long-form `combined_timeseries.csv` intake (`D->M\|A<n>_deg`) | [FR-105](#21-csv-intake) |
| BL-02 | RosbridgeSink (websocket) transport for the browser/WebApp path | [FR-406](#24-transport--publish) |
| BL-03 | SSE/websocket live playback status + scrub-to-time | [FR-711](#27-server--web) |
| BL-04 | Embedded web UI SPA (load captures, pick cell, edit mapping, start/stop) | [FR-712](#27-server--web) |
| BL-05 | CSV upload endpoint (`POST /api/v1/csv/upload`, multipart, sandboxed) | [FR-703](#27-server--web) |
| BL-06 | Server-side DDS sink toggle so a playback job can publish real DDS | [FR-403](#24-transport--publish), [FR-704](#27-server--web) |
| BL-07 | Velocity / effort feed-forward channel (currently position-only) | [FR-404](#24-transport--publish) |
| BL-08 | Zenoh transport as an alternative to RustDDS — realized as [FR-407](#24-transport--publish) (`--sink zenoh` + the `robo-zenoh-bridge` companion crate) | [FR-403](#24-transport--publish) |
