# `robo-fitting-room` Requirements Tracker

> **Status legend**: DONE (linked to source), PARTIAL (linked + remaining gap noted), NOT STARTED, BACKLOG (deferred)

A pure-Rust multi-backend scene composition and validation engine that ingests URDF structures and 2D CAD floor plans to assemble, validate, and deploy simulation scenes. Backend-agnostic at the manifest level — the same scene YAML drives all supported physics engines (Isaac Sim first, MuJoCo and Gazebo planned).

Target: standalone open-source release.

---

## 1. Definitions

| Term | Meaning |
|------|---------|
| **Dressed URDF** | A composite robot+tool URDF produced by `urdf-robo-dresser`. Tool meshes live under `meshes/tool/`, robot meshes under `meshes/robot/`. |
| **Fixture URDF** | A URDF representing static cell infrastructure (conveyor, fixture, fence) with geometry but only fixed joints. |
| **Scene Manifest** | A YAML file declaring which URDFs to place, at what world-space poses, forming a complete production cell. Optionally declares a `scene_origin` to centre placements around. |
| **Floor Plan** | A 2D CAD export (DXF→OBJ) containing line geometry that represents cell boundaries, fences, and robot footprints. Currently parsed but not emitted into USDA (Hydra CAR-811). |
| **Station** | A spatial region in the cell assigned to one robot. Contains reachability targets and collision boundaries. |
| **USD Stage** | The OpenUSD scene graph that Isaac Sim operates on. `robo-fitting-room` programmatically populates this as ASCII USDA, packaged into a zip alongside the manifest. |
| **ActionGraph** | An OmniGraph node network embedded in the USD stage that bridges Isaac Sim physics to ROS2 topics and actions. |
| **Trajectory** | A joint-space waypoint sequence defining a robot's planned motion. Used for offline collision checking. |
| **Streaming Client** | Isaac Sim's Kit Remote API for connecting to a running instance and managing stages programmatically. |
| **Deploy** | The act of pushing a composed USD stage to a running Isaac Sim instance via the streaming client. |
| **Force Threshold** | Minimum contact force (N) below which events are filtered as numerical noise (default: 5 N). |
| **Event Debouncing** | Collapsing continuous per-frame contacts into COLLISION_STARTED / COLLISION_ENDED state transitions. |
| **SimBackend** | A Rust trait abstracting backend-specific logic: scene format conversion, deployment protocol, and ROS2 wiring strategy. |
| **Backend** | A physics engine implementation (Isaac Sim, MuJoCo, Gazebo) that the tool can compose scenes for and deploy to. |
| **Home pose** | The joint-angle vector applied at scene load. Currently hard-coded as a KUKA-style "stowed" pose `[0, -90°, +90°, 0, 0, 0]` and baked into both the link bind transforms (FK pre-computed in compose) and the per-joint drive `targetPosition`. |
| **Steel-arm inertia override** | Per-link mass + diagonal inertia values applied when the URDF link name matches a known KR240 suffix (`_base_link`, `_link_1`..`_link_6`). Overrides the URDF's placeholder inertials so PhysX doesn't NaN-out under stiff drives. |

---

## 2. Functional Requirements

### 2.1 Scene Manifest & Composition

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-100 | The tool SHALL parse a scene manifest YAML file declaring all URDF assets and their world-space placements (position xyz + orientation ABC degrees). | Must | Given a manifest, the tool resolves all referenced URDF paths and reports missing files. | [DONE](src/scene.rs#L116) |
| FR-101 | The tool SHALL support placing multiple dressed robots in a single scene, each at a distinct world-space pose. | Must | A 5-robot cell manifest produces a scene with 5 independent robot placements. | [DONE](src/scene.rs#L141) |
| FR-102 | The tool SHALL support placing static fixture URDFs (fixtures, fences, conveyors) as non-articulated rigid bodies. | Must | Fixture URDFs from `urdf-fixture-collection` are placed at world-space poses; non-fixed joints in fixture URDFs are rejected at compose time. | [DONE](src/compose.rs#L725) |
| FR-103 | The tool SHALL validate the scene manifest for completeness: all referenced URDF paths must exist, positions must be finite, orientations must be valid, URDF files must contain a `<robot>` element. | Must | Invalid manifests produce structured errors before any simulation setup. | [DONE](src/scene.rs#L209) |
| FR-104 | The tool SHALL support scene inheritance: a base manifest can be extended with overlay manifests that add/override placements. | Should | Overlay adds robot without duplicating the full base definition. | NOT STARTED |
| FR-105 | The tool SHALL load a 2D CAD floor plan (OBJ line geometry) into the scene for cell boundary and fence visualization. The floor plan supports offset and scale parameters and separates footprint layers from general geometry. | Must | A floor plan OBJ with `l` elements produces line segments grouped by layer. Parsing supports both `o <group>` directives and `# Layer:` comments. | [PARTIAL](src/floor_plan.rs#L12) |
| FR-106 | The manifest SHALL accept an optional `scene_origin` field that is subtracted from every placement and floor-plan offset, so the composed scene is centred near the world origin for ergonomic viewport navigation. | Must | A manifest with `scene_origin: { x, y, z }` produces robots whose `xformOp:translate` is the original placement minus that origin. | [DONE](src/scene.rs#L135) |
| FR-107 | Floor-plan emission into the composed USDA is currently disabled (Hydra CAR-811 prevented BasisCurves and flat-mesh fallbacks from rendering). The manifest field is still parsed and counted in the report. | Must | Compose runs without emitting any `floor_plan` prim into the USDA; segment counts are reported in `ComposeReport.floor_plan_segments`. | [PARTIAL](src/compose.rs#L222) |

### 2.2 URDF-to-USD Conversion

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-110 | The tool SHALL convert URDF joint/link trees into USD PhysX articulations with correct joint types, limits, and axis definitions. | Must | A 6-DOF revolute arm URDF produces a USD articulation with 6 revolute joints matching the URDF limits (degrees on revolute, metres on prismatic). | [DONE](src/compose.rs#L1268) |
| FR-111 | The tool SHALL convert URDF visual meshes (OBJ) into USD mesh prims with correct relative transforms. Mesh data is inlined per visual (no external references) so Isaac Sim doesn't invoke the asset converter at load time. | Must | Mesh geometry renders correctly in Isaac Sim viewport. | [DONE](src/compose.rs#L1042) |
| FR-112 | The tool SHALL convert URDF collision meshes into USD collision geometry with `physics:approximation = "convexHull"` and `PhysicsCollisionAPI` + `PhysicsMeshCollisionAPI`. | Must | PhysX detects collisions using the URDF-defined collision shapes. | [DONE](src/compose.rs#L1089) |
| FR-113 | The tool SHALL map URDF inertial properties (mass, inertia tensor, CoM) to USD PhysX rigid body mass properties. When the URDF inertial is missing or contains placeholder values, recognised KR240-class links receive realistic steel-arm overrides; unknown orphan links receive a 10 kg / identity-inertia fallback so PhysX never falls back to its "negative mass small sphere" path. | Must | Every rigid body carries `PhysicsMassAPI` with positive mass; KR240 links use the override table. | [DONE](src/compose.rs#L893) |
| FR-114 | The tool SHALL assign deterministic USD prim paths based on the robot ID and link/joint names from the manifest. Names are passed through `sanitize_prim_name` (digit-prefixed names get an underscore, "World" → "world_link" to avoid stage-root shadowing). | Must | Prim paths are predictable and stable across scene reloads. | [DONE](src/compose.rs#L2056) |
| FR-115 | The tool SHALL handle the URDF root link by anchoring it to the USD stage root at the placement pose from the manifest. Implemented as a `PhysicsFixedJoint "world_anchor"` whose `localPos0/localRot0` matches the robot's world placement and whose `body1` rel points at the root link. | Must | Robot base is fixed at the specified world position. | [DONE](src/compose.rs#L684) |
| FR-116 | The tool SHALL pre-compute forward kinematics in compose so each link's bind pose is its world-frame transform at the configured home pose. Avoids Isaac Sim's articulation pose initialiser disagreeing with the per-joint frames (which used to print "disjointed body transforms" every step). | Must | Each link's `matrix4d xformOp:transform` equals `parent_world × joint_local × R(axis, home_angle)`. | [DONE](src/compose.rs#L1971) |
| FR-117 | When a robot does NOT specify a per-robot home pose (FR-118), the tool SHALL fall back to the KUKA stowed default `[0°, -90°, +90°, 0°, 0°, 0°]`. The same angles are written into both the per-link FK bind pose AND the per-joint drive `targetPosition`. | Must | Default-everything manifests still produce the stowed pose. | [DONE](src/compose.rs#L1941) |
| FR-118 | The manifest SHALL accept an optional `home_pose_deg` field per robot — a list of N angles in degrees, one per actuated joint of the dressed URDF (fixed joints excluded, declaration order). The length is validated against the URDF at validate-time; mismatches are rejected with a structured error. The override drives both the FK bind pose and per-joint `targetPosition`. | Must | A manifest with `home_pose_deg: [10, -45, 30, 0, 0, 0]` produces matching `targetPosition` values; a wrong-length list fails validation. | [DONE](src/scene.rs#L231) |
| FR-119 | The manifest SHALL accept an optional per-robot `pedestal` block (`height_mm`, default 0; `diameter_mm`, default 800) describing an elevated mounting fundament. When `height_mm > 0` the robot is raised by that height (mm→m) and a static collision `Cylinder` of the given diameter is emitted beneath it, spanning floor (z=0) to z=height at the robot's XY. Height must be finite and ≥ 0, diameter finite and > 0. | Should | A manifest with `pedestal: { height_mm: 500 }` raises the robot's `xformOp:translate` and `world_anchor` z by 0.5 and produces a `def Scope "pedestals"` with a `def Cylinder` (radius 0.4, height 0.5) carrying `PhysicsCollisionAPI`. | [DONE](src/compose.rs#L879) |

### 2.3 Runtime Tool Swapping

> **Note:** Tool swapping requires a full dresser cycle (`urdf-robo-dresser dress`) to produce a new dressed URDF, followed by a scene recompose and redeploy. There is no in-place USD subtree replacement — the canonical workflow is: re-dress → update manifest → redeploy.

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-120 | The tool SHALL support hot-swapping a robot's tool during simulation runtime by loading a different dressed URDF. | Should | Swap triggers dresser cycle, recompose, and redeploy without manual steps. | BACKLOG |
| FR-121 | The tool SHALL preserve the robot arm's joint state across a tool swap — only the tool subtree is replaced. | Should | After swap, arm joint positions are unchanged; only the tool attachment subtree differs. | BACKLOG |
| FR-122 | The tool SHALL validate the replacement URDF is compatible with the current arm (same flange interface) before executing the swap. | Should | Incompatible swap attempts produce an error without modifying the scene. | BACKLOG |

### 2.4 Spatial Validation

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-130 | The tool SHALL compute forward kinematics for each robot to determine the reachable workspace envelope. | Must | Given joint limits, the tool reports whether defined target poses are reachable. | NOT STARTED |
| FR-131 | The tool SHALL detect static collisions between placed assets at their home poses. | Must | Overlapping geometry at rest produces a warning listing the colliding pairs. | NOT STARTED |
| FR-132 | The tool SHALL validate that a dressed robot fits its designated station: no geometry extends beyond station boundaries. | Should | Station boundary violations are reported with the offending link and overshoot distance. | NOT STARTED |
| FR-133 | The tool SHALL validate reachability of target frames defined in environment URDFs from the dressed robot's workspace. | Should | Unreachable targets produce a report listing the target and closest achievable pose. | NOT STARTED |

### 2.5 CLI & API Interface

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-140 | The tool SHALL provide a `compose` subcommand that builds the full USD stage from a scene manifest without launching simulation. Output is a `.zip` package containing the manifest YAML and an inlined `.usda` file. | Must | Produces a `.zip` file on disk with `<stem>.usda` and the manifest inside. | [DONE](src/main.rs#L22) |
| FR-141 | The tool SHALL provide a `validate` subcommand that checks manifest structure, URDF paths, and placement values. Exits 0 if all checks pass; non-zero with diagnostics otherwise. | Must | Exits 0 if all checks pass; non-zero with diagnostics otherwise. | [DONE](src/main.rs#L17) |
| FR-142 | The tool SHALL provide a `launch` subcommand that composes the scene and starts Isaac Sim with the stage loaded. | Should | Isaac Sim opens with the composed scene ready for simulation. | NOT STARTED |
| FR-143 | The tool SHALL provide a `swap` subcommand that triggers a dresser cycle + recompose + redeploy for tool replacement. | Should | Full re-dress and redeploy cycle completes from a single command. | BACKLOG |
| FR-144 | The tool SHALL expose a Rust library API for programmatic scene parsing, validation, floor-plan loading, and compose. `compose_stage_with_options(scene, manifest, output, &ComposeOptions)` is the primary entry point. | Must | Consumers can integrate without the CLI. | [DONE](src/lib.rs#L14) |
| FR-145 | The compose subcommand SHALL accept `--full-robots` (default false) and `--full-tools` (default true). When `--full-robots=false` the per-robot visual meshes are dropped and the convex collision hulls are surfaced in the viewport (`purpose=default`). When `--full-tools=false` the same applies to tool meshes (path-prefix `meshes/tool/`). Articulations and joints are unaffected by either flag. | Must | Toggling either flag changes the emitted `def Mesh "visual_*"` set without touching the joint/body rels. | [DONE](src/main.rs#L36) |
| FR-146 | The compose output SHALL be a deterministic `.zip` archive containing the manifest YAML and a single inlined `.usda` so the package is self-contained and reproducible across machines. | Must | Two consecutive composes of the same manifest produce byte-identical zip contents (ordering-stable). | [DONE](src/compose.rs#L148) |
| FR-147 | The library SHALL provide `compose_urdf_bundle(scene, manifest, out)` producing a deterministic `.zip` containing the scene YAML (rewritten so `dressed_urdf` / `urdf` / `floor_plan.geometry_file` reference bundled assets by relative path) plus every referenced source URDF — deduplicated by directory, each in its own folder with its `meshes/` subtree copied byte-for-byte — and the floor-plan OBJ when present, so the extracted archive is standalone. This is a SECOND artifact and does not alter the USDA compose path. | Must | Extracting the bundle and running `parse_scene`/`validate_scene` on the bundled YAML resolves every asset with no reference to the original tree; two runs are byte-identical. | [DONE](src/compose.rs#L358) |
| FR-148 | `compose_urdf_bundle` SHALL reject (error) any URDF whose `<mesh>` filename resolves outside the URDF's own directory (absolute path or `..`), because such a reference cannot be captured by a whole-directory copy and would break self-containedness. | Must | A URDF referencing a mesh by absolute path or via `..` fails bundle creation with a structured error naming the URDF and path. | [DONE](src/compose.rs#L533) |
| FR-149 | The compose subcommand SHALL accept `--urdf-bundle` (default false); when set it ALSO writes a second `<output-stem>-urdf.zip` (URDF-source bundle) alongside the USDA zip, without changing the USDA output. | Must | `compose -o cell.zip --urdf-bundle` writes both `cell.zip` and `cell-urdf.zip`. | [DONE](src/main.rs#L111) |

### 2.6 ActionGraph & ROS2 Preparation

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-150 | The tool SHALL generate a per-robot OmniGraph ActionGraph in the USD stage containing ROS2 bridge nodes (JointState publisher, Joint Command subscriber, articulation controller). The FollowJointTrajectory action server is not yet emitted. | Must | Each robot in the composed USD has an ActionGraph with functional ROS2 nodes. | BACKLOG |
| FR-151 | Every ROS 2 / DDS topic emitted by the composer SHALL be namespaced under `<cell_id>/<robot_id>`, where `cell_id` is the manifest's `name` field and `robot_id` is the per-robot manifest field. Both segments are sanitised through `sanitize_ros_segment`: digit-leading values get an `r_` prefix, non-alphanumerics become `_`. | Must | A scene with `name: "Demo Cell"` and a digit-leading `robot_id: "42"` publishes on `/Demo_Cell/r_42/joint_states` and subscribes on `/Demo_Cell/r_42/joint_command`. | [DONE](src/compose.rs#L1554) |
| FR-152 | The composed USD stage SHALL include a cell-scoped simulation clock publisher on the `/<cell_id>/clock` topic. Multiple cells sharing one ROS_DOMAIN_ID stay disjoint. | Must | A scene with `name: "Demo Cell"` publishes on `/Demo_Cell/clock` at simulation rate when physics is running. | [DONE](src/compose.rs#L1546) |
| FR-153 | ActionGraph joint mappings SHALL be derived automatically from the URDF actuated joint names (fixed joints excluded) in declaration order. | Must | Joint names in ROS2 messages match the URDF joint names exactly. | [DONE](src/compose.rs#L1519) |
| FR-154 | The composed PhysicsScene SHALL run at 120 Hz (`physxScene:timeStepsPerSecond = 120`) with `solverPositionIterationCount = 64` and `solverVelocityIterationCount = 4` on each articulation root, and per-joint drive gains of stiffness 1.5e5 / damping 100 / `maxForce` from the URDF effort limit. These match the published Isaac Sim guidance for stable heavy industrial-arm articulations. | Must | An 8-robot scene plays without "Invalid PhysX transform" warnings or per-step drive overshoot. | [DONE](src/compose.rs#L309) |
| FR-155 | The compose output SHALL set GPU broadphase capacities sufficient for an 8-robot dense-fixture scene: `gpuFoundLostPairsCapacity = 8192`, `gpuFoundLostAggregatePairsCapacity = 8192`, `gpuTotalAggregatePairsCapacity = 4096`, `gpuMaxRigidContactCount = 1048576`, `gpuMaxRigidPatchCount = 16384`. | Must | The 8-robot demo cell loads without sustained "needs to increase Capacity" PhysX errors during steady state. | [DONE](src/compose.rs#L324) |

### 2.7 Collision Reporting (Two-Tier Fan-Out)

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-160 | The composed USD stage SHALL include Contact Sensor OmniGraph nodes for each robot link that has collision geometry. Sensors live inside the existing per-robot `ActionGraph_<safe_id>` (no new graph prim) — one `IsaacReadContactSensor` + one `ROS2PublishContactSensor` per collision-bearing link, deriving the link list from the URDF. | Must | Every collision-enabled link publishes contact events to ROS2 at simulation runtime; resulting topics match `/<cell_id>/<robot_id>/contacts/<link>`. | [DONE](src/compose.rs#L1741) |
| FR-161 | Contact Sensor nodes SHALL only publish when `contact == true` — no empty contact arrays per simulation frame. Implemented as a compose-time exec chain: each `ROS2PublishContactSensor.inputs:execIn.connect` points at the matching `IsaacReadContactSensor.outputs:execOut`, NOT at `OnPlaybackTick.outputs:tick`, so the publisher only fires when the sensor reports a contact event. | Must | ROS2 topic is silent when no contacts exist; publishes only on actual collision. | [DONE](src/compose.rs#L1855) |
| FR-162 | Contact sensor wiring SHALL be automatic — derived from the URDF collision geometry without manual configuration. `collect_collision_link_names(&urdf)` walks the URDF and emits one sensor + publisher pair per link with at least one `<collision>` block; no manifest knob, no opt-in. | Must | Adding a robot to the manifest automatically includes contact sensors for all its collision links; doubling the robot count exactly doubles the sensor count. | [DONE](src/compose.rs#L1576) |
| FR-163 | The USD collision geometry SHALL use the convex hull meshes generated by `urdf-robo-dresser` (per-link hulls from the dressed URDF's `<collision>` elements). | Must | PhysX contact sensors operate on the dresser-generated hulls, not simplified primitives. | [DONE](src/compose.rs#L1089) |
| FR-164 | The tool SHALL provide a `monitor` subcommand that connects to a running Isaac Sim instance (via streaming client), subscribes to ROS2 contact topics, and applies force threshold filtering and event debouncing before reporting. | Must | Monitor connects at `--target host:port`, filters noise, and emits COLLISION_STARTED/ENDED transitions. | NOT STARTED |
| FR-165 | The monitor SHALL derive the list of topics to subscribe to from the scene manifest (one `/{robot_id}/contacts` per robot). | Must | No manual topic configuration needed — manifest defines what to monitor. | NOT STARTED |
| FR-166 | The monitor SHALL support a configurable monitoring duration and exit non-zero if any collision event is received. | Must | Usable in CI/CD pipelines to verify collision-free execution. | NOT STARTED |
| FR-167 | The monitor SHALL write collision events to a JSON report and persist them to a configurable time-series database for compliance/audit. | Must | `--output report.json` for file output; DB sink for long-term storage (5-10 concurrent consumers). | NOT STARTED |
| FR-168 | The monitor SHALL apply a configurable force threshold (default: 5 N) to suppress numerical chatter from PhysX micro-contacts. | Must | Contacts with force below threshold are discarded before fan-out. | NOT STARTED |
| FR-169 | The monitor SHALL debounce continuous contacts into COLLISION_STARTED and COLLISION_ENDED state transitions instead of per-frame events. | Must | A sustained collision produces exactly one STARTED and one ENDED event with duration. | NOT STARTED |

### 2.8 USD Deploy (Isaac Sim Streaming Client)

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-170 | The tool SHALL connect to a remote Isaac Sim instance via the Kit Remote streaming client (target running in headless mode with `omni.services.streamclient.webrtc` extension). | Must | Connection to `host:8211` succeeds and reports status. | NOT STARTED |
| FR-171 | The tool SHALL deploy a composed USD stage to the running Isaac Sim session (load stage remotely). | Must | After deploy, the scene is visible in Isaac Sim viewport and physics can be started. | NOT STARTED |
| FR-172 | The tool SHALL support live stage reload via `omni.kit.hotreload` extension on the target (`--reload` flag) without restarting Isaac Sim. | Should | Scene updates take effect without physics state loss on unaffected robots. | NOT STARTED |
| FR-173 | The tool SHALL expose deploy status (connecting, deploying, ready, error) via CLI output and library API. | Must | Status is queryable at any point during the deploy lifecycle. | NOT STARTED |
| FR-174 | The tool SHALL resolve asset paths relative to the git repository root. Both client and target machines must have the repository cloned with identical relative paths. | Must | Manifest with relative paths works when repo is cloned at same location on both machines. | NOT STARTED |
| FR-175 | The tool SHALL validate network connectivity to the target (port 8211 TCP reachable) before attempting deployment, with actionable error messages on failure. | Must | Unreachable target produces clear error identifying the connectivity issue. | NOT STARTED |
| FR-176 | The tool SHALL require matching `ROS_DOMAIN_ID` between client and target for ROS2 topic visibility, configurable via `--ros-domain-id`. | Must | Mismatched domain IDs produce a warning; topics not visible without matching IDs. | NOT STARTED |
| FR-177 | The deploy command SHALL verify that the target Isaac Sim instance has livestream and hotreload extensions active before stage upload. | Must | Missing extensions produce actionable error identifying which extension is unavailable. | NOT STARTED |

### 2.9 Server Mode

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-180 | The tool SHALL provide an HTTP server (axum) behind the `server` feature flag with configurable bind address and port. The server hosts the embedded scene-layout editor SPA + a small REST API over the existing compose pipeline. | Must | `serve --port 3040 --asset-root <dir>` starts and accepts connections; the SPA is served at `/`. | [DONE](src/server/mod.rs#L65) |
| FR-181 | `POST /api/v1/compose` SHALL accept a manifest path under `asset_root` and stream the composed `.zip` back as the response body. The unversioned `POST /api/compose` alias is retained for one release with a `Deprecation: true` response header. | Must | Response Content-Type is `application/zip`; body starts with the `PK` zip header. (Stage-ID registry deferred — BL-19.) | [PARTIAL](src/server/handlers/compose.rs#L68) |
| FR-182 | `POST /api/validate` SHALL validate a manifest body (raw YAML) and return structured diagnostics as JSON. | Must | Returns `{ok, errors[]}` (empty array on success). | [DONE](src/server/handlers/compose.rs#L30) |
| FR-183 | `POST /api/monitor` SHALL start collision monitoring and stream contact events via Server-Sent Events (SSE). `DELETE /api/monitor` stops it. | Must | Client receives real-time collision events as SSE stream. | NOT STARTED |
| FR-184 | The composed `.zip` SHALL be streamable as a binary download via `POST /api/compose`; a stage-ID registry + `GET /api/scene/:id` for re-download is deferred (BL-19). | Must | Zip stream covers the binary download case used by the editor. | [PARTIAL](src/server/handlers/compose.rs#L68) |
| FR-185 | `POST /api/deploy` SHALL compose and deploy to the connected Isaac Sim instance, returning deploy status. | Must | Response includes connection state and available ROS2 topics. | NOT STARTED |
| FR-186 | `POST /api/swap` SHALL trigger a dresser cycle + recompose + redeploy for tool replacement via API. | Should | Response confirms full cycle completion or reports error. | BACKLOG |
| FR-187 | `GET /api/status` SHALL return the current Isaac Sim connection state, deploy state, active monitor state, and list of available ROS2 topics. | Must | Response includes connection state, robot topic list, and monitor status. | NOT STARTED |
| FR-188 | `POST /api/v1/compose-urdf-bundle?manifest_path=…` SHALL stream the URDF-source bundle `.zip` (`application/zip`, `Content-Disposition: attachment; filename="<stem>-urdf.zip"`) built by `compose_urdf_bundle`. The unversioned `/api/compose-urdf-bundle` alias is retained with a `Deprecation: true` header. | Must | Response Content-Type is `application/zip`; body starts with the `PK` zip header and contains the scene YAML + `urdf/<folder>/…` entries. | [DONE](src/server/handlers/compose.rs#L185) |
| FR-190 | The SSE fan-out SHALL use `tokio::sync::broadcast` to support 5-10 concurrent monitoring clients without backpressure on the ROS2 subscriber. | Must | Multiple SSE clients receive debounced collision events independently. | NOT STARTED |
| FR-191 | The server SHALL persist all collision events to a configurable time-series database (e.g., InfluxDB, TimescaleDB) for compliance and post-incident audit. | Must | Events include timestamp, robot_id, colliding pair, peak force, and duration. | NOT STARTED |

### 2.10 2D Scene-Layout Editor (in-binary SPA)

The editor ships **inside** the `robo-fitting-room` binary — `cargo build
--features server` embeds the SPA via `include_dir!`. No separate app, no
sibling crate. The SPA renders a top-down 2D layout client-side; the
server is a thin REST shim over the existing compose pipeline.

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-200 | The editor SHALL render a top-down 2D scene layout with drag-and-drop placement of robots and fixtures over an optional floor plan. Pan, zoom, and selection all work in browser without server round-trips. | Must | Loading the demo manifest renders all 8 robots + 2 fixtures on the floor plan; drag updates `position.{x,y}` in the in-memory state. | [DONE](web/js/canvas.js#L8) |
| FR-201 | The editor SHALL load floor plans by uploading an OBJ file (multipart) which the server stores under `<asset_root>/floor-plans/`. (Client-side DXF parsing via `dxf-viewer` is tracked under BL-22.) | Must | Uploaded OBJ becomes selectable in the palette; `geometry_file` references the stored relative path. | [PARTIAL](src/server/handlers/assets.rs#L60) |
| FR-202 | The editor SHALL expose a form-based inspector — every numeric parameter is `<input type="number">` with explicit step + units. Per-robot sliders are explicitly NOT used; values are set as exact decimals. | Must | Inspector renders one numeric field per joint for the home pose, derived from `/api/urdf/joints`. | [DONE](web/js/inspector.js#L8) |
| FR-203 | The Generate-USDA modal SHALL expose an optional "Also download URDF source bundle" checkbox (default off); when checked, after downloading the USDA zip the editor issues a second request to `/api/compose-urdf-bundle` and downloads the resulting `<name>-urdf.zip`. | Must | Checking the box and generating downloads two files: the USDA zip and the URDF-source bundle. | [DONE](web/index.html#L88) |
| FR-204 | The compose API and the SPA SHALL run from the same `robo-fitting-room` binary built with `--features server`. The `Serve` subcommand boots both the REST API and the embedded SPA on the same port. | Must | `cargo run --features server -- serve --asset-root ../assets` exposes the editor at `http://localhost:3040/`. | [DONE](src/main.rs#L53) |
| FR-205 | The server SHALL publish a generated OpenAPI 3.x spec at `GET /openapi.json` and a Swagger UI at `GET /docs` so the REST surface is self-describing. The spec is built from `#[utoipa::path(...)]` decorators on every handler plus the schema list in `ApiDoc`. A checked-in snapshot at `robo-fitting-room/openapi.json` is enforced by `tests/openapi_snapshot.rs`; intentional changes are refreshed via `RFR_UPDATE_SNAPSHOTS=1 cargo test --features server --test openapi_snapshot`. | Should | `curl http://localhost:3040/openapi.json` returns a valid OpenAPI document with all 12 versioned paths; `/docs` renders Swagger UI; the spec only lists `/api/v1/*` and the root probes. | [DONE](src/server/openapi.rs#L24) |
| FR-206 | The server SHALL provide a YAML round-trip pair: `POST /api/scenes/load` returns the raw manifest YAML at `<asset_root>/<path>`, and `POST /api/scenes/save` writes a `{path, yaml}` JSON body verbatim under `<asset_root>`. Both reject any path containing `..`. | Must | The "Save" button writes a manifest under `fitted-robots/` and a subsequent load returns the byte-identical YAML. | [DONE](src/server/handlers/scene.rs#L11) |
| FR-207 | All asset-root-rooted paths accepted by REST handlers SHALL be sandboxed: absolute paths and any segment containing `..` are rejected; the resolved canonical path must descend from `<asset_root>`. | Must | `POST /api/urdf/joints {"path":"../etc/passwd"}` returns 4xx and never reads outside `<asset_root>`. | [DONE](src/server/state.rs#L26) |
| FR-208 | The server SHALL expose `POST /api/urdf/joints {"path":<rel>}` returning the URDF's actuated (non-Fixed) joints in declaration order, including name, axis, and degree-converted lower/upper limits. The editor uses this to derive the per-joint home-pose form. | Must | A KR240-style URDF returns 6 entries; revolute axis tokens come back canonicalised via `JointType`. | [DONE](src/server/handlers/urdf.rs#L15) |
| FR-209 | The manifest's `floor_plan` block SHALL accept an optional `cell_origin_dxf` companion to `offset`. When set without `offset`, parse_scene derives `effective_offset = scene_origin? - cell_origin_dxf` so the chosen DXF point lands at the world origin. When both are set, `offset` wins (canonical math input). | Must | A manifest with `cell_origin_dxf: { x: 100, y: 50, z: 0 }` and no `offset` produces floor-plan segments shifted by `-cell_origin_dxf` relative to the world origin. | [DONE](src/scene.rs#L184) |
| FR-210 | The server SHALL expose Kubernetes-shaped liveness + readiness probes at the **root**: `GET /healthz` (always 200, returns `{status:"ok", version:"<semver>"}`) and `GET /readyz` (200 with `ready:true` once the asset_root canonicalises and ≥1 URDF is discoverable; 503 with the typed error envelope otherwise). | Must | `kubectl exec` curl against `/healthz` returns 200 from the moment axum binds; `/readyz` returns 503 until the asset directory is mounted, then flips to 200. | [DONE](src/server/handlers/health.rs#L23) |
| FR-211 | Every 4xx / 5xx response from the API SHALL serialise the typed `ErrorEnvelope` body `{code, message, hint?}` (machine-readable token, human message, optional remediation). Bare strings or untyped JSON are forbidden. | Must | A request to `/api/v1/urdf/joints` with an unresolvable path returns 400 with a JSON body matching `ErrorEnvelope`'s schema. | [DONE](src/server/error.rs#L11) |
| FR-212 | All emitted ROS 2 / DDS topics and namespaces SHALL pass `rmw_validate_namespace` and `rmw_validate_topic_name` so Isaac Sim can run with either FastDDS (default) **or** Cyclone DDS (`RMW_IMPLEMENTATION=rmw_cyclonedds_cpp`) without bridge-side validation errors. Each segment matches the rmw token regex `[A-Za-z_][A-Za-z0-9_]*`; this is the contract `sanitize_ros_segment` enforces. | Must | A composed USDA's every `inputs:nodeNamespace` and `inputs:topicName` value parses as a slash-separated list of rmw tokens with no `//`, no leading `/`, no empty segments. | [DONE](src/compose.rs#L1554) |
| FR-213 | The cell SHALL surface a cell-wide collision-event stream so downstream consumers can observe contact activity for the whole cell from a single subscription. | Must | A subscriber sees one ROS 2 topic per cell carrying contact events as the cell runs. | NOT STARTED |
| FR-214 | Any future ROS message schema published by the toolchain SHALL evolve via additive minor / breaking-major versioning so older subscribers keep working across additive revisions and breaking changes are surfaced as a distinct major version. | Must | An additive revision of a published schema does not break a subscriber compiled against the previous minor; a breaking revision is exposed under a new major identifier alongside the previous one for at least one release. | NOT STARTED |

---

## 3. Non-Functional Requirements

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| NFR-100 | The tool SHALL be implemented in pure Rust with no native USD/OpenUSD or PhysX C++ dependencies. The pipeline emits ASCII USDA text directly. | Must | Builds with `cargo build` on a clean checkout, no cmake / libclang / Python / OpenUSD SDK required. | [DONE](src/lib.rs#L1) |
| NFR-101 | Scene composition for an 8-robot cell (each with ~200 mesh references) SHALL complete in under 5 seconds on a developer laptop. | Should | The 8-robot demo cell composes in well under 5 s in release mode. | [DONE](src/compose.rs#L148) |
| NFR-102 | Runtime tool swap SHALL complete in under 200ms (USD stage manipulation only). | Should | Measured from swap API call to stage consistency. | NOT STARTED |
| NFR-103 | The tool SHALL run on Linux x86_64 and macOS arm64 for compose+validate. Deploy to Isaac Sim is currently Windows-targeted because the target machine runs Isaac Sim. | Must | `cargo test` passes on macOS arm64 (current dev box). | [DONE](src/lib.rs#L1) |
| NFR-104 | The tool SHALL accept only URDF as 3D articulated/rigid body input. 2D floor plans (OBJ line geometry) are allowed as non-physical reference overlays. No direct USD or STL scene input. | Must | Non-URDF 3D file arguments produce an error. Floor plan OBJ accepted only for line geometry. | [DONE](src/scene.rs#L282) |
| NFR-105 | The tool SHALL produce deterministic USD output: identical manifests yield identical stage structure (prim ordering, attribute ordering, material registry, mesh cache). | Must | Two consecutive composes of the same manifest produce byte-identical zip contents. | [DONE](src/compose.rs#L148) |
| NFR-106 | The tool SHALL not modify source URDF or mesh files. | Must | Input files are read-only; only the output zip is written. | [DONE](src/compose.rs#L148) |
| NFR-107 | The tool SHALL fail with actionable errors when required network ports are unreachable (8211/TCP for Kit Remote, 3040/TCP for serve mode, UDP 7400-7500 for ROS2 DDS discovery). | Must | Error messages identify the specific port and suggest remediation. | NOT STARTED |
| NFR-108 | The composed `.usda` SHALL match the layout convention of NVIDIA's `flat-scene.usda` template (header `customLayerData` / `Render` / `Environment` / `FlatGrid` blocks) so loading into Isaac Sim feels native. | Must | A handcrafted base USDA loads identically to a composed one. | [DONE](src/compose.rs#L345) |

---

## 4. Backlog

Items not yet prioritized for implementation:

| ID | Description | Origin |
|----|-------------|--------|
| BL-01 | Runtime tool swap: re-dress → recompose → redeploy as single operation (requires orchestrating urdf-robo-dresser externally) | FR-120, FR-121, FR-122 |
| BL-02 | `swap` CLI subcommand wrapping the full dresser+redeploy cycle | FR-143 |
| BL-03 | `POST /api/swap` REST endpoint for tool swap cycle | FR-186 |
| BL-04 | **MuJoCo backend**: implement `SimBackend` trait for MuJoCo — URDF→MJCF conversion, actuator mapping, `mujoco_ros2` bridge for ROS2 control, MuJoCo server deploy via socket API | — |
| BL-05 | **Gazebo backend**: implement `SimBackend` trait for Gazebo/Ignition — URDF→SDF conversion, `ros_gz_bridge` wiring, Gazebo transport deploy, native ROS2 plugin integration | — |
| BL-06 | **SimBackend trait**: define the Rust trait interface covering `compose()`, `deploy()`, `monitor()`, `status()` — each backend implements this independently | — |
| BL-07 | **MuJoCo collision monitoring**: MuJoCo contact data → ROS2 topic via `mujoco_ros2` bridge, same `/{robot_id}/contacts` topic convention | BL-04 |
| BL-08 | **Gazebo collision monitoring**: Gazebo contact sensor plugin → ROS2 topic via `ros_gz_bridge`, same topic convention | BL-05 |
| BL-09 | **Floor-plan visual emission**: workaround for Hydra CAR-811 so the parsed floor plan is actually rendered (flat triangulated mesh OR `BasisCurves` once Hydra fixes are in). Currently disabled in compose. | FR-105, FR-107 |
| BL-10 | ~~**ContactSensor OmniGraph emission**~~ — shipped: `IsaacReadContactSensor` + `ROS2PublishContactSensor` pairs are emitted inside the existing per-robot `ActionGraph_<safe_id>`; FR-160 / FR-161 / FR-162 now DONE. See [src/compose.rs#L1741](src/compose.rs#L1741). | FR-160, FR-161, FR-162 |
| BL-11 | **FollowJointTrajectory action server emission** in the per-robot ActionGraph. Currently the graph carries telemetry + position-command subscribe + articulation controller, but no action-server node. Required to flip FR-150 from PARTIAL → DONE. | FR-150 |
| BL-12 | ~~**ROS2 namespace = robot_id verbatim**~~ — superseded. FR-151 was amended to require a two-segment `<cell_id>/<robot_id>` topology with sanitisation rules; no further verbatim-namespace work needed. | FR-151 |
| BL-13 | **Generic per-link inertia** — `steel_arm_inertia` only matches KUKA KR240 link suffixes. Robots from other vendors will fall through to URDF inertials (or the orphan fallback). A density-from-bbox path (mass = `bbox_volume × material_density`) would generalise this without hardcoding. | FR-113 |
| BL-14 | **Per-articulation collision filter group** between `/World/robots` and `/World/fixtures` so future scenes with grazing fixtures don't trigger PhysX broadphase contact storms at home pose. | FR-131 |
| BL-15 | **Optional mesh decimator** — quadric-edge-collapse + vertex-cluster decimators behind `--decimate <ratio> [--decimate-algo qem|quantization]`. Was implemented and reverted; reintroduce once we confirm the mesh-emission topology is the only remaining variable. | NFR-101 |
| BL-16 | ~~**Manifest-level home pose**~~ — resolved by FR-118 (per-robot `home_pose_deg` accepted in YAML, validated against URDF actuated-joint count). | FR-117, FR-118 |
| BL-17 | **SSE collision-event stream** — `POST /api/monitor` opens an SSE channel; `tokio::sync::broadcast` fans out events to 5–10 concurrent clients. | FR-183, FR-187, FR-190 |
| BL-18 | **Time-series DB sink** — persist collision events to InfluxDB / TimescaleDB for compliance/audit. | FR-191 |
| BL-19 | **Stage-ID registry** — `POST /api/compose` returns a stage_id; `GET /api/scene/:id` re-downloads or streams the cached USDA. Today the server streams the zip directly without persisting a registry. | FR-181, FR-184 |
| BL-20 | **Isaac Sim deploy from server** — `POST /api/deploy` boots/streams the composed scene to a running Isaac Sim instance via Kit Remote. | FR-185 |
| BL-21 | **Editor-driven action-server emission** — surface a "Generate FollowJointTrajectory action server" toggle once BL-11 is in the compose pipeline. | FR-150, BL-11 |
| BL-22 | **Client-side DXF support** — wire `dxf-viewer` into the editor so users can load DXF directly. The editor parses the entities client-side, emits OBJ, and POSTs to `/api/assets/floor-plans/upload-obj`. Today only OBJ uploads are accepted. | FR-201 |
| BL-24 | **Cell-level collision aggregator** — DEFERRED. The Track 1 emission keeps per-link Bool topics on `/<cell>/<robot>/contacts/<link>`, which has proven sufficient for current operator dashboards + ad-hoc CLI watchers. A typed cell-wide event stream (CollisionEvent message + Rust aggregator) was scaffolded once and reverted on 2026-06-03; revisit when richer wire data (force/normal/penetration) is required and the per-link feed is no longer sufficient. | FR-213, FR-214, FR-164, FR-168, FR-169 |

---
