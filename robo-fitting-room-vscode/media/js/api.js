// Thin fetch wrappers around the robo-fitting-room REST API.
//
// Unlike the in-binary SPA (which is served same-origin and uses
// relative `/api/...` paths), this webview lives on the vscode-webview://
// origin and must call an absolute server URL. The extension host injects
// that base URL as `window.__RFR_SERVER_URL__`; we hit the stable
// `/api/v1` surface. The server runs `CorsLayer::permissive()`, so these
// cross-origin fetches succeed.

const BASE = (window.__RFR_SERVER_URL__ || "http://localhost:3040").replace(/\/+$/, "");
const API = `${BASE}/api/v1`;

async function ok(resp) {
  if (!resp.ok) {
    const txt = await resp.text();
    throw new Error(`${resp.status} ${resp.statusText}: ${txt}`);
  }
  return resp;
}

export async function listRobots() {
  return (await ok(await fetch(`${API}/assets/robots`))).json();
}

export async function listFixtures() {
  return (await ok(await fetch(`${API}/assets/fixtures`))).json();
}

export async function listFloorPlans() {
  return (await ok(await fetch(`${API}/assets/floor-plans`))).json();
}

export async function urdfJoints(path) {
  return (await ok(await fetch(`${API}/urdf/joints`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  }))).json();
}

export async function floorPlanSegments(path) {
  return (await ok(await fetch(`${API}/floor-plan`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  }))).json();
}

export async function validateYaml(yamlText) {
  return (await ok(await fetch(`${API}/validate`, {
    method: "POST",
    headers: { "Content-Type": "application/x-yaml" },
    body: yamlText,
  }))).json();
}

export async function saveScene(path, yamlText) {
  return (await ok(await fetch(`${API}/scenes/save`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, yaml: yamlText }),
  }))).json();
}

export async function loadScene(path) {
  return (await ok(await fetch(`${API}/scenes/load`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  }))).text();
}

export async function uploadObj(file, filename) {
  const fd = new FormData();
  fd.append("filename", filename);
  fd.append("file", file);
  return (await ok(await fetch(`${API}/assets/floor-plans/upload-obj`, {
    method: "POST",
    body: fd,
  }))).json();
}

// NOTE: compose + compose-urdf-bundle are intentionally NOT called from
// the webview. Those responses can be 100+ MB zips; streaming them through
// the webview and back out to disk is wasteful and hits postMessage size
// limits. Instead app.js delegates compose to the extension host (Node),
// which fetches the same endpoints and writes straight to a file the user
// picks. See bridge.compose().
