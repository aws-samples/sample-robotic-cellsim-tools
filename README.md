# robotic-cell-sim

Three Rust binaries for assembling, validating, and driving simulation
scenes of multi-robot industrial cells in Isaac Sim. Inputs are
off-the-shelf URDFs; outputs are Isaac Sim 5.1+ USDA stages with
PhysX articulations, ROS 2 topics, and per-link contact telemetry.

```
URDFs ─►  urdf-robo-dresser  ─►  dressed URDFs
                                       │
                  scene manifest YAML  ▼
                                  robo-fitting-room
                                       │
                                       ▼
                                composed .zip  ─►  Isaac Sim 5.1+
                                                          │
                                                          ▼
                                                 ROS 2 topics
                                                          ▲
                          captured joint timeline (CSV)   │
                                                  robo-ros-runner
```

All three binaries ship two modes: a one-shot CLI and a long-running REST
server (with an embedded 2D scene editor in `robo-fitting-room`).
Default `cargo build` produces both — pick the mode at invocation
time. `robo-ros-runner` replays a captured joint-angle timeline onto the
cell's ROS 2 `joint_command` topics, driving the robots in a running sim.

---

## Get going in 5 minutes

You need Rust 1.75+ and (for the runtime sim) Isaac Sim 5.1+.

### 1. Build

```sh
git clone <this repo>
cd robotic-cell-sim
cargo build --release --manifest-path robo-fitting-room/Cargo.toml
cargo build --release --manifest-path urdf-robo-dresser/Cargo.toml
cargo build --release --manifest-path robo-ros-runner/Cargo.toml
# robo-ros-runner needs no feature flags — ROS 2 / DDS + server are core
```

No native USD, PhysX, or Python dependencies. ~3 minutes on a stock
laptop.

### 2. List what's in the catalogue

```sh
./urdf-robo-dresser/target/release/urdf-robo-dresser list \
    --catalogue ./assets
```

Prints every robot URDF and tool URDF the dresser can see. Use this
output to pick robot+tool ids for the next step.

### 3. Dress one robot with one tool

```sh
./urdf-robo-dresser/target/release/urdf-robo-dresser dress \
    --robot ./assets/urdf-robot-collection/kr240r2900/kr240r2900.urdf \
    --tool  ./assets/urdf-tool-collection/gripper_tool/gripper_tool.urdf \
    --output /tmp/dressed.urdf
```

Auto-orients the tool against the robot's flange, generates convex-hull
collision meshes for both, rewrites mesh paths so the output is
self-contained.

### 4. Compose a multi-robot cell into a USDA

```sh
./robo-fitting-room/target/release/robo-fitting-room compose \
    --manifest ./assets/fitted-robots/demo-cell.yaml \
    --output   /tmp/demo-cell.zip \
    --full-robots --full-tools true
```

Reads an 8-robot cell manifest, walks every referenced URDF, emits
one inlined `.usda` + the manifest into a self-contained `.zip`.
~25 s for a full-fidelity 8-robot scene.

### 5. Load into Isaac Sim

```sh
cd /tmp && unzip -o /tmp/demo-cell.zip
# Open Isaac Sim 5.1+, File → Open → /tmp/demo-cell.usda
```

Hit **Play**. The 8 robots load stowed in their home pose. ROS 2
topics start publishing immediately:

```sh
source /opt/ros/humble/setup.bash    # or jazzy
ros2 topic list
# expect:
#   /Demo_Cell/clock
#   /Demo_Cell/<robot>/joint_states
#   /Demo_Cell/<robot>/joint_command
#   /Demo_Cell/<robot>/contacts/<link>     (one per collision-bearing link)
```

Push a robot in the Isaac viewport into a fixture or another robot;
the matching `contacts/<link>` topic flips to `data: true` while the
contact persists.

### 6. Drive a robot from ROS 2

A single pose, by hand:

```sh
ros2 topic pub --once \
    /Demo_Cell/robot_01/joint_command \
    sensor_msgs/msg/JointState \
    "{name: ['joint_a1','joint_a2','joint_a3','joint_a4','joint_a5','joint_a6'],
      position: [0.0, -0.5, 0.5, 0.0, 0.0, 0.0]}"
```

The arm moves to that joint configuration; `/joint_states` continues to
publish the live position at simulation rate.

### 7. Replay a whole captured timeline with `robo-ros-runner`

Instead of one pose, replay a real capture (`assets/data-captures/`) across the
whole cell. Dry-run first (prints the `ros2 topic pub` equivalents, no DDS), then
publish for real (DDS is the default transport):

```sh
# preview the resolved topics + the publish-equivalents
./robo-ros-runner/target/release/robo-ros-runner dry-run \
    --cell ../assets/fitted-robots/demo-cell.yaml \
    --csv  ../assets/data-captures/joint_angles_timeline.csv --once

# real replay to a running Isaac Sim
robo-ros-runner run --config robo-ros-runner/r3-run.example.yaml --ros-domain-id 0
```

The capture is raw KUKA degrees; some axes sit outside the URDF limits, so R3 applies
a configurable per-axis deg→rad mapping with optional `--clamp`. See
[robo-ros-runner/docs/concepts.md](robo-ros-runner/docs/concepts.md).

---

## When to use which binary

| If you want to... | Use... |
|---|---|
| Merge ONE robot URDF with ONE tool URDF | `urdf-robo-dresser dress` |
| Merge every robot × every tool combination | `urdf-robo-dresser batch` |
| Validate a URDF for missing joints / orphan meshes / etc. | `urdf-robo-dresser validate` |
| List the robots & tools a catalogue contains | `urdf-robo-dresser list` |
| Validate a scene manifest YAML | `robo-fitting-room validate` |
| Compose a multi-robot cell into an Isaac-loadable USDA | `robo-fitting-room compose` |
| Edit a cell layout in a 2D drag-drop editor | `robo-fitting-room serve` then open `http://localhost:3040/` |
| See which ROS 2 topics a cell drives | `robo-ros-runner topics --cell <yaml>` |
| Inspect a captured joint-angle CSV | `robo-ros-runner inspect-csv --csv <csv>` |
| Replay a captured timeline onto a running sim | `robo-ros-runner run` (publishes real DDS by default) |
| Drive any tool from another service via REST | `<tool> serve --catalogue|asset-root …` then hit `/api/v1/…` |
| See an interactive REST docs / try requests in a browser | open `http://localhost:30{30,40,50}/docs` (Swagger UI) |

Every CLI subcommand drills into its own help with copy-pasteable
examples:

```sh
./robo-fitting-room/target/release/robo-fitting-room compose --help
./urdf-robo-dresser/target/release/urdf-robo-dresser dress --help
```

---

## Editing a cell layout in the 2D editor

```sh
./robo-fitting-room/target/release/robo-fitting-room serve \
    --asset-root ./assets
# open http://localhost:3040/ in a browser
```

Drag robots and fixtures from the palette onto the canvas, position
them on the floor plan, set per-robot home poses in the inspector,
click **Generate USDA** to download a recomposed zip. The same REST
endpoints the SPA uses are documented at
[`http://localhost:3040/docs`](http://localhost:3040/docs) (Swagger
UI) and the spec is at
[`http://localhost:3040/openapi.json`](http://localhost:3040/openapi.json).

---

## Driving the toolchain from another service

Both binaries expose a versioned `/api/v1/` REST surface. Same
operations as the CLI, JSON in/out, OpenAPI 3.x spec checked into the
crate root.

```sh
# fitting-room — list URDFs, validate a manifest, compose, save/load scenes
./robo-fitting-room/target/release/robo-fitting-room serve --asset-root ./assets &
curl -s http://localhost:3040/healthz | jq .
curl -s http://localhost:3040/api/v1/assets/robots | jq .

# dresser — list catalogue, dress, validate, batch jobs
./urdf-robo-dresser/target/release/urdf-robo-dresser serve --catalogue ./assets &
curl -s http://localhost:3030/healthz | jq .
curl -s http://localhost:3030/api/v1/catalogue | jq .
```

Per-endpoint detail with curl recipes:
[fitting-room API](robo-fitting-room/docs/api.md) ·
[dresser API](urdf-robo-dresser/docs/api.md).

---

## Repo layout

```
robotic-cell-sim/
├── README.md                       ← this file
├── IDEA.md                         ← original design notes
├── urdf-robo-dresser/              ← Rust crate: URDF merger
├── robo-fitting-room/              ← Rust crate: scene composer
│   ├── REQUIREMENTS.md             ← FR/NFR tracker (single source of progress)
│   └── web/                        ← embedded 2D editor SPA
├── robo-ros-runner/                ← Rust crate: CSV → ROS 2 joint_command replay (R3)
│   ├── REQUIREMENTS.md             ← FR/NFR tracker (single source of progress)
│   └── r3-run.example.yaml         ← sample run config
└── assets/
    ├── urdf-robot-collection/      ← bare robot URDFs (vendor-specific)
    ├── urdf-tool-collection/       ← bare tool URDFs (vendor-specific)
    ├── urdf-fixture-collection/    ← static cell fixtures (vendor-specific)
    ├── urdf-dressed-robots/        ← outputs of `dress` / `batch`
    ├── floor-plans/                ← 2D CAD exports (DXF→OBJ)
    ├── fitted-robots/              ← scene manifest YAMLs (e.g. demo-cell.yaml)
    └── data-captures/              ← joint-angle timeline CSVs + ros-topics.txt
```

`assets/` contents are vendor-specific and won't ship with the
open-source release. The toolchain is asset-agnostic — point it at any
catalogue with the same directory shape.

---

## Build flavours

| Build invocation | Binary contains | Use this when |
|---|---|---|
| `cargo build --release` | CLI **and** REST server (default) | Most cases. One binary, both modes. |
| `cargo build --release --no-default-features` | CLI only — no axum/tokio/utoipa | Slim build for size-sensitive deployments / CI smokes. |
| `cargo build --release --features real-assets` | + the `real_assets.rs` integration tests pass against `./assets/` | Local + CI runs that have the vendor URDFs available. |

---

## Documentation

Each crate's `docs/` is the in-depth reference:

- **robo-fitting-room** —
  [README](robo-fitting-room/README.md) ·
  [CLI reference](robo-fitting-room/docs/cli.md) ·
  [REST API](robo-fitting-room/docs/api.md) ·
  [Concepts](robo-fitting-room/docs/concepts.md) ·
  [Manifest schema](robo-fitting-room/docs/manifests.md) ·
  [USDA layout](robo-fitting-room/docs/usda.md) ·
  [`openapi.json`](robo-fitting-room/openapi.json) ·
  [Requirements tracker](robo-fitting-room/REQUIREMENTS.md)
- **urdf-robo-dresser** —
  [README](urdf-robo-dresser/README.md) ·
  [CLI reference](urdf-robo-dresser/docs/cli.md) ·
  [REST API](urdf-robo-dresser/docs/api.md) ·
  [Concepts](urdf-robo-dresser/docs/concepts.md) ·
  [`openapi.json`](urdf-robo-dresser/openapi.json) ·
  [Requirements tracker](urdf-robo-dresser/REQUIREMENTS.md)
- **robo-ros-runner** —
  [README](robo-ros-runner/README.md) ·
  [CLI reference](robo-ros-runner/docs/cli.md) ·
  [REST API](robo-ros-runner/docs/api.md) ·
  [Concepts](robo-ros-runner/docs/concepts.md) ·
  [`openapi.json`](robo-ros-runner/openapi.json) ·
  [Requirements tracker](robo-ros-runner/REQUIREMENTS.md)

Project design notes: [`IDEA.md`](IDEA.md).

## License

Dual-licensed under MIT or Apache-2.0. See each crate's
`Cargo.toml` for details.

---

## Appendix A — EKS deployment sequence

> The deployment story for running `robo-fitting-room`,
> `urdf-robo-dresser`, and Isaac Sim as Kubernetes services on EKS,
> sharing storage with an Engineering Development Hub neighbour.
> Operator-facing — skip if you're just running the binaries on a
> developer laptop.

### Context

The robotic-cell-sim toolchain has three runtime components that need to live in EKS:

1. **`urdf-robo-dresser`** — has `serve` mode (axum + REST API over the dress/batch/validate/catalogue pipeline; headless, no SPA). Ready to run as a long-lived k8s Service.
2. **`robo-fitting-room`** — already has `serve` mode (axum + SPA + REST). Needs a Deploy path that calls Isaac.
3. **`isaac-sim`** — runs from the official NVIDIA container on a GPU-enabled node, in headless streaming mode. We add one Carb extension (`omni.cell_sim.fitting_room.agent`) that exposes a small HTTP control plane.

**Focus of this document: stable, versioned REST APIs as the deployment contract.** The binaries and Helm charts are implementation details; the wire-level API is what other systems integrate against. Both Rust services publish OpenAPI 3.x specs and pin a `/api/v1/...` URL prefix.

**Hard constraint — dual-mode binaries.** Both `urdf-robo-dresser` and `robo-fitting-room` MUST continue to work as standalone CLI binaries (offline / one-shot / scripted use) AND as long-lived servers exposing the same operations over REST. The CLI surface is the existing one (`dress`, `batch`, `validate`, `list` for dresser; `compose`, `validate` for fitting-room) and stays first-class. The `serve` subcommand is purely additive: same library functions, different transport. There must be exactly one implementation of every operation — both transports call the same `pub fn` in `lib.rs`.

This file describes the deployment *sequence* — the order in which artefacts are built, images are published, k8s manifests are applied, and APIs are smoke-tested.

---

### Step 0 — Preconditions

The EKS cluster is deployed **adjacent to an existing AWS Labs [Engineering Development Hub (EDH)](https://github.com/awslabs/engineering-development-hub) stack** in the same VPC (or a peered VPC). EDH already provides:

- An **FSx for NetApp ONTAP** filesystem with a Storage Virtual Machine and at least one volume exposing both NFS v3/v4 and SMB. This becomes the shared `scenes/` + `assets/` volume for the cluster — no separate EFS provisioning.
- A managed bastion / dev-desktop fleet that can mount the same FSx volumes (engineers edit URDFs / floor plans on EDH desktops; the cluster sees the same files instantly).
- Networking, IAM baselines, and centralised logging that the new EKS cluster reuses.

| Item | Why |
|---|---|
| Existing EDH stack with FSx ONTAP filesystem + SVM in the same/peered VPC | Storage source — we don't provision new FSx |
| New EKS cluster ≥1.28, in EDH's VPC (or peered with route + SG rules to FSx) | Baseline |
| One GPU node group (e.g. `g5.xlarge` or `p3.2xlarge`), labelled `workload=isaac` and tainted `nvidia.com/gpu=true:NoSchedule` | Isaac pod scheduling |
| NVIDIA k8s device plugin DaemonSet installed | Exposes `nvidia.com/gpu` resource |
| **NetApp Trident CSI** installed in the cluster, configured against EDH's FSx ONTAP SVM | Dynamic ONTAP-backed PVCs (NFS RWX) |
| `aws-load-balancer-controller` + an Ingress class | SPA + WebRTC viewport entry |
| ECR repos: `urdf-robo-dresser`, `robo-fitting-room`, `omni-cell-sim-fitting-room-agent` (the Kit ext base image) | Image registry |
| ConfigMap `ros-config` with `ROS_DOMAIN_ID` and a default FastDDS unicast `XML` | Cross-pod DDS discovery (multicast doesn't traverse pod networking) |
| TURN server reachable from browsers (cluster-internal coturn or hosted) | Browser↔Isaac WebRTC viewport |

If any of these aren't in place, that's the first PR — tracked under `infra/` and out of scope for the application code below.

#### Networking notes for the EDH neighbour

- The EKS cluster's worker security group must be added to the FSx ONTAP filesystem's SG inbound rules on TCP 111, 635, 2049 (NFS). EDH's terraform exposes the FSx SG ID as an output — reuse it.
- If EKS is in a peered VPC (not EDH's), the FSx ENI subnets must have route-table entries to the EKS VPC CIDR and vice versa. Same SG rules still apply.
- EDH's existing IAM roles for FSx access become the trust source for Trident's controller IAM role (IRSA) — no new long-lived AWS keys.

---

### Step 1 — `urdf-robo-dresser serve` (built)

The dresser now ships a `serve` subcommand: an axum REST API over the dress/batch/validate/catalogue pipeline (headless, no SPA). The surface below is what it exposes.

#### REST API surface (v1)

All routes prefixed `/api/v1/`. The verb naming mirrors fitting-room for consistency.

| Route | Verb | Purpose | Backed by |
|---|---|---|---|
| `/api/v1/catalogue` | `GET` | List robots + tools available under the `--catalogue` root | `catalogue::scan` (`urdf-robo-dresser/src/catalogue.rs:50`) |
| `/api/v1/dress` | `POST` | Merge one robot + one tool, return dressed URDF + meshes as a zip | `merger::merge` (`urdf-robo-dresser/src/merger.rs:10`) |
| `/api/v1/validate` | `POST` | Validate a URDF body, return structured diagnostics | `validator::validate` (`urdf-robo-dresser/src/validator.rs:8`) |
| `/api/v1/batch` | `POST` | Trigger a batch merge across the catalogue (long-running, returns job id) | `merger::merge` in a loop |
| `/api/v1/jobs/{id}` | `GET` | Poll a batch job's status | new in-memory job registry |
| `/openapi.json` | `GET` | Generated spec | `utoipa` derives |
| `/docs` | `GET` | Swagger UI | `utoipa-swagger-ui` |
| `/healthz` | `GET` | Liveness | static `200 OK` |
| `/readyz` | `GET` | Readiness — confirms catalogue scanned successfully | `catalogue::scan` smoke |

**Wire format:**
- `dress` request: `multipart/form-data` (robot urdf + tool urdf + flags) **or** `{robot_id, tool_id, attach_to, orient, hull}` JSON pointing into the catalogue.
- `dress` response: `application/zip` containing the dressed URDF + `meshes/{robot,tool}/...`. Reuses fitting-room's zip-streaming pattern from [`compose_zip`](robo-fitting-room/src/server/handlers/compose.rs#L68).
- All error responses share a single `{code, message, hint}` envelope.

#### Crate changes

- New module `urdf-robo-dresser/src/server/` mirroring `robo-fitting-room/src/server/`. **All `#[cfg(feature = "server")]`** — default `cargo build` keeps producing a pure-CLI binary with no axum/tokio in the dependency graph:
  - `mod.rs` — router + `run`
  - `state.rs` — `AppState { catalogue_root, http: reqwest::Client, jobs: Arc<RwLock<JobRegistry>> }`
  - `dto.rs` — wire types with `utoipa::ToSchema`
  - `openapi.rs` — `ApiDoc` registration
  - `handlers/{catalogue,dress,validate,batch,jobs,health}.rs` — every handler is a thin shell calling the same `pub fn` the CLI subcommand calls (`merger::merge`, `validator::validate`, `catalogue::scan`). No business logic in handlers.
- `src/main.rs` keeps every existing CLI subcommand (`dress`, `batch`, `validate`, `list`) untouched; the `Serve` variant is gated `#[cfg(feature = "server")]` and runs a `tokio::runtime` block calling `server::run`.
- New `tests/server.rs` (gated on `--features server`) — happy-path tests for each endpoint, modelled on [`robo-fitting-room/tests/server.rs`](robo-fitting-room/tests/server.rs). Existing CLI tests in `tests/requirements.rs` keep running without the feature.

#### Stable-API guarantees

- URL prefix `/api/v1/` is the contract. Breaking changes get `/api/v2/`.
- Every response shape is a `#[derive(serde::Serialize, utoipa::ToSchema)]` struct in `dto.rs` — never an inline `serde_json::json!`. This forces schema changes through code review.
- OpenAPI is exported as a CI artefact (`cargo test` regenerates it; `git diff` flags drift).

---

### Step 2 — Stabilise `robo-fitting-room` REST API

The fitting-room server is already working but currently routes under `/api/...` without a version. Same drift risk.

- Move all existing routes to `/api/v1/...` (compose, validate, scenes, urdf, floor-plan, assets). Keep unversioned aliases for one release, then drop.
- Add `/healthz` + `/readyz` (readyz checks asset_root canonicalised + at least one URDF visible).
- Lock DTOs the same way the dresser does — every handler returns a typed struct.
- **CLI parity is preserved.** `compose` and `validate` subcommands keep working without the `server` feature compiled in. `serve` remains the only feature-gated subcommand. Both transports route through `compose_stage_with_options` / `parse_scene` / `validate_scene` — single source of truth, no logic duplication.
- New v1 endpoints landing in this step (foundations for the Isaac path tracked in the companion plan file):
  - `GET /api/v1/status` — deploy + agent status
  - `POST /api/v1/deploy/check` — agent reachability + extension probe
  - `POST /api/v1/deploy` — compose + push to Isaac (covered in detail in the companion plan)
  - **No matching CLI subcommand for `deploy`** in v1 — the EKS topology assumes the SPA + REST is the user-facing path. A future `robo-fitting-room deploy --isaac-target ...` CLI verb is straightforward to add later (it just calls the same client), but isn't in scope here.

---

### Step 3 — Containerise

Two near-identical Rust Dockerfiles (one per crate). Pattern:

```dockerfile
## stage 1: cargo-chef + builder
FROM rust:1.81-slim AS chef
RUN cargo install cargo-chef
WORKDIR /src

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --features server --recipe-path recipe.json
COPY . .
RUN cargo build --release --features server --bin <crate>

## stage 2: minimal runtime
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /src/target/release/<crate> /usr/local/bin/<crate>
EXPOSE 3040
ENTRYPOINT ["<crate>"]
CMD ["serve", "--host", "0.0.0.0", "--port", "3040"]
```

- Image tags: semver-pinned (`robo-fitting-room:0.1.0`), `latest` floats.
- The Isaac agent image is the official `nvcr.io/nvidia/isaac-sim:<version>` plus a `COPY` of the `omni.cell_sim.fitting_room.agent` Kit extension into the Kit extensions search path. Built separately, much heavier, lives on the GPU node only.

---

### Step 4 — k8s manifests / Helm

A single chart `charts/robotic-cell-sim/` with three Deployments + Services + the shared PVC backed by FSx ONTAP via Trident. Skeleton:

```
charts/robotic-cell-sim/
├── Chart.yaml
├── values.yaml                  # image tags, replica counts, ROS_DOMAIN_ID,
│                                #   trident.storageClass, fsx.svm, fsx.exportPolicy
├── templates/
│   ├── trident-backend.yaml     # TridentBackendConfig pointing at EDH's FSx SVM
│   ├── trident-storageclass.yaml# SC: nas (NFS), reclaim Retain, mediaType ssd
│   ├── pvc.yaml                 # RWX, ReadWriteMany, references the SC above
│   ├── configmap-ros.yaml       # ROS_DOMAIN_ID + FastDDS XML
│   ├── dresser-deployment.yaml  # 2 replicas, no GPU, mounts PVC ro
│   ├── dresser-service.yaml     # ClusterIP :80 → :3041
│   ├── fitting-room-deployment.yaml  # 2 replicas, no GPU, mounts PVC rw
│   ├── fitting-room-service.yaml     # ClusterIP :80 → :3040
│   ├── isaac-deployment.yaml    # 1 replica, GPU node, mounts PVC ro, hostNetwork off
│   ├── isaac-service.yaml       # ClusterIP for the agent control plane
│   ├── isaac-stream-service.yaml # LoadBalancer / Ingress for WebRTC :8211
│   └── ingress.yaml             # ALB → fitting-room SPA at /
```

The FSx volume already exists (provisioned by EDH); Trident's `TridentBackendConfig` just authenticates against the SVM and the `StorageClass` lets Trident dynamically carve PVCs out of it. Engineers on EDH desktops mount the same FSx export over NFS/SMB and see the same `scenes/` directory the cluster reads.

#### Service DNS the components use to reach each other

| From → to | DNS |
|---|---|
| fitting-room → dresser | `dresser.<ns>.svc.cluster.local:80` |
| fitting-room → isaac agent | `isaac-agent.<ns>.svc.cluster.local:<agent-port>` |
| browser → fitting-room SPA | Ingress hostname / TLS via ACM |
| browser → Isaac WebRTC | Separate Ingress + TURN |

#### Pod env

All three pods read `ROS_DOMAIN_ID` and `FASTRTPS_DEFAULT_PROFILES_FILE` from the same ConfigMap so DDS stays consistent. fitting-room additionally reads `ISAAC_TARGET`, `STAGE_ROOT` (PVC mount), `DRESSER_URL`.

---

### Step 5 — Apply order (the actual sequence)

```
0.  Verify EDH neighbour:                 confirm FSx SVM reachable from EKS subnets,
                                          SG rule for NFS in place, Trident CSI healthy
1.  kubectl apply TridentBackendConfig +  → ONTAP backend authenticated
       StorageClass
2.  helm install / kubectl apply        → PVC + ConfigMap          (PVC binds via Trident)
3.  kubectl apply Service for dresser   → DNS exists before fitting-room boots
4.  kubectl apply Service for isaac-agent
5.  kubectl rollout                     → dresser Deployment        (waits for /readyz)
6.  kubectl rollout                     → isaac-sim Deployment      (waits for /ext_check OK)
7.  kubectl rollout                     → fitting-room Deployment   (env points at the two services above)
8.  kubectl apply Ingress for fitting-room (SPA)
9.  kubectl apply Ingress + TURN for Isaac WebRTC viewport
```

**Why the order:** EDH + Trident are the foundations — without a healthy ONTAP backend, every PVC is `Pending` and every pod is stuck. Services exist before the Deployments so DNS is resolvable from the start. fitting-room boots last because its `--isaac-target` and `--dresser-url` flags are validated at startup (TCP probe). If you reverse this, fitting-room CrashLoopBackoffs until the others are up — annoying but not fatal.

`helm install` does steps 1–9 in one shot via templates; the manual `kubectl apply` order above is for ad-hoc cluster bringup or first-time validation. Step 0 (EDH-side checks) is operator-driven and not in the chart.

---

### Step 6 — Smoke verification

After every full apply, run from a workstation with `kubectl` configured:

```bash
## 1. Pods all Ready
kubectl get pods -n robotic-cell-sim
## expect: 2 dresser, 2 fitting-room, 1 isaac-sim, all Running, 1/1 Ready

## 2. Each REST API self-describes
kubectl port-forward svc/dresser 8081:80
curl -s localhost:8081/openapi.json | jq '.info.version'        # → "0.1.0"
curl -s localhost:8081/api/v1/catalogue | jq '. | length'        # → robot count

kubectl port-forward svc/fitting-room 8082:80
curl -s localhost:8082/api/v1/status | jq '.target'              # → ISAAC_TARGET
curl -sX POST localhost:8082/api/v1/deploy/check                 # → {ok:true,...}

## 3. Cross-service call works
curl -sX POST localhost:8082/api/v1/dresser/proxy-list \
  | jq                                                            # fitting-room talked to dresser via cluster DNS

## 4. End-to-end: open SPA in a browser
open https://<ingress-host>/
## Load a manifest, click Deploy, badge → ready
## Open WebRTC viewport in second tab, see the scene loaded

## 5. Live update path (the real value)
## Drag a robot in the SPA, click Deploy again
## Badge cycles connecting → deploying → ready
## Viewport updates (physics resets in v1 — expected)
```

CI runs steps 1–3 against a `kind` cluster on every PR. Step 4–5 is the manual gate before promotion to staging.

---

### Step 7 — API stability process (the focus)

Both Rust services publish their `openapi.json` as a CI artefact. The contract is enforced in three ways:

1. **Spec is checked into the repo.** `cargo test` regenerates `openapi-snapshot.json`; CI fails on `git diff`. Same pattern as `cargo insta`.
2. **`/api/v1/` is frozen** once a release tag ships. New work goes to `/api/v1.<minor>/` for additive endpoints, `/api/v2/` for breaking changes.
3. **Cross-service contract.** fitting-room's dresser client (`src/dresser/client.rs`, new) is generated from the dresser's `openapi.json` so a dresser API change either updates fitting-room's client or fails the build.

This makes the deployment story self-documenting: pull the OpenAPI spec from any of the three services, you know exactly how to drive it.

---

### Critical files (to be created or modified)

**New (dresser):**
- `urdf-robo-dresser/src/server/{mod,state,dto,openapi}.rs`
- `urdf-robo-dresser/src/server/handlers/{catalogue,dress,validate,batch,jobs,health}.rs`
- `urdf-robo-dresser/tests/server.rs`
- `urdf-robo-dresser/Dockerfile`

**Modified (dresser):**
- `urdf-robo-dresser/src/main.rs` — replace the `cmd_serve` stub
- `urdf-robo-dresser/src/lib.rs` — `#[cfg(feature = "server")] pub mod server;`
- `urdf-robo-dresser/Cargo.toml` — add `utoipa`, `utoipa-swagger-ui`, `tower-http` (cors + trace), `reqwest` (already in `tokio`)

**Modified (fitting-room):**
- All route registrations in [`src/server/mod.rs`](robo-fitting-room/src/server/mod.rs) — prefix `/api/v1/`
- New module `src/dresser/client.rs` — typed client against dresser's OpenAPI
- `src/main.rs` — add `--dresser-url` flag

**New (fitting-room):**
- `robo-fitting-room/Dockerfile`
- New handlers for `/api/v1/status`, `/api/v1/deploy/check`, `/api/v1/deploy` (covered in the companion plan)

**New (infra):**
- `charts/robotic-cell-sim/` Helm chart per Step 4
- `infra/eks/` — terraform/eksctl for the cluster + GPU node group + EFS CSI + NVIDIA device plugin (out of scope for this app PR; tracked separately)
- `docs/eks-deploy.md` — operator runbook (this file's user-facing companion)

**Pattern repeated across files:** every new handler is a 3-step ladder — `dto.rs` defines wire types with `utoipa::ToSchema`, `handlers/<verb>.rs` implements the route, `openapi.rs` registers the schemas. Same shape both crates. Reviewer just verifies the three pieces line up.

---

### Out of scope (for this doc)

- **The Isaac stage-update mechanism.** Covered in `propose-a-plan-for-goofy-biscuit.md`.
- **The actual `omni.cell_sim.fitting_room.agent` Kit extension code.** Python, lives in a separate repo, ships as part of the Isaac container image build.
- **Cluster authn/authz.** Assumed: trusted in-cluster traffic between services; Ingress terminates TLS and (optionally) hooks into Cognito/OIDC. If untrusted, add bearer tokens — additive.
- **Observability.** Prometheus scrape annotations on each Service + structured `tracing` logs are easy follow-ups; not in v1.

---

### Open questions before code starts

1. **Helm vs raw kustomize.** Helm is more cluster-friendly long-term; kustomize is simpler for v1. Recommendation: Helm because `values.yaml` makes per-environment (dev/stage/prod) image pinning trivial.
2. **Dresser as a stateless service vs job-runner.** v1 plan above assumes stateless `dress`/`validate` (one-shot per request) plus an in-memory job registry for `batch`. If batches are multi-minute and need to survive a pod restart, jobs need to live on the FSx volume instead. Confirm batch SLA before committing.
3. **Image registry.** ECR private (recommended — already aligned with EDH) vs Artifactory vs ghcr. Affects pull secrets but nothing else.
4. **Kit extension distribution.** Bake into a custom Isaac image (recommended — single artefact) vs mount via a `subPath` from the FSx volume at pod boot. The bake-in version is simpler ops; the mount version makes iterating on the extension faster (no image rebuild) and reuses the EDH FSx integration.
5. **EDH ownership boundary.** Is the Trident install + FSx SVM auth managed by the EDH operators (cluster-as-tenant model) or by the robotic-cell-sim team (cluster-as-product model)? Affects who owns the `TridentBackendConfig` lifecycle and rotation.
6. **VPC topology.** Same VPC as EDH (simplest) vs peered (cleaner blast radius). Same-VPC for v1 unless EDH operators object on noisy-neighbour grounds.
