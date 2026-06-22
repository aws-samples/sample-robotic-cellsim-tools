# REST API

The `serve` subcommand boots an in-binary HTTP server hosting the 2D
scene-layout editor SPA + a versioned REST API over the compose
pipeline. Every CLI operation is reachable via REST; both transports
call the same Rust library functions, so behaviour is identical.

## Schema-of-record

The authoritative wire schema is the **OpenAPI 3.x spec** at
[`openapi.json`](../openapi.json) (checked into the crate root). Pull
it from the running server too:

```sh
curl -s http://localhost:3040/openapi.json | jq '.info.version'   # → "0.2.0"
```

Snapshot test: `tests/openapi_snapshot.rs` regenerates the spec on
every build and CI fails on drift. To refresh after a deliberate
change:

```sh
RFR_UPDATE_SNAPSHOTS=1 cargo test --features server --test openapi_snapshot
```

The Swagger UI at `/docs` (loaded from the running server) lets you
click "Try it out" against any endpoint without writing curl.

<!-- SCREENSHOT-NEEDED: Swagger UI at http://localhost:3040/docs with the operations list expanded — show the `assets`, `compose`, `urdf`, `floor-plan`, `scene`, `system` tag groups. Save under docs/img/swagger-ui.png -->

## Versioning policy

- `/api/v1/...` is the stable contract. Once a release tag ships, the
  paths and DTO shapes under `/api/v1` don't change in breaking ways.
- An unversioned `/api/...` alias of every `/api/v1/...` endpoint is
  served for one release as a deprecation cushion, with `Deprecation:
  true` and `Sunset: use /api/v1` response headers.
- New work goes to `/api/v1.<minor>/` for additive endpoints,
  `/api/v2/` for breaking changes. v1 stays alongside for one release.
- Breaking the spec without a version bump is a CI failure (snapshot
  test).

## Authentication

None in v1. Every endpoint is open. The server is intended for a
trusted network — local dev, sidecar pods on a cluster-internal
service, or behind a reverse proxy that handles auth. If you need
token auth, add it at the proxy layer for now; see
[REQUIREMENTS.md](../REQUIREMENTS.md) for the future-API-key plan.

## Health & readiness

| Path | Method | Returns |
|---|---|---|
| `/healthz` | GET | `200` always once axum binds. JSON `{status:"ok", version:"0.2.0"}` |
| `/readyz` | GET | `200` once `asset_root` canonicalises and ≥1 URDF is discoverable; `503` with the typed error envelope otherwise |

```sh
curl -s http://localhost:3040/healthz | jq .
curl -s http://localhost:3040/readyz  | jq .
```

These probes sit at the **root**, NOT under `/api/v1` — that's the
Kubernetes convention every probe configurator expects out of the
box.

## Error envelope

Every 4xx / 5xx response carries a typed JSON body:

```json
{
  "code": "invalid_path",
  "message": "asset path may not contain '..': ../etc/passwd",
  "hint": "POST the manifest body to /api/v1/validate to inspect each error"
}
```

- `code` is a stable machine-readable token (no spaces, no version
  drift). Matchable in client code.
- `message` is human-readable.
- `hint` is optional — only populated when there's an actionable
  remediation pointer.

This shape is documented in the OpenAPI spec under
`components.schemas.ErrorEnvelope`. Bare strings or untyped JSON in
error responses are forbidden and would fail the snapshot test.

## Endpoints

The `/api/v1` surface is documented in the OpenAPI spec
([`openapi.json`](../openapi.json)) including request bodies, query
parameters, response shapes, and per-status error codes. The summary
below is a quick orientation; see Swagger UI at `/docs` for the full
interactive reference.

| Path | Method | Tag | Purpose |
|---|---|---|---|
| `/api/v1/assets/robots` | GET | assets | List dressed robots under `<asset_root>/urdf-dressed-robots/` |
| `/api/v1/assets/fixtures` | GET | assets | List fixture URDFs under `<asset_root>/urdf-fixture-collection/` |
| `/api/v1/assets/floor-plans` | GET | assets | List floor-plan files under `<asset_root>/floor-plans/` |
| `/api/v1/assets/floor-plans/upload-obj` | POST | assets | Multipart upload of a new OBJ floor plan |
| `/api/v1/urdf/joints` | POST | urdf | Parse a URDF and return its actuated joints (used by the editor's home-pose form) |
| `/api/v1/floor-plan` | POST | floor-plan | Parse an OBJ floor plan into segment arrays (used by the editor canvas) |
| `/api/v1/scenes/load` | POST | scene | Load a manifest YAML by path |
| `/api/v1/scenes/save` | POST | scene | Save a manifest YAML by `{path, yaml}` |
| `/api/v1/validate` | POST | compose | Validate a manifest YAML body, return diagnostics |
| `/api/v1/compose` | POST | compose | Compose a manifest into a USDA zip; streams `application/zip` |
| `/healthz` | GET | system | Liveness probe |
| `/readyz` | GET | system | Readiness probe |

## Compose recipe

End-to-end: edit a manifest in the SPA, validate, compose, download
the USDA zip.

```sh
# 1. Save the manifest
curl -s -X POST http://localhost:3040/api/v1/scenes/save \
    -H "Content-Type: application/json" \
    -d '{"path":"fitted-robots/scene.yaml","yaml":"name: \"t\"\n…"}' | jq .

# 2. Validate
curl -s -X POST http://localhost:3040/api/v1/validate \
    -H "Content-Type: application/x-yaml" \
    --data-binary @../assets/fitted-robots/demo-cell.yaml | jq .

# 3. Compose; --output to capture the zip
curl -s -o /tmp/cell.zip \
    "http://localhost:3040/api/v1/compose?manifest_path=fitted-robots/demo-cell.yaml"
unzip -l /tmp/cell.zip
```

The SPA does these three calls (save → validate → compose) when you
click **Generate USDA**. The same flow is callable from any HTTP
client.

## Path sandboxing

Every endpoint that accepts a `path` parameter resolves it against
the configured `--asset-root`. Absolute paths and any path containing
`..` segments are rejected with `400 invalid_path`. This is enforced
in `AppState::resolve_asset` and is the same check applied to URLs
the SPA constructs from manifest references. Path-traversal probes
return the typed error envelope, never a directory listing or a 404.

## Editor app

The SPA at `/` is a plain ES-modules application — no framework, no
build step. Source under [`web/`](../web/). The editor uses the same
REST endpoints documented above plus `GET /` (the SPA itself).

Drag-drop layout, per-robot inspector with home-pose form, floor-plan
upload, click-to-compose. See the screenshots in
[`../README.md`](../README.md) for the workflow.

## Running on Cyclone DDS

The composer's emitted topic schema passes both
`rmw_validate_namespace` and `rmw_validate_topic_name`, so Isaac Sim
can run with either FastDDS (default) or Cyclone DDS:

```sh
export RMW_IMPLEMENTATION=rmw_cyclonedds_cpp
# launch Isaac Sim — every /<cell>/<robot>/joint_states topic resolves
# without bridge-side validation errors
```

The contract is enforced by `tests::ros2_topics_are_cyclone_dds_compatible`
at compile time against a digit-leading manifest stress test.
