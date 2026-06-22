# REST API

The `serve` subcommand boots an axum-based HTTP server hosting a
versioned REST API over the dressing pipeline. Every CLI operation is
reachable via REST; both transports call the same Rust library
functions, so behaviour is identical.

## Schema-of-record

The authoritative wire schema is the **OpenAPI 3.x spec** at
[`openapi.json`](../openapi.json). Pull it from the running server too:

```sh
curl -s http://localhost:3030/openapi.json | jq '.info.version'   # → "0.1.0"
```

A snapshot test (`tests/openapi_snapshot.rs`) regenerates the spec on
every build and CI fails on drift. Refresh after a deliberate change:

```sh
URD_UPDATE_SNAPSHOTS=1 cargo test --features server --test openapi_snapshot
```

The Swagger UI at `/docs` (loaded from the running server) lets you
click "Try it out" against any endpoint without writing curl.

<!-- SCREENSHOT-NEEDED: Swagger UI at http://localhost:3030/docs with the operations expanded — show the catalogue, dress, validate, batch, jobs, system tag groups. Save under docs/img/swagger-ui.png -->

## Versioning policy

- `/api/v1/...` is the stable contract. Once a release tag ships, the
  paths and DTO shapes don't change in breaking ways.
- New work goes to `/api/v1.<minor>/` for additive endpoints,
  `/api/v2/` for breaking changes.
- Breaking the spec without a version bump is a CI failure (snapshot
  test).

## Authentication

None in v1. The server is intended for trusted networks — local dev,
sidecar pods on a cluster-internal service, or behind a reverse proxy
that handles auth. If you need token auth, add it at the proxy layer
for now.

## Health & readiness

| Path | Method | Returns |
|---|---|---|
| `/healthz` | GET | `200` always once axum binds. JSON `{status:"ok", version:"0.1.0"}` |
| `/readyz` | GET | `200` once `--catalogue` canonicalises and ≥1 robot URDF is discoverable; `503` with the typed error envelope otherwise |

```sh
curl -s http://localhost:3030/healthz | jq .
curl -s http://localhost:3030/readyz  | jq .
```

These probes sit at the **root**, NOT under `/api/v1` — the
Kubernetes convention every probe configurator expects out of the
box.

## Error envelope

Every 4xx / 5xx response carries a typed JSON body:

```json
{
  "code": "robot_not_found",
  "message": "no robot with id 'kr240r2901' under <catalogue>/urdf-robot-collection/",
  "hint": "GET /api/v1/catalogue to list available robot ids"
}
```

- `code` is a stable machine-readable token (no spaces). Matchable in
  client code.
- `message` is human-readable.
- `hint` is optional — only populated when there's an actionable
  remediation pointer.

Documented in the OpenAPI spec under
`components.schemas.ErrorEnvelope`. Bare strings or untyped JSON in
error responses are forbidden and would fail the snapshot test.

## Endpoints

| Path | Method | Tag | Purpose |
|---|---|---|---|
| `/api/v1/catalogue` | GET | catalogue | Combined listing of all robots + tools |
| `/api/v1/catalogue/robots` | GET | catalogue | Robots only |
| `/api/v1/catalogue/tools` | GET | catalogue | Tools only |
| `/api/v1/dress` | POST | dress | Merge one robot + one tool by catalogue ID; returns the dressed URDF |
| `/api/v1/validate` | POST | validate | Validate a URDF by catalogue ID, return diagnostics |
| `/api/v1/batch` | POST | batch | Long-running permutation merge → `202 Accepted` + job id |
| `/api/v1/jobs/{id}` | GET | jobs | Poll batch job status |
| `/api/v1/jobs/{id}/artifact` | GET | jobs | Download finished batch as `application/zip` |
| `/healthz` | GET | system | Liveness probe |
| `/readyz` | GET | system | Readiness probe |

Full request/response schemas are in [`openapi.json`](../openapi.json) and
the Swagger UI at `/docs`.

## Catalogue listing

```sh
curl -s http://localhost:3030/api/v1/catalogue | jq .
```

```json
{
  "robots": [
    {"id": "kr240r2900", "urdf": "kr240r2900/kr240r2900.urdf", "links": 8, "joints": 7}
  ],
  "tools": [
    {"id": "gripper_tool", "urdf": "gripper_tool/gripper_tool.urdf", "links": 1, "joints": 0}
  ]
}
```

## Dress (one-shot)

```sh
curl -s -X POST http://localhost:3030/api/v1/dress \
    -H "Content-Type: application/json" \
    -d '{
        "robot_id": "kr240r2900",
        "tool_id":  "gripper_tool",
        "hull": "medium"
    }' | jq .
```

Response shape:

```json
{
  "dressed_id": "kr240r2900__gripper_tool",
  "urdf": "<full XML body>",
  "auto_orient": {"orient": [90, 180, 90], "normal_dot": 0.910},
  "stats": {"links": 9, "joints": 8, "hulls": 956}
}
```

Fields:

| Field | Type | Notes |
|---|---|---|
| `robot_id` | string | Required. Must match a `catalogue.robots[].id` |
| `tool_id` | string | Required. Must match a `catalogue.tools[].id` |
| `orient` | `[u16, u16, u16]` | Optional. Multiples of 90 in `[0, 270]`. When omitted or all zeros, auto-orient runs |
| `attach_to` | string | Optional. Override flange-link detection |
| `hull` | enum | `tight` / `medium` (default) / `max` |

## Validate

```sh
curl -s -X POST http://localhost:3030/api/v1/validate \
    -H "Content-Type: application/json" \
    -d '{"path": "urdf-robot-collection/kr240r2900/kr240r2900.urdf", "is_tool": false}' | jq .
```

Response is the structured diagnostics array (same shape as the CLI's
exit-code-driven output, but always 200 unless the request itself is
malformed).

## Batch (long-running)

```sh
JOB=$(curl -s -X POST http://localhost:3030/api/v1/batch \
    -H "Content-Type: application/json" \
    -d '{"hull": "medium"}' | jq -r .job_id)

# Poll
curl -s http://localhost:3030/api/v1/jobs/$JOB | jq .
# {"id":"...", "state":"running", "completed":2, "total":4, "errors":[]}

# When state == "completed", grab the zip
curl -s -o all-dressed.zip http://localhost:3030/api/v1/jobs/$JOB/artifact
unzip -l all-dressed.zip
```

Job lifecycle:

| State | Meaning |
|---|---|
| `queued` | Accepted, not yet started |
| `running` | Spawned tokio task in progress; `completed` of `total` updates as pairs finish |
| `completed` | Every permutation merged successfully; artifact ready |
| `failed` | One or more pairs failed unrecoverably; `errors[]` populated; partial artifact still available |

`POST /api/v1/batch` returns `202 Accepted`, NOT `200`. The
`Location:` response header points at `/api/v1/jobs/{id}`.

The `JobRegistry` is in-memory only — restarting the server discards
all job state. For durable batches, use the CLI `batch` subcommand
which writes directly to disk.

## Path sandboxing

Every endpoint that accepts a path resolves it against the configured
`--catalogue`. Absolute paths and any path containing `..` segments
are rejected with `400 invalid_path`. Path-traversal probes return the
typed error envelope, never a directory listing or a 404.

## Concurrency

The server reuses a single `Arc<Catalogue>` across requests; reads
are lock-free. Dress operations run inside `tokio::spawn_blocking`
because URDF parse + hull generation are CPU-bound. Batch jobs spawn
one task per permutation, scheduled across the tokio worker pool.

## Cross-crate use

[robo-fitting-room](../../robo-fitting-room/)'s manifests reference
dressed URDFs by relative path. A typical workflow:

```sh
# 1. Boot the dresser server
urdf-robo-dresser serve --catalogue assets

# 2. Trigger the batch over every robot × tool pair
curl -X POST http://localhost:3030/api/v1/batch \
    -H "Content-Type: application/json" \
    -d '{"hull": "medium"}'

# 3. Poll the job; download the zip; unzip into assets/urdf-dressed-robots/

# 4. Author a manifest in robo-fitting-room's editor referencing the dressed paths
# 5. Compose a USDA scene → Isaac Sim
```

Issuing dress jobs straight from the fitting-room SPA against a
sibling dresser pod is on the roadmap (cross-crate `DresserClient`)
but not in v1.
