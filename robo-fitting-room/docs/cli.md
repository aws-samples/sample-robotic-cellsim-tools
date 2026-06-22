# CLI reference

`robo-fitting-room` ships three subcommands: `validate`, `compose`,
`serve`. The first two are one-shot CLI; `serve` is the long-running
REST + SPA server (covered in [`api.md`](api.md)). Default `cargo
build` produces all three; `--no-default-features` drops `serve`.

```
$ robo-fitting-room --help
robo-fitting-room — manifest-driven scene composition + 2D scene editor

  CLI (one-shot):    validate / compose a manifest, exit.
  Server (long-run): host the SPA + REST API, drive Isaac Sim.

Commands:
  validate  [CLI] Validate a scene manifest (YAML structure + URDF paths + placement values)
  compose   [CLI] Compose a manifest into a USDA stage zip ready to load in Isaac Sim
  serve     [SERVER] Boot the HTTP server: 2D editor SPA at /, REST API at /api/v1, Swagger UI at /docs
```

Every subcommand drills into `--help` with full per-flag detail and
copy-pasteable examples.

---

## `validate` — check a scene manifest

```sh
robo-fitting-room validate ../assets/fitted-robots/demo-cell.yaml
```

Output:

```
Scene: "Demo Cell" — 8 robots, 2 fixtures, floor_plan: yes
OK: all checks passed
```

Exits `0` on success, non-zero on validation errors with diagnostics.
Validation covers:

- URDF path existence (every `dressed_urdf` and `fixture.urdf` resolves)
- Finite `position` and `orientation` values (NaN/Inf rejected)
- Valid URDF structure (file contains a `<robot>` element)
- Fixture-only-fixed-joints rule (any non-fixed joint in a fixture URDF fails compose)
- `home_pose_deg` length matches the robot's actuated-joint count

`validate` is read-only — no filesystem writes, no network. Safe to
run in CI on every manifest change.

---

## `compose` — build the simulation scene

```sh
robo-fitting-room compose \
    --manifest ../assets/fitted-robots/demo-cell.yaml \
    --output   ../assets/fitted-robots/demo-cell.zip
```

```
Composing scene "Demo Cell" → ../assets/fitted-robots/demo-cell.zip ...
  Robot robot_01: dressed_urdf = kr240r2900__gripper_tool.urdf
  …
  Fixture station_fixture_a: urdf = station_fixture_a.urdf
  Fixture station_fixture_b: urdf = station_fixture_b.urdf
  Robots: collision hulls only (use --full-robots for visuals)
  Tools: full visual fidelity
Written: ../assets/fitted-robots/demo-cell.zip (8 robots, 2 fixtures, 5557 floor-plan segments)
  Inline meshes: 14498 (7747557 vertices, 15527046 faces)
  Zip: 112845225 bytes (usda: 559365211 bytes)
```

| Flag | Default | Description |
|---|---|---|
| `--manifest` / `-m` | required | Path to the scene YAML |
| `--output` / `-o` | required | Output `.zip` path. Must end in `.zip` |
| `--backend` | `isaac-sim` | Only `isaac-sim` is implemented today |
| `--full-robots` | `false` | Keep full robot visual meshes. Default drops them and surfaces collision hulls (`purpose=default`). Articulation behaviour is identical either way |
| `--full-tools` | `true` | Keep full tool visual meshes. Tools are detected by the `meshes/tool/` path prefix the dresser writes. Set `--full-tools=false` to drop tool visuals |

The output `.zip` always contains exactly two entries: the original
manifest YAML and one ASCII USDA file. Geometry is **inlined** — no
per-mesh references, no asset-converter step at Isaac load time.

### Worked examples

```sh
# Lean default — minimum zip size, viewport shows collision hulls
robo-fitting-room compose -m scene.yaml -o /tmp/scene.zip

# Full visual fidelity — for screenshots / demos
robo-fitting-room compose -m scene.yaml -o /tmp/scene.zip \
    --full-robots --full-tools=true

# Lean robots + tool hulls only — smallest zip, for batch CI
robo-fitting-room compose -m scene.yaml -o /tmp/scene.zip \
    --full-tools=false

# 8-robot demo cell with full fidelity
robo-fitting-room compose \
    -m ../assets/fitted-robots/demo-cell.yaml \
    -o /tmp/demo-full.zip \
    --full-robots --full-tools=true
```

### Visual fidelity matrix (demo cell)

| `--full-robots` | `--full-tools` | Zip size | Inline meshes | What's visible in viewport |
|---|---|---:|---:|---|
| false (default) | true (default) | 113 MB | 14498 | Robot collision hulls + tool visuals |
| false | false | 43 MB | 8809 | Robot + tool collision hulls only |
| true | true | 122 MB | 14554 | Full robot + full tool visuals |
| true | false | mixed | — | Full robot visuals + tool hulls |

---

## `serve` — REST API + 2D editor

```sh
robo-fitting-room serve --asset-root ../assets
```

Defaults:

- Host: `127.0.0.1`
- Port: `3040`
- Web root: embedded SPA bundle (compile-time)

Served paths:

| Path | What |
|---|---|
| `/` | 2D scene-layout editor (SPA) |
| `/docs` | Swagger UI |
| `/openapi.json` | OpenAPI 3.x spec |
| `/healthz` | Liveness probe (always 200) |
| `/readyz` | Readiness probe (200 once asset_root canonicalises and ≥1 URDF discoverable) |
| `/api/v1/...` | Versioned REST surface — see [`api.md`](api.md) |

Override the embedded SPA bundle with files from disk for live JS
editing:

```sh
robo-fitting-room serve --asset-root ../assets --web-root ./web
```

Useful when iterating on the editor's JS without rebuilding the
binary.

---

## Library API

Both transports (CLI and REST) call the same lib functions:

```rust
use robo_fitting_room::{
    parse_scene, validate_scene, compose_stage, compose_stage_with_options,
    ComposeOptions, SceneDefinition,
};

let scene = parse_scene("scenes/demo-cell.yaml".as_ref())?;
let errs = validate_scene(&scene);
assert!(errs.is_empty());

let opts = ComposeOptions { full_robots: false, full_tools: true };
compose_stage_with_options(
    &scene,
    "scenes/demo-cell.yaml".as_ref(),
    "demo-cell.zip".as_ref(),
    &opts,
)?;
```

`compose_stage(scene, manifest, out)` is a convenience wrapper that
calls `compose_stage_with_options` with `ComposeOptions::default()`.

## Testing

```sh
cargo test                                       # all unit + integration tests
cargo test --features "server real-assets"       # full suite incl. real-asset tests
cargo test --test requirements                   # FR/NFR mapping
```

`tests/requirements.rs` maps every requirement in
[REQUIREMENTS.md](../REQUIREMENTS.md) to a function named after its
FR/NFR ID. Implemented requirements have real assertions; pending ones
panic with descriptive `todo!()` — green = implemented, red = pending.
