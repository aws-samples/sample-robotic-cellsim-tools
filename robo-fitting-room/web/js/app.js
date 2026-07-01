// Top-level glue: wires the palette, canvas, and inspector to a single
// in-memory SceneDefinition. No framework — plain ES modules.

import * as api from "./api.js";
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
  // Floor-plan offset, AS AUTHORED in the manifest — no
  // scene_origin subtraction. Robots/fixtures render at their raw
  // world coords too, so everything shares the same frame and the
  // floor plan stays where it was placed in CAD.
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
  // Refresh inspector position/orientation values when the user drags
  // the selected item.
  if (view.selected) {
    if (view.selected.type === "robot") inspector.showRobot(state, view.selected.index);
    else inspector.showFixture(state, view.selected.index);
  }
};
view.onMouseWorld = (x, y) => {
  // Display alongside (not replacing) any active status message.
  // Use a separate readout element so dragging-while-statusing doesn't clobber.
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
    setStatus("Palette load failed: " + e.message);
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
document.getElementById("btn-load-yaml").addEventListener("click", () => {
  document.getElementById("file-yaml").click();
});
document.getElementById("file-yaml").addEventListener("change", async (e) => {
  const f = e.target.files[0];
  if (!f) return;
  const text = await f.text();
  loadYamlText(text, f.name);
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
          // Fit AFTER the segments arrive — that's the one moment we
          // know the full AABB (robots + floor + fixtures).
          view.fitToScene();
        })
        .catch((err) => setStatus("Floor plan: " + err.message));
    } else {
      view.setFloor({ segments: [], footprint_segments: [] }, null, [0, 0, 0], 1.0);
    }
    inspector.showEmpty();
    view.selected = null;
    view.render();
    // Immediate fit for the no-floor case (and a reasonable first
    // approximation while floor segments are still in flight). The
    // post-segments call above tightens it once they arrive.
    view.fitToScene();
    setStatus(`Loaded ${filename || "scene"}: ${state.robots.length} robots, ${state.fixtures.length} fixtures.`);
  } catch (e) {
    setStatus("Load failed: " + e.message);
  }
}

document.getElementById("btn-save").addEventListener("click", async () => {
  // Always save under fitted-robots/. The prompt only asks for a filename
  // so users can't accidentally place the manifest somewhere that breaks
  // its `../` references.
  const cur = currentManifestPath.replace(/^fitted-robots\//, "");
  const filename = prompt("Save manifest as (under fitted-robots/):", cur);
  if (!filename) return;
  const safeName = filename.replace(/^\/+/, "").replace(/\.\./g, "");
  const path = `fitted-robots/${safeName.endsWith(".yaml") || safeName.endsWith(".yml") ? safeName : safeName + ".yaml"}`;
  currentManifestPath = path;
  const yamlText = jsyaml.dump(toYamlObject(state), {
    lineWidth: 120,
    noCompatMode: true,   // don't quote 'y'/'n'/'on'/'off' as YAML 1.1 booleans
    quotingType: '"',     // double quotes when any quoting IS needed
    forceQuotes: false,
  });
  try {
    const r = await api.saveScene(path, yamlText);
    setStatus(`Saved ${r.bytes} bytes to ${path}.`);
  } catch (e) {
    setStatus("Save failed: " + e.message);
  }
});

// ----- Generate USDA: modal-driven options + run -----

const composeModal = document.getElementById("compose-modal");
const composeFilename = document.getElementById("opt-filename");
const composeFullRobots = document.getElementById("opt-full-robots");
const composeFullTools = document.getElementById("opt-full-tools");
const composeUrdfBundle = document.getElementById("opt-urdf-bundle");

function openComposeModal() {
  // Pre-seed defaults each time so a previous edit doesn't carry over
  // unexpectedly.
  composeFullRobots.checked = false;
  composeFullTools.checked = true;
  composeUrdfBundle.checked = false;
  composeFilename.value = `scene-${timestamp()}.zip`;
  composeModal.hidden = false;
  // Focus the primary action so Enter triggers Generate.
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

/** Trigger a browser download of a Blob under the given filename. */
function downloadBlob(blob, filename) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.click();
  URL.revokeObjectURL(url);
}

document.getElementById("btn-compose-go").addEventListener("click", async () => {
  const fullRobots = composeFullRobots.checked;
  const fullTools = composeFullTools.checked;
  const urdfBundle = composeUrdfBundle.checked;
  let downloadName = composeFilename.value.trim();
  if (!downloadName) downloadName = `scene-${timestamp()}.zip`;
  if (!/\.zip$/i.test(downloadName)) downloadName += ".zip";
  // Bundle name mirrors the CLI: <stem>-urdf.zip next to the USDA zip name.
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
    setStatus("Composing…");
    const blob = await api.composeZip(path, { fullRobots, fullTools });
    downloadBlob(blob, downloadName);
    if (urdfBundle) {
      setStatus("Bundling source URDFs…");
      const bundle = await api.composeUrdfBundle(path);
      downloadBlob(bundle, bundleName);
      setStatus(`Compose complete; downloaded ${downloadName} + ${bundleName}.`);
    } else {
      setStatus(`Compose complete; downloaded ${downloadName}.`);
    }
  } catch (e) {
    setStatus("Compose failed: " + e.message);
  }
});

// Sidebar collapse toggles
document.querySelectorAll(".aside-toggle").forEach((btn) => {
  btn.addEventListener("click", () => {
    const id = btn.dataset.aside; // "palette" | "inspector"
    const aside = document.getElementById(id);
    const collapsed = aside.classList.toggle("collapsed");
    // Flip the chevron: palette collapses leftward (◂ ↔ ▸); inspector
    // collapses rightward (▸ ↔ ◂).
    if (id === "palette") {
      btn.textContent = collapsed ? "▸" : "◂";
    } else {
      btn.textContent = collapsed ? "◂" : "▸";
    }
    btn.title = collapsed ? `Expand ${id}` : `Collapse ${id}`;
    // Canvas size changed — wait for the grid transition to settle, then
    // dispatch a window resize so CanvasView's existing _fitDpr handler
    // recomputes the backing-store dimensions.
    setTimeout(() => window.dispatchEvent(new Event("resize")), 200);
  });
});

refreshPalette();
setStatus("Ready. Load a scene YAML, or click an asset in the palette to add it.");
