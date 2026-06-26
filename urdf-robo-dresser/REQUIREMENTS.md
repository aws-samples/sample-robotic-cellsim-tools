# `urdf-robo-dresser` Requirements Tracker

> **Status legend**: DONE (linked to source), PARTIAL (linked + remaining gap noted), UNTESTED (implemented, test pending), NOT STARTED, BACKLOG (deferred — linked to its `#bl-xxx` entry)

A pure-Rust CLI and library that programmatically merges robot base URDFs with tool/end-effector URDFs from a structured asset catalogue, producing validated composite URDF files.

Target: standalone open-source release.

---

## 1. Definitions

| Term | Meaning |
|------|---------|
| **Asset Catalogue** | The directory structure holding base URDFs, tool URDFs, and their associated mesh files. Convention: `<name>/<name>.urdf` per asset. |
| **Auto-Orient** | The 3-stage algorithm that determines the correct tool mounting rotation by testing 64 candidates and scoring direction alignment, centroid outward distance, and AABB intersection. |
| **Base URDF** | A URDF file describing a robot arm's kinematic chain. Contains joints, links, visual/collision meshes, and inertial properties. The chain terminates at a flange link. |
| **Convex Hull** | A simplified collision geometry computed by wrapping a mesh's vertices in the tightest convex shape. Generated per visual component. |
| **Dressed URDF** | The output: a single valid URDF combining a base robot and a tool, with tool geometry injected into the robot's flange link. |
| **FK Chain** | Forward Kinematics chain — the sequence of joints from robot base to flange, each with origin translation and rotation. Walked at home pose (all joints=0) to compute directions. |
| **Flange Face** | The outermost point of the flange link's mesh along the last joint's axis. This is where the tool docking point connects. |
| **Flange Link** | The terminal link of the robot's kinematic chain where tools are attached. Auto-detected as the deepest leaf link (most hops from root). |
| **Flange Outward** | The direction the robot arm extends at home pose — computed by transforming the last joint's rotation axis through the FK chain. Used for auto-orient scoring. |
| **Hull Buffer** | A scale factor applied to hull vertices (tight=1.0×, medium=1.05×, max=1.15×) to inflate collision boundaries. |
| **MTL** | Wavefront material file (`.mtl`) referenced by OBJ meshes. Contains color (Kd), specular, and texture definitions. Copied alongside OBJ files to preserve colors. |
| **TCP** | Tool Center Point — a reference frame on the tool representing the working point. |
| **Tool Direction** | The normalized vector from the tool's origin (0,0,0) toward the centroid of all its visual positions. Represents the direction the tool "points." |
| **Tool URDF** | A URDF file describing an end-effector or gripper. Contains geometry anchored to a root link that represents the tool's mounting interface. Origin (0,0,0) at flange center. |
| **World Link** | A common root link in URDF files (name="World"). During merge, the tool's World link is discarded and its geometry transferred to the flange link. |

---

## 2. Functional Requirements

### 2.1 Asset Catalogue Management

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-001 | The tool SHALL discover and index all base robot URDFs from a configurable catalogue directory. | Must | Given a catalogue path, the tool lists all `.urdf` files with their robot name and flange link identifier. | [DONE](src/catalogue.rs#L39) |
| FR-002 | The tool SHALL discover and index all tool URDFs from a configurable catalogue directory (`<tool-name>/<tool-name>.urdf` convention). | Must | Given a catalogue path, the tool lists all tool directories and their root URDF files. | [DONE](src/catalogue.rs#L39) |
| FR-003 | The tool SHALL validate that referenced mesh files (`.obj`, `.stl`, `.mtl`) exist relative to each URDF's location at index time. | Must | Missing mesh references produce a warning listing the URDF and missing path. The URDF is still indexed but flagged as incomplete. | [DONE](src/catalogue.rs#L39) |
| FR-004 | The tool SHALL support a catalogue manifest file (TOML) that declares metadata per asset: flange link name, TCP offset, compatible tool interfaces, and mesh directory. | Should | The manifest is optional; without it the tool infers flange link from the kinematic chain terminus. | [BACKLOG](#bl-001) |

### 2.2 URDF Parsing & Validation

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-010 | The tool SHALL parse URDF XML into a typed in-memory kinematic tree representation. | Must | All joints, links, visual/collision geometry, inertial properties, and joint limits are preserved without data loss during round-trip (parse then serialize). | [DONE](src/parser.rs#L11) |
| FR-011 | The tool SHALL validate the kinematic tree is acyclic (a proper tree rooted at a single `World` or base link). | Must | Cyclic or multi-root graphs produce an error identifying the offending links. | [DONE](src/validator.rs#L7) |
| FR-012 | The tool SHALL validate that every `<parent>` and `<child>` link reference in joints resolves to a declared `<link>`. | Must | Dangling references produce an error listing unresolved names. | [DONE](src/validator.rs#L7) |
| FR-013 | The tool SHALL validate joint limit consistency: `lower < upper` for revolute/prismatic joints. | Must | Out-of-range limits produce a warning with the joint name and values. | [DONE](src/validator.rs#L7) |
| FR-014 | The tool SHALL detect and report duplicate link or joint names within a single URDF. | Must | Duplicates produce an error listing the conflicting names. | [DONE](src/validator.rs#L7) |
| FR-015 | The tool SHALL parse all joint types defined in the URDF spec: `revolute`, `continuous`, `prismatic`, `fixed`, `floating`, `planar`. | Must | Unknown type strings produce an error. | [DONE](src/types.rs#L67) |
| FR-016 | The tool SHALL auto-generate per-component collision convex hulls for visual mesh geometry, with configurable buffer (tight/medium/max). | Must | Every link in the output URDF has collision geometry. Hulls generated per visual component with parry3d. | [DONE](src/collision.rs#L37) |
| FR-017 | The tool SHALL validate that tool URDFs have their origin (0/0/0) located at the center of the docking flange interface. | Must | Tools whose root link geometry centroid deviates from origin beyond a configurable tolerance produce an error with the measured offset. | [PARTIAL](#bl-009) |
| FR-018 | The tool SHALL parse MTL material files referenced by OBJ meshes and inject URDF-spec `<material><color rgba="..."/>` elements into each `<visual>`. | Should | Dressed URDF renders with correct colors in viewers that don't read OBJ material files (RViz, Isaac Sim). | [BACKLOG](#bl-005) |
| FR-019 | The tool SHALL compute `<inertial>` properties (mass and inertia tensor) from mesh volume and a user-provided material density (`--density kg/m³`). | Should | Each link in the output has physically realistic mass/inertia derived from geometry, not dummy values. | [BACKLOG](#bl-006) |

### 2.3 Tree Merging (Dressing)

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-020 | The tool SHALL merge a tool URDF into a base URDF by injecting the tool's visual and collision geometry directly into the robot's flange link. The robot's coordinate system and kinematic tree remain unchanged — no extra links or joints are created. | Must | The output URDF has the same links and joints as the base robot. The flange link contains additional visual/collision elements from the tool. | [DONE](src/merger.rs#L9) |
| FR-020a | During merge, the tool geometry SHALL be 3D-translated so that its docking origin aligns with the outermost centerpoint of the robot's end-effector flange. The tool moves to the robot — never the other way around. | Must | The flange face offset is computed from mesh projection along the joint axis. The robot tree is unchanged. | [DONE](src/merger.rs#L9) |
| FR-021 | The tool SHALL apply a configurable additional 6-DOF offset (xyz + rpy) on top of the computed flange-center attachment, defaulting to zero. | Should | When a non-zero offset is specified, it is added to the auto-computed flange-center origin. | PARTIAL |
| FR-021a | The tool SHALL support discrete 90-degree orientation steps for tool attachment around each axis (X, Y, Z). The user selects from {0°, 90°, 180°, 270°} per axis to orient the tool correctly relative to the robot's end-effector. | Should | `--orient 0,90,270` applies the combined rotation. Only multiples of 90° are accepted; other values produce an error. Default is 0,0,0 (auto-orient). | [DONE](src/merger.rs#L9) |
| FR-021b | When `--orient` is not specified, the tool SHALL automatically determine the correct orientation using a 3-stage scoring algorithm: (1) direction alignment with flange outward, (2) centroid extends outward from robot base, (3) minimal AABB intersection. All 64 axis-aligned rotations are tested. | Must | The tool is mounted in the correct orientation without user input. Auto-orient logs the selected rotation and confidence score. | [DONE](src/merger.rs#L9) |
| FR-022 | ~~The tool SHALL namespace-prefix all tool link and joint names.~~ **Superseded** — tool geometry is flattened into the flange link; no separate tool links exist in the output. | N/A | N/A | N/A |
| FR-023 | The tool SHALL rewrite mesh file paths in the tool URDF to be correct relative to the dressed URDF's output location. | Must | All `<mesh filename="...">` attributes in the output resolve to existing files. MTL material files also copied. | [DONE](src/merger.rs#L9) |
| FR-024 | The tool SHALL preserve all inertial properties from both base and tool URDFs without modification. | Must | Inertial `<mass>`, `<inertia>`, and `<origin>` values are byte-identical in the output. | [DONE](src/merger.rs#L9) |
| FR-025 | The tool SHALL set the `<robot name="">` attribute in the output to a deterministic composite name: `{robot_name}__{tool_name}`. | Must | Output file is named `{robot_name}__{tool_name}.urdf`. | [DONE](src/merger.rs#L9) |
| FR-026 | The tool SHALL support attaching a tool to an arbitrary named link (not just the terminal flange), specified via CLI flag or manifest. | Should | When `--attach-to custom_link` is provided, the tool is attached to that link instead of the inferred flange. | [DONE](src/merger.rs#L9) |
| FR-027 | The tool SHALL discard the tool URDF's `World` link during merge and re-parent its children to the attachment point. | Must | No duplicate `World` link appears in the output. The tool's visual/collision geometry under `World` is transferred to the flange link. | [DONE](src/merger.rs#L9) |

### 2.4 Combinatorial Export

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-030 | The tool SHALL support a batch mode that generates dressed URDFs for all valid robot-tool permutations in the catalogue. | Must | Given N robots and M tools, up to N*M output files are produced in the output directory. | [DONE](src/main.rs#L40) |
| FR-031 | The tool SHALL skip invalid combinations (e.g. interface mismatch declared in manifest) and report them. | Should | A summary lists skipped combinations with the reason. | [BACKLOG](#bl-004) |
| FR-032 | The tool SHALL support incremental generation: only re-generate outputs whose source URDFs or configuration have changed since last run. | Should | Unchanged combinations are not re-processed; the tool uses content hashes (SHA-256). | [BACKLOG](#bl-002) |
| FR-033 | The tool SHALL produce a machine-readable index (JSON) of all generated dressed URDFs with metadata: source robot, source tool, output path, generation timestamp. | Should | The index file is written to the output directory root. | [BACKLOG](#bl-003) |

### 2.5 CLI Interface

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-040 | The tool SHALL provide a `dress` subcommand: `urdf-robo-dresser dress --robot <path> --tool <path> --output <path>`. | Must | Produces a single dressed URDF at the specified output path. | [DONE](src/main.rs#L16) |
| FR-041 | The tool SHALL provide a `batch` subcommand: `urdf-robo-dresser batch --catalogue <dir> --output <dir>`. | Must | Produces all valid combinations in the output directory. | [DONE](src/main.rs#L40) |
| FR-042 | The tool SHALL provide a `validate` subcommand: `urdf-robo-dresser validate <path>`. | Must | Exits 0 on valid URDF, non-zero on errors. Human-readable diagnostics on stderr. | [DONE](src/main.rs#L52) |
| FR-043 | The tool SHALL provide a `list` subcommand: `urdf-robo-dresser list --catalogue <dir>`. | Should | Outputs a table or JSON of all discovered robots and tools with status. | [DONE](src/main.rs#L60) |
| FR-044 | The tool SHALL accept a `--config <path>` flag pointing to a TOML configuration file for default paths, naming conventions, and attachment offsets. | Should | CLI flags override config file values. | [BACKLOG](#bl-001) |

### 2.6 Server Mode

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-050 | The tool SHALL provide a `serve` subcommand that starts an HTTP server exposing all operations via a REST API: `urdf-robo-dresser serve --port <port> --catalogue <dir>`. | Must | Server starts, binds to specified port, and responds to API requests. | [DONE](src/main.rs#L489) |
| FR-051 | The REST API SHALL expose endpoints for: catalogue listing (`GET /api/v1/catalogue`), validation (`POST /api/v1/validate`), single dress (`POST /api/v1/dress`), batch export (`POST /api/v1/batch`), and dressed-artifact retrieval (`GET /api/v1/jobs/:id/artifact`). | Must | All CLI operations are available via REST with JSON request/response bodies. | [DONE](src/server/mod.rs#L47) |
| FR-052 | The server SHALL serve a built-in web UI (single-page application) that provides a browser-based interface to all operations. | Must | Navigating to the server root in a browser renders the UI without external dependencies. | NOT STARTED |
| FR-053 | The web UI SHALL include a WebGL-based 3D URDF viewer for previewing and approving dressed robots directly in the browser. | Must | After a dress operation, the user can rotate/zoom/pan the dressed robot in 3D before accepting the output. | NOT STARTED |
| FR-054 | The web UI's 3D viewer SHALL support interactive tool orientation adjustment using the 90-degree stepping controls (FR-021a) and live preview of the result. | Should | Clicking orientation buttons updates the 3D view in real-time without a full re-dress round-trip. | NOT STARTED |
| FR-055 | The web UI SHALL support interactive joint manipulation (sliders per joint axis) in the 3D viewer to inspect the dressed robot's range of motion. | Should | Moving a joint slider updates the 3D pose of all downstream links in real-time. | NOT STARTED |

### 2.7 Visualization (CLI Mode)

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| FR-060 | The CLI `dress` and `validate` subcommands SHALL support an optional `--view` flag that opens a system-native 3D viewer window displaying the resulting URDF. | Must | A native window renders the robot with visual meshes, joint frames, and collision geometry togglable. | [DONE](src/main.rs#L196) |
| FR-061 | The native viewer SHALL support interactive joint manipulation (keyboard or mouse) to inspect range of motion. | Should | User can articulate joints and visually confirm tool clearance. | [DONE](src/main.rs#L197) |
| FR-062 | The native viewer SHALL support toggling visibility of: visual meshes, collision geometry, joint axes, and link frames. | Should | Each layer can be shown/hidden independently. | [DONE](src/main.rs#L197) |

---

## 3. Non-Functional Requirements

| ID | Requirement | Priority | Acceptance Criteria | Status |
|----|-------------|----------|---------------------|--------|
| NFR-001 | The core library SHALL be implemented in pure Rust. The server mode and native viewer may use additional Rust crates with native backends (GPU rendering, HTTP). | Must | `cargo build` succeeds for the core library without external installations. Server/viewer features are gated behind cargo feature flags. | [DONE](Cargo.toml) |
| NFR-002 | The tool SHALL process a single dress operation in under 10 seconds. | Must | Benchmarked on Apple M-series or equivalent x86-64. | [DONE](src/merger.rs#L9) |
| NFR-003 | The tool SHALL produce deterministic output: identical inputs always yield byte-identical output URDFs. | Must | Two consecutive runs produce identical files (verified via SHA-256). | [PARTIAL](#bl-010) |
| NFR-004 | The tool SHALL run on Linux (x86_64, aarch64), macOS (aarch64), and Windows (x86_64). | Must | CI passes on all targets. | [PARTIAL](#bl-011) (macOS verified) |
| NFR-005 | The tool SHALL provide structured error messages with source location and actionable remediation hints. | Should | Errors include file path and suggestion. | [DONE](src/error.rs) |
| NFR-006 | The tool SHALL be usable as a Rust library crate (`urdf_robo_dresser`) in addition to the CLI binary. | Must | Public API exposes: parse, validate, merge, and serialize functions. | [DONE](src/lib.rs) |
| NFR-007 | The tool SHALL not modify or move source asset files. All operations are read-only on inputs. | Must | Source catalogue directory remains unchanged after any operation. | [DONE](src/merger.rs#L9) |
| NFR-008 | The tool SHALL handle URDF files with up to 500 links and 10,000 mesh references without memory errors. | Should | Peak RSS stays below 512 MB. | [UNTESTED](#bl-012) |
| NFR-009 | The crate SHALL have zero `unsafe` blocks. | Should | `#![forbid(unsafe_code)]` in lib.rs. | [DONE](src/lib.rs#L1) |
| NFR-010 | Only URDF XML supported as input/output. SDF, MJCF, USD out of scope. | Must | Non-URDF files produce an error. | [DONE](src/parser.rs#L11) |
| NFR-011 | Tool orientation constrained to 90° increments — arbitrary angles not supported. | Must | Values not in {0, 90, 180, 270} produce an error. | [DONE](src/merger.rs#L9) |
| NFR-012 | Server mode is optional (feature flag). Core library and CLI work without network dependencies. | Must | Default build has no tokio/axum. | [DONE](Cargo.toml) |
| NFR-013 | Tool URDF origin (0/0/0) MUST be pre-aligned to the docking flange center. | Must | The tool enforces this as a precondition, not corrects it. | [PARTIAL](#bl-009) |

---

## 4. Backlog

Items not yet prioritized for implementation:

| ID | Description | Origin |
|----|-------------|--------|
| <span id="bl-001">BL-001</span> | Parse optional TOML manifest per asset (flange link override, TCP offset, interfaces) | [FR-004](#21-asset-catalogue-management), [FR-044](#25-cli-interface) |
| <span id="bl-002">BL-002</span> | Incremental batch generation (SHA-256 hash state file, skip unchanged) | [FR-032](#24-combinatorial-export) |
| <span id="bl-003">BL-003</span> | JSON index of generated dressed URDFs (metadata manifest) | [FR-033](#24-combinatorial-export) |
| <span id="bl-004">BL-004</span> | Skip invalid combinations in batch (interface mismatch detection) | [FR-031](#24-combinatorial-export) |
| <span id="bl-005">BL-005</span> | Parse MTL material files → inject URDF `<material><color rgba="..."/>` per visual | [FR-018](#22-urdf-parsing--validation) |
| <span id="bl-006">BL-006</span> | Compute `<inertial>` (mass + inertia tensor) from mesh volume × user-provided density | [FR-019](#22-urdf-parsing--validation) |
| <span id="bl-007">BL-007</span> | Embed urdf-viz as library crate (remove external binary dependency) | [FR-060](#27-visualization-cli-mode) |
| <span id="bl-008">BL-008</span> | Interactive tool orientation in viewer (rotate in 90° steps live) | [FR-054](#26-server-mode) |
| <span id="bl-009">BL-009</span> | Tool origin validation — compute actual centroid, check tolerance | [FR-017](#22-urdf-parsing--validation), [NFR-013](#3-non-functional-requirements) |
| <span id="bl-010">BL-010</span> | Stable tie-breaking in auto-orient for determinism | [NFR-003](#3-non-functional-requirements) |
| <span id="bl-011">BL-011</span> | CI matrix for Linux/macOS/Windows | [NFR-004](#3-non-functional-requirements) |
| <span id="bl-012">BL-012</span> | Stress test with 500-link URDF | [NFR-008](#3-non-functional-requirements) |
