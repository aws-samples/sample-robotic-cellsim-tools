# REST API

The REST server is core (always built). `serve --asset-root <dir>` hosts the API
on port 3050. The authoritative wire schema is [`openapi.json`](../openapi.json) (also
served at `/openapi.json`); Swagger UI is at `/docs`. Every API route is under
`/api/v1`; liveness/readiness probes sit at the root.

Every operation calls the same library function the CLI does — handlers are thin
shells. All `path` fields are sandboxed under `--asset-root` (absolute paths and `..`
segments are rejected). Every 4xx/5xx response is the `{code, message, hint?}` envelope.

## Endpoints

### `POST /api/v1/topics`
Resolve the per-robot `joint_command` topics for a cell manifest.
```json
{ "cell_path": "fitted-robots/demo-cell.yaml", "csv_path": "data-captures/joint_angles_timeline.csv" }
```
→ `{ "cell_name": "...", "topics": [ { "robot_id", "topic", "has_csv_data" }, ... ] }`
(`csv_path` optional; when given, each topic is flagged `has_csv_data`.)

### `POST /api/v1/csv/inspect`
```json
{ "csv_path": "data-captures/joint_angles_timeline.csv" }
```
→ `{ "robots": [...], "axes_per_robot": 6, "row_count": 99774, "t_min": 0.011, "t_max": 419.995 }`

### `POST /api/v1/playback`
Start a playback job (runs off the request thread, publishing via the StdoutSink).
```json
{ "cell_path": "...", "csv_path": "...", "rate": 1.0, "loop_play": false,
  "start_s": null, "end_s": null, "robots": null, "clamp": false }
```
→ `{ "job_id": "<uuid>", "status": "running", "topics": [...] }`

### `GET /api/v1/playback/{id}`
→ `{ "job_id", "status": "running|done|cancelled|failed", "frames_published",
     "messages_published", "clamped_count", "looped", "error?" }`

### `DELETE /api/v1/playback/{id}`
Request cancellation. → the same status shape.

### `GET /healthz`
Always 200 once bound. → `{ "status": "ok", "version": "<semver>" }`

### `GET /readyz`
200 when the asset root canonicalises, 503 otherwise. → `{ "ready", "asset_root", "errors[]" }`

## Notes

- Server-started playback jobs currently publish via the `StdoutSink` (preview). Wiring
  server jobs to the core DDS transport (a per-request sink choice) is tracked in the
  Backlog (BL-06). The CLI `run` already publishes real DDS by default.
- Live status streaming (SSE/websocket) + scrub-to-time, CSV upload, and an embedded
  web UI are Backlog (BL-03/04/05) — this is the REST scaffold those will build on.
