// Thin fetch wrappers around the /api/v1/* endpoints plus the /assets byte
// route. Every method returns parsed JSON or throws with the server's message.

async function ok(resp) {
  if (!resp.ok) {
    const txt = await resp.text();
    throw new Error(`${resp.status} ${resp.statusText}: ${txt}`);
  }
  return resp;
}

/** GET /api/v1/catalogue -> { robots: [...], tools: [...] }. */
export async function listCatalogue() {
  return (await ok(await fetch("/api/v1/catalogue"))).json();
}

/** GET /api/v1/catalogue/dressed -> [{ name, urdf_rel }]. Already-dressed robots. */
export async function listDressed() {
  return (await ok(await fetch("/api/v1/catalogue/dressed"))).json();
}

/**
 * POST /api/v1/dress. Returns { ok, dressed_urdf_rel, diagnostics }.
 * `dressed_urdf_rel` is catalogue-relative and feeds assetUrl() directly.
 */
export async function dress({ robotId, toolId, attachTo = null, orient = [0, 0, 0], hull = "medium" }) {
  return (await ok(await fetch("/api/v1/dress", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ robot_id: robotId, tool_id: toolId, attach_to: attachTo, orient, hull }),
  }))).json();
}

/** POST /api/v1/validate -> { ok, diagnostics }. */
export async function validate({ path, tool = false }) {
  return (await ok(await fetch("/api/v1/validate", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path, tool }),
  }))).json();
}

/**
 * Map a catalogue-relative path to the /assets byte route. urdf-loader
 * fetches THIS url and resolves the URDF's own ./meshes/*.obj references
 * against it — no per-mesh rewriting needed.
 */
export function assetUrl(rel) {
  return "/assets/" + rel.split("/").map(encodeURIComponent).join("/");
}
