// Top-level glue: wires the palette, canvas, and inspector to a single
// in-memory SceneDefinition. No framework — plain ES modules.
//
// This is the VS Code webview variant of the in-binary SPA's app.js. The
// canvas / inspector / scene_state modules are byte-identical to the
// server's; only the host-integration seams differ:
//   • Load Scene  → native open dialog via the extension host (bridge).
//   • Save        → input box via the host, then POST to the server.
//   • Generate    → compose delegated to the host (writes zip to disk).

import * as api from "./api.js";
import * as bridge from "./bridge.js";
import {
  blankScene, fromYamlObject, toYamlObject,
  defaultPosition, defaultOrientation, apiPath,
} from "./scene_state.js";
import { CanvasView } from "./canvas.js";
import { Inspector } from "./inspector.js";

const state = blankScene();
let currentManifestPath = "fitted-robots/scene.yaml";

const canvasEl = document.getElementById("canvas");
const inspectorEl = document.getElementById("inspector");
const statusEl = document.getElementById("status-line");

/** UTC timestamp in `yyyymmddhhmmss` form, used for download filenames. */
function timestamp() {
  const d = new Date();
  const p = (n, w = 2) => String(n).padStart(w, "0");
  return (
    d.getFullYear() +
    p(d.getMonth() + 1) +
    p(d.getDate()) +
    p(d.getHours()) +
    p(d.getMinutes()) +
    p(d.getSeconds())
  );
}

function computeEffectiveOffset(state) {
  const fp = state.floor_plan;
  if (!fp) return { offset: [0, 0, 0], scale: 1.0 };
  const o = fp.offset
    ? fp.offset
    : fp.cell_origin_dxf
    ? { x: -fp.cell_origin_dxf.x, y: -fp.cell_origin_dxf.y, z: -fp.cell_origin_dxf.z }
    : { x: 0, y: 0, z: 0 };
  return {
    offset: [o.x, o.y, o.z],
    scale: fp.scale && fp.scale > 0 ? fp.scale : 1.0,
  };
}

const inspector = new Inspector(inspectorEl, () => {
  view.render();
});

const view = new CanvasView(canvasEl, () => state, (sel) => {
  if (sel == null) inspector.showEmpty();
  else if (sel.type === "robot") inspector.showRobot(state, sel.index);
  else inspector.showFixture(state, sel.index);
});
view.onItemMoved = () => {
  if (view.selected) {
    if (view.selected.type === "robot") inspector.showRobot(state, view.selected.index);
    else inspector.showFixture(state, view.selected.index);
  }
};
view.onMouseWorld = (x, y) => {
  const el = document.getElementById("coord-readout");
  if (el) el.textContent = `x=${x.toFixed(3)} m   y=${y.toFixed(3)} m`;
};

function setStatus(msg) { statusEl.textContent = msg; }

// ---------- Palette ----------
async function refreshPalette() {
  try {
    const [robots, fixtures, floors] = await Promise.all([
      api.listRobots(), api.listFixtures(), api.listFloorPlans(),
    ]);
    paint("#palette-robots", robots, (r) => addRobotFromPalette(r));
    paint("#palette-fixtures", fixtures, (f) => addFixtureFromPalette(f));
    paint("#palette-floor-plans", floors.map((f) => ({ id: f.name, urdf_rel: f.rel })),
          (f) => attachFloorPlan(f.urdf_rel));
  } catch (e) {
    setStatus("Palette load failed: " + e.message + " — is the server running?");
  }
}

function paint(selector, entries, onClick) {
  const el = document.querySelector(selector);
  el.innerHTML = "";
  for (const e of entries) {
    const li = document.createElement("li");
    li.textContent = e.id;
    li.title = e.urdf_rel || "";
    li.addEventListener("click", () => onClick(e));
    el.appendChild(li);
  }
}

function addRobotFromPalette(asset) {
  const robotId = uniqueId(asset.id, state.robots.map((r) => r.robot_id));
  state.robots.push({
    robot_id: robotId,
    dressed_urdf: asset.urdf_rel,
    position: defaultPosition(),
    orientation: defaultOrientation(),
    home_pose_deg: null,
  });
  view.selected = { type: "robot", index: state.robots.length - 1 };
  inspector.showRobot(state, view.selected.index);
  view.render();
  setStatus(`Added robot ${robotId}.`);
}

function addFixtureFromPalette(asset) {
  const id = uniqueId(asset.id, state.fixtures.map((f) => f.id));
  state.fixtures.push({
    id,
    urdf: asset.urdf_rel,
    position: defaultPosition(),
    orientation: defaultOrientation(),
  });
  view.selected = { type: "fixture", index: state.fixtures.length - 1 };
  inspector.showFixture(state, view.selected.index);
  view.render();
  setStatus(`Added fixture ${id}.`);
}

async function attachFloorPlan(rel) {
  state.floor_plan = {
    geometry_file: rel,
    offset: null,
    cell_origin_dxf: null,
    scale: 1.0,
  };
  try {
    const segs = await api.floorPlanSegments(apiPath(rel));
    const eff = computeEffectiveOffset(state);
    view.setFloor(segs, state.floor_plan?.cell_origin_dxf || null, eff.offset, eff.scale);
    setStatus(`Floor plan: ${rel}`);
  } catch (e) {
    setStatus("Floor plan load failed: " + e.message);
  }
}

function uniqueId(base, existing) {
  if (!existing.includes(base)) return base;
  for (let i = 2; i < 100; i++) {
    const cand = `${base}_${i}`;
    if (!existing.includes(cand)) return cand;
  }
  return base + "_dup";
}

// ---------- File operations ----------
document.getElementById("btn-load-yaml").addEventListener("click", async () => {
  try {
    const res = await bridge.openYaml();
    if (res.cancelled) return;
    loadYamlText(res.text, res.filename);
  } catch (e) {
    setStatus("Load failed: " + e.message);
  }
});

function loadYamlText(text, filename) {
  try {
    const obj = jsyaml.load(text);
    Object.assign(state, fromYamlObject(obj));
    if (filename) currentManifestPath = "fitted-robots/" + filename;
    if (state.floor_plan?.geometry_file) {
      api.floorPlanSegments(apiPath(state.floor_plan.geometry_file))
        .then((s) => {
          const eff = computeEffectiveOffset(state);
          view.setFloor(s, state.floor_plan?.cell_origin_dxf || null, eff.offset, eff.scale);
          view.fitToScene();
        })
        .catch((err) => setStatus("Floor plan: " + err.message));
    } else {
      view.setFloor({ segments: [], footprint_segments: [] }, null, [0, 0, 0], 1.0);
    }
    inspector.showEmpty();
    view.selected = null;
    view.render();
    view.fitToScene();
    setStatus(`Loaded ${filename || "scene"}: ${state.robots.length} robots, ${state.fixtures.length} fixtures.`);
  } catch (e) {
    setStatus("Load failed: " + e.message);
  }
}

document.getElementById("btn-save").addEventListener("click", async () => {
  const cur = currentManifestPath.replace(/^fitted-robots\//, "");
  const filename = await bridge.promptSaveName(cur);
  if (!filename) return;
  const safeName = filename.replace(/^\/+/, "").replace(/\.\./g, "");
  const path = `fitted-robots/${safeName.endsWith(".yaml") || safeName.endsWith(".yml") ? safeName : safeName + ".yaml"}`;
  currentManifestPath = path;
  const yamlText = jsyaml.dump(toYamlObject(state), {
    lineWidth: 120,
    noCompatMode: true,
    quotingType: '"',
    forceQuotes: false,
  });
  try {
    const r = await api.saveScene(path, yamlText);
    setStatus(`Saved ${r.bytes} bytes to ${path}.`);
  } catch (e) {
    setStatus("Save failed: " + e.message);
  }
});

// ----- Export URDF source bundle (extracted folder tree) -----
document.getElementById("btn-export-bundle").addEventListener("click", async () => {
  setStatus("Validating…");
  const yamlText = jsyaml.dump(toYamlObject(state), {
    lineWidth: 120,
    noCompatMode: true,
    quotingType: '"',
    forceQuotes: false,
  });
  try {
    const v = await api.validateYaml(yamlText);
    if (!v.ok) {
      setStatus("Validation errors: " + v.errors.join(" | "));
      return;
    }
    // The bundle endpoint reads the manifest from disk, so persist the
    // current edit first (server-side, under fitted-robots/).
    const path = currentManifestPath;
    await api.saveScene(path, yamlText);
    setStatus("Exporting URDF bundle…");
    const res = await bridge.exportBundle({ manifestPath: path });
    setStatus(res.message || (res.ok ? "Bundle exported." : "Export failed."));
  } catch (e) {
    setStatus("Export failed: " + e.message);
  }
});

// ----- Generate USDA: modal-driven options + run -----

const composeModal = document.getElementById("compose-modal");
const composeFilename = document.getElementById("opt-filename");
const composeFullRobots = document.getElementById("opt-full-robots");
const composeFullTools = document.getElementById("opt-full-tools");
const composeUrdfBundle = document.getElementById("opt-urdf-bundle");

function openComposeModal() {
  composeFullRobots.checked = false;
  composeFullTools.checked = true;
  composeUrdfBundle.checked = false;
  composeFilename.value = `scene-${timestamp()}.zip`;
  composeModal.hidden = false;
  document.getElementById("btn-compose-go").focus();
}
function closeComposeModal() {
  composeModal.hidden = true;
}

document.getElementById("btn-compose").addEventListener("click", openComposeModal);
document.getElementById("btn-compose-cancel").addEventListener("click", closeComposeModal);
composeModal.querySelector(".modal-backdrop").addEventListener("click", closeComposeModal);
document.addEventListener("keydown", (e) => {
  if (!composeModal.hidden && e.key === "Escape") closeComposeModal();
});

document.getElementById("btn-compose-go").addEventListener("click", async () => {
  const fullRobots = composeFullRobots.checked;
  const fullTools = composeFullTools.checked;
  const urdfBundle = composeUrdfBundle.checked;
  let downloadName = composeFilename.value.trim();
  if (!downloadName) downloadName = `scene-${timestamp()}.zip`;
  if (!/\.zip$/i.test(downloadName)) downloadName += ".zip";
  const bundleName = downloadName.replace(/\.zip$/i, "") + "-urdf.zip";
  closeComposeModal();

  setStatus("Validating…");
  const yamlText = jsyaml.dump(toYamlObject(state), {
    lineWidth: 120,
    noCompatMode: true,
    quotingType: '"',
    forceQuotes: false,
  });
  try {
    const v = await api.validateYaml(yamlText);
    if (!v.ok) {
      setStatus("Validation errors: " + v.errors.join(" | "));
      return;
    }
    const path = currentManifestPath;
    await api.saveScene(path, yamlText);
    setStatus("Composing… (this can take a while for full-fidelity scenes)");
    // The host performs the compose fetch(es) and writes to disk, so
    // large zips never round-trip through the webview.
    const res = await bridge.compose({
      manifestPath: path,
      fullRobots,
      fullTools,
      urdfBundle,
      downloadName,
      bundleName,
    });
    setStatus(res.message || (res.ok ? "Compose complete." : "Compose failed."));
  } catch (e) {
    setStatus("Compose failed: " + e.message);
  }
});

// Sidebar collapse toggles
document.querySelectorAll(".aside-toggle").forEach((btn) => {
  btn.addEventListener("click", () => {
    const id = btn.dataset.aside;
    const aside = document.getElementById(id);
    const collapsed = aside.classList.toggle("collapsed");
    if (id === "palette") {
      btn.textContent = collapsed ? "▸" : "◂";
    } else {
      btn.textContent = collapsed ? "◂" : "▸";
    }
    btn.title = collapsed ? `Expand ${id}` : `Collapse ${id}`;
    setTimeout(() => window.dispatchEvent(new Event("resize")), 200);
  });
});

// The extension host asks us to reload the palette once the backend is
// running (e.g. after the asset root is set on a panel that opened before the
// server was ready). This is separate from bridge.js's request/response
// listener, which ignores messages without a matching requestId.
window.addEventListener("message", (event) => {
  if (event.data && event.data.type === "refreshPalette") {
    refreshPalette();
  }
});

refreshPalette();
setStatus("Ready. Load a scene YAML, or click an asset in the palette to add it.");
