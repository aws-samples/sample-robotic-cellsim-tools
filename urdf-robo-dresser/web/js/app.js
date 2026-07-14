// Orchestration: catalogue palette -> select robot+tool pair -> preview
// sources -> Dress -> load the dressed URDF -> orbit + joint sliders.
// Framework-free ES modules, mirroring the fitting-room's app.js style.

import * as api from "./api.js";
import { Viewer } from "./viewer.js";
import { buildJointSliders } from "./joints.js";
import { installControls, refreshOrientLabels } from "./controls.js";

const state = {
  robotId: null,
  toolId: null,
  orient: [0, 0, 0],
  hull: "medium",
  dressedRel: null,
};

const viewer = new Viewer(document.getElementById("viewport"));
const statusEl = document.getElementById("status-line");
const diagEl = document.getElementById("diagnostics");
const jointsEl = document.getElementById("joint-sliders");
const hoverEl = document.getElementById("joint-hover");

function setStatus(msg) { statusEl.textContent = msg; }

// Handle to the current joint sliders (rebuilt on every load) so viewport
// dragging can sync the panel and highlight the active row.
let jointSliders = { setValue() {}, highlight() {} };

function showJoints(robot) {
  jointSliders = buildJointSliders(jointsEl, robot, viewer);
}

// Dragging a joint in the viewport updates its slider live (FR-059).
viewer.onJointChange = (name, value) => jointSliders.setValue(name, value);
// Hovering a joint shows its name in the viewport + highlights the panel row.
viewer.onJointHover = (name) => {
  jointSliders.highlight(name);
  if (name) {
    hoverEl.textContent = name;
    hoverEl.hidden = false;
  } else {
    hoverEl.hidden = true;
  }
};

// ---------- Palette ----------
async function refreshPalette() {
  try {
    const cat = await api.listCatalogue();
    paint("#palette-robots", cat.robots, "robot");
    paint("#palette-tools", cat.tools, "tool");
    setStatus(`Catalogue: ${cat.robots.length} robots, ${cat.tools.length} tools.`);
  } catch (e) {
    setStatus("Catalogue load failed: " + e.message);
  }
  await refreshDressed();
}

/** List already-dressed robots so the user can view prior results (FR-057). */
async function refreshDressed() {
  try {
    const dressed = await api.listDressed();
    const el = document.querySelector("#palette-dressed");
    el.innerHTML = "";
    if (dressed.length === 0) {
      el.innerHTML = '<li class="muted">None yet — dress a pair.</li>';
      return;
    }
    for (const entry of dressed) {
      const li = document.createElement("li");
      li.dataset.kind = "dressed";
      li.dataset.name = entry.name;
      const name = document.createElement("span");
      name.className = "entry-name";
      name.textContent = entry.name;
      name.title = entry.urdf_rel;
      li.append(name);
      li.addEventListener("click", () => viewDressed(entry, li));
      el.appendChild(li);
    }
  } catch (e) {
    setStatus("Dressed list failed: " + e.message);
  }
}

function paint(selector, entries, kind) {
  const el = document.querySelector(selector);
  el.innerHTML = "";
  for (const entry of entries) {
    const li = document.createElement("li");
    li.dataset.kind = kind;
    li.dataset.name = entry.name;

    const dot = document.createElement("span");
    dot.className = "status-dot " + (entry.status?.kind || "valid");
    dot.title = statusTitle(entry.status);

    const name = document.createElement("span");
    name.className = "entry-name";
    name.textContent = entry.name;

    li.append(dot, name);
    li.addEventListener("click", () => selectEntry(kind, entry, li));
    el.appendChild(li);
  }
}

/** Load a previously-dressed robot straight into the viewer (FR-057). */
async function viewDressed(entry, li) {
  const list = li.parentElement;
  list.querySelectorAll("li.selected").forEach((n) => n.classList.remove("selected"));
  li.classList.add("selected");
  setStatus(`Loading dressed ${entry.name}…`);
  try {
    const robot = await viewer.loadUrdf(api.assetUrl(entry.urdf_rel));
    showJoints(robot);
    setOutputPath(entry.urdf_rel);
    setStatus(`Viewing dressed ${entry.name}. Drag to rotate.`);
  } catch (e) {
    setStatus(`Load failed (${entry.name}): ${e.message}`);
  }
}

let badgeTimer = null;

/**
 * Show where a dressed URDF lives, under urdf-dressed-robots/. When `fresh`
 * is true (a dress just ran), flag that a NEW URDF was written to disk; the
 * badge auto-hides after 10s.
 */
function setOutputPath(rel, fresh = false) {
  const el = document.getElementById("output-path");
  const badge = document.getElementById("output-badge");
  if (badgeTimer) { clearTimeout(badgeTimer); badgeTimer = null; }
  if (rel) {
    el.textContent = rel;
    el.title = rel;
    el.classList.remove("muted");
    badge.hidden = !fresh;
    if (fresh) {
      badgeTimer = setTimeout(() => { badge.hidden = true; badgeTimer = null; }, 10000);
    }
  } else {
    el.textContent = "—";
    el.classList.add("muted");
    badge.hidden = true;
  }
}

function statusTitle(status) {
  if (!status) return "";
  if (status.kind === "valid") return "valid";
  if (status.kind === "incomplete") return "incomplete: " + (status.issues || []).join(", ");
  if (status.kind === "error") return "error: " + (status.message || "");
  return status.kind;
}

// ---------- Selection + source preview ----------
async function selectEntry(kind, entry, li) {
  // Highlight the selected entry within its list.
  const list = li.parentElement;
  list.querySelectorAll("li.selected").forEach((n) => n.classList.remove("selected"));
  li.classList.add("selected");

  if (kind === "robot") {
    state.robotId = entry.name;
    document.getElementById("sel-robot").textContent = "Robot: " + entry.name;
    await preview(`urdf-robot-collection/${entry.name}/${entry.name}.urdf`, `robot ${entry.name}`);
  } else {
    state.toolId = entry.name;
    document.getElementById("sel-tool").textContent = "Tool: " + entry.name;
    await preview(`urdf-tool-collection/${entry.name}/${entry.name}.urdf`, `tool ${entry.name}`);
  }
  updateDressEnabled();
}

async function preview(rel, label) {
  setStatus(`Loading ${label}…`);
  try {
    const robot = await viewer.loadUrdf(api.assetUrl(rel));
    // Show joint names for the source too, not just after dressing.
    showJoints(robot);
    setStatus(`Previewing ${label}.`);
  } catch (e) {
    setStatus(`Preview failed (${label}): ${e.message}`);
  }
}

function updateDressEnabled() {
  document.getElementById("btn-dress").disabled = !(state.robotId && state.toolId);
}

// ---------- Dress + load result ----------
async function runDress() {
  if (!state.robotId || !state.toolId) {
    setStatus("Select a robot and a tool first.");
    return;
  }
  setStatus(`Dressing ${state.robotId} + ${state.toolId}…`);
  diagEl.innerHTML = "";
  try {
    const r = await api.dress(state);
    renderDiagnostics(r.diagnostics || []);
    state.dressedRel = r.dressed_urdf_rel;
    setOutputPath(r.dressed_urdf_rel, true); // just written to disk
    const robot = await viewer.loadUrdf(api.assetUrl(r.dressed_urdf_rel));
    showJoints(robot);
    await refreshDressed(); // the new combo now appears in the Dressed list
    setStatus(`Dressed ${state.robotId}__${state.toolId}. Drag to rotate.`);
  } catch (e) {
    // 422 = blocking validation; surface the message, keep last good model.
    renderDiagnostics([{ severity: "error", code: "dress_failed", message: e.message }]);
    setStatus("Dress failed: " + e.message);
  }
}

function renderDiagnostics(diags) {
  diagEl.innerHTML = "";
  if (diags.length === 0) {
    diagEl.innerHTML = '<li class="muted">No diagnostics.</li>';
    return;
  }
  for (const d of diags) {
    const li = document.createElement("li");
    li.className = d.severity === "error" ? "error" : "warning";
    const code = document.createElement("span");
    code.className = "code";
    code.textContent = `[${d.code}] `;
    li.append(code, document.createTextNode(d.message));
    diagEl.appendChild(li);
  }
}

// ---------- Boot ----------
installControls(state, runDress);
refreshOrientLabels(state);
updateDressEnabled();
setOutputPath(null);

// FR-058: collision-hull overlay toggle.
document.getElementById("toggle-collision").addEventListener("change", (e) => {
  viewer.setCollisionVisible(e.target.checked);
});

refreshPalette();
setStatus("Ready. Pick a robot and a tool, then Dress.");
