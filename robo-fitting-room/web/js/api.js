// Thin fetch wrappers around the /api/* endpoints. Every method returns
// either the parsed JSON / text or throws.

async function ok(resp) {
  if (!resp.ok) {
    const txt = await resp.text();
    throw new Error(`${resp.status} ${resp.statusText}: ${txt}`);
  }
  return resp;
}

export async function listRobots() {
  return (await ok(await fetch("/api/assets/robots"))).json();
}

export async function listFixtures() {
  return (await ok(await fetch("/api/assets/fixtures"))).json();
}

export async function listFloorPlans() {
  return (await ok(await fetch("/api/assets/floor-plans"))).json();
}

export async function urdfJoints(path) {
  return (await ok(await fetch("/api/urdf/joints", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  }))).json();
}

export async function floorPlanSegments(path) {
  return (await ok(await fetch("/api/floor-plan", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  }))).json();
}

export async function validateYaml(yamlText) {
  return (await ok(await fetch("/api/validate", {
    method: "POST",
    headers: { "Content-Type": "application/x-yaml" },
    body: yamlText,
  }))).json();
}

export async function saveScene(path, yamlText) {
  return (await ok(await fetch("/api/scenes/save", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, yaml: yamlText }),
  }))).json();
}

export async function loadScene(path) {
  return (await ok(await fetch("/api/scenes/load", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  }))).text();
}

/** Returns a Blob of the composed zip. */
export async function composeZip(manifestPath, opts = {}) {
  const params = new URLSearchParams({
    manifest_path: manifestPath,
    full_robots: String(!!opts.fullRobots),
    full_tools: String(opts.fullTools !== false),
  });
  return (await ok(await fetch("/api/compose?" + params, {
    method: "POST",
  }))).blob();
}

/** Returns a Blob of the self-contained URDF-source bundle zip. */
export async function composeUrdfBundle(manifestPath) {
  const params = new URLSearchParams({ manifest_path: manifestPath });
  return (await ok(await fetch("/api/compose-urdf-bundle?" + params, {
    method: "POST",
  }))).blob();
}

export async function uploadObj(file, filename) {
  const fd = new FormData();
  fd.append("filename", filename);
  fd.append("file", file);
  return (await ok(await fetch("/api/assets/floor-plans/upload-obj", {
    method: "POST",
    body: fd,
  }))).json();
}
