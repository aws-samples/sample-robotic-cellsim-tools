// FR-055: per-joint sliders. Articulates the loaded robot fully client-side —
// moving a slider (or dragging the joint in the viewport) calls
// robot.setJointValue, which urdf-loader propagates to all downstream links.

/**
 * Build one range slider per non-fixed joint of `robot` into `container`.
 * Each input drives viewer.setJointValue(name, value) live.
 *
 * Returns { setValue(name, v), highlight(name|null) } so the viewport drag
 * controls can sync the sliders and highlight the active row.
 */
export function buildJointSliders(container, robot, viewer) {
  container.innerHTML = "";
  const joints = Object.entries(robot.joints || {}).filter(
    ([, j]) => j.jointType !== "fixed" && j.jointType !== undefined,
  );

  if (joints.length === 0) {
    container.innerHTML = '<span class="muted">No articulated joints.</span>';
    return { setValue() {}, highlight() {} };
  }

  const rows = {}; // name -> { row, input, val }

  for (const [name, j] of joints) {
    const continuous = j.jointType === "continuous";
    const lower = Number.isFinite(Number(j.limit?.lower)) && !continuous ? Number(j.limit.lower) : -Math.PI;
    const upper = Number.isFinite(Number(j.limit?.upper)) && !continuous ? Number(j.limit.upper) : Math.PI;
    const start = Number(j.angle) || 0;

    const row = document.createElement("div");
    row.className = "joint-row";
    row.dataset.joint = name;

    const head = document.createElement("div");
    head.className = "joint-head";
    const label = document.createElement("span");
    label.className = "joint-name";
    label.textContent = name;
    label.title = `${name} (${j.jointType})`;
    const val = document.createElement("span");
    val.className = "joint-val";
    val.textContent = start.toFixed(3);
    head.append(label, val);

    const input = document.createElement("input");
    input.type = "range";
    input.min = String(lower);
    input.max = String(upper);
    input.step = String((upper - lower) / 200 || 0.01);
    input.value = String(start);
    input.addEventListener("input", () => {
      const v = Number(input.value);
      viewer.setJointValue(name, v);
      val.textContent = v.toFixed(3);
    });

    row.append(head, input);
    container.appendChild(row);
    rows[name] = { row, input, val };
  }

  return {
    /** Reflect an externally-driven value (e.g. viewport drag) into the slider. */
    setValue(name, v) {
      const r = rows[name];
      if (!r) return;
      r.input.value = String(v);
      r.val.textContent = Number(v).toFixed(3);
    },
    /** Mark the row for the hovered/dragged joint (null clears). */
    highlight(name) {
      for (const [n, r] of Object.entries(rows)) {
        r.row.classList.toggle("active", n === name);
      }
    },
  };
}
