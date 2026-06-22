// Single in-memory scene mirror. Shape mirrors the Rust SceneYaml types
// in src/scene.rs field-for-field.

export function blankScene() {
  return {
    name: "Untitled",
    description: "",
    scene_origin: null,           // {x,y,z} | null
    robots: [],                    // {robot_id, dressed_urdf, position, orientation, home_pose_deg?}
    fixtures: [],                  // {id, urdf, position, orientation}
    floor_plan: null,              // {geometry_file, offset?, cell_origin_dxf?, scale}
  };
}

export function defaultPosition() {
  return { x: 0, y: 0, z: 0 };
}
export function defaultOrientation() {
  return { a: 0, b: 0, c: 0 };
}

/** KUKA stowed default for the first 6 actuated joints (degrees). */
export function defaultHomePoseDeg(jointCount) {
  const stowed = [0, -90, 90, 0, 0, 0];
  const out = new Array(jointCount).fill(0);
  for (let i = 0; i < Math.min(jointCount, stowed.length); i++) out[i] = stowed[i];
  return out;
}

/** Deep clone via JSON — adequate for our flat data. */
export function cloneScene(s) {
  return JSON.parse(JSON.stringify(s));
}

/** Serialise the in-memory scene back to a YAML-shaped object for save.
 *  - Paths are emitted as authored (no `..` collapsing).
 *  - Position / orientation / scene_origin / floor_plan are emitted
 *    as plain object literals; the dump call uses flowLevel to render
 *    them on a single line per the project's manifest convention.
 *  - home_pose_deg is ALWAYS emitted on every robot so the saved YAML
 *    records the exact pose currently shown in the editor — including
 *    the inspector's defaults if the user never touched a slider.
 *    Defaults come from the inspector at render time and are reflected
 *    into state, so by the time we save, every robot has a populated
 *    array. */
export function toYamlObject(s) {
  const obj = cloneScene(s);
  for (const r of obj.robots || []) {
    r.dressed_urdf = manifestPath(r.dressed_urdf);
    if (!Array.isArray(r.home_pose_deg) || r.home_pose_deg.length === 0) {
      // No pose recorded yet — fall back to KUKA stowed for the first 6,
      // 0 for any beyond. Keeps saved YAML self-describing.
      const len = 6;
      const stowed = [0, -90, 90, 0, 0, 0];
      r.home_pose_deg = Array.from({ length: len }, (_, i) => stowed[i] ?? 0);
    }
  }
  for (const f of obj.fixtures || []) {
    f.urdf = manifestPath(f.urdf);
  }
  if (!obj.scene_origin) delete obj.scene_origin;
  if (!obj.floor_plan) delete obj.floor_plan;
  if (obj.floor_plan) {
    obj.floor_plan.geometry_file = manifestPath(obj.floor_plan.geometry_file);
    if (!obj.floor_plan.offset) delete obj.floor_plan.offset;
    if (!obj.floor_plan.cell_origin_dxf) delete obj.floor_plan.cell_origin_dxf;
  }
  return obj;
}

/** Convert an as-authored manifest path into a path the server's
 *  asset_root-rooted REST API can consume. The manifest sits one level
 *  below asset_root (`assets/fitted-robots/scene.yaml`) and references
 *  siblings via `../foo`; collapsing the leading `..` makes the path
 *  asset-root-relative without touching state. State keeps the
 *  original verbatim so save round-trips losslessly.
 */
export function apiPath(rel) {
  if (!rel || typeof rel !== "string") return rel;
  if (rel.startsWith("/")) return rel;
  const parts = rel.split("/").filter((p) => p !== "" && p !== ".");
  const out = [];
  for (const p of parts) {
    if (p === "..") {
      if (out.length) out.pop();
    } else {
      out.push(p);
    }
  }
  return out.join("/");
}

/** Hydrate a parsed YAML object into the editor's canonical shape. */
export function fromYamlObject(obj) {
  const s = blankScene();
  s.name = obj.name || "Untitled";
  s.description = obj.description || "";
  s.scene_origin = obj.scene_origin || null;

  // The editor renders robots, fixtures, and the floor plan at their
  // RAW manifest world coordinates — no scene_origin subtraction.
  // The manifest's `scene_origin` is preserved in state and emitted
  // verbatim on save; compose still subtracts it on its own.
  // This keeps the layout aligned with the floor plan as authored
  // without forcing anything to (0,0).
  // Paths in state are stored asset-root-relative (matching the
  // palette convention). On save we add the `../` prefix back so the
  // saved YAML — which lives at `fitted-robots/<name>.yaml` — keeps
  // working with compose's `parse_scene`, which resolves relative to
  // the manifest's own directory.
  s.robots = (obj.robots || []).map((r) => ({
    robot_id: r.robot_id,
    dressed_urdf: apiPath(r.dressed_urdf),
    position: { ...defaultPosition(), ...(r.position || {}) },
    orientation: { ...defaultOrientation(), ...(r.orientation || {}) },
    home_pose_deg: Array.isArray(r.home_pose_deg) ? r.home_pose_deg.slice() : null,
  }));
  s.fixtures = (obj.fixtures || []).map((f) => ({
    id: f.id,
    urdf: apiPath(f.urdf),
    position: { ...defaultPosition(), ...(f.position || {}) },
    orientation: { ...defaultOrientation(), ...(f.orientation || {}) },
  }));
  if (obj.floor_plan) {
    s.floor_plan = {
      geometry_file: apiPath(obj.floor_plan.geometry_file),
      offset: obj.floor_plan.offset || null,
      cell_origin_dxf: obj.floor_plan.cell_origin_dxf || null,
      scale: obj.floor_plan.scale || 1.0,
    };
  }
  return s;
}

/** Convert an asset-root-relative path back to manifest-relative form
 *  (`../foo`) for serialisation. The manifest lives at
 *  `<asset_root>/fitted-robots/<name>.yaml`, so any asset-root-relative
 *  path needs `../` prepended to be reachable from the manifest dir.
 *  Absolute paths and already-prefixed paths are left alone. */
export function manifestPath(rel) {
  if (!rel || typeof rel !== "string") return rel;
  if (rel.startsWith("/") || rel.startsWith("../")) return rel;
  return "../" + rel;
}
