// FR-054: dress-option controls — hull select + X/Y/Z ±90 orient buttons +
// the Dress trigger. Changing an orient value re-dresses (a round-trip:
// POST /api/v1/dress with the new orient, then reload the result). The
// no-round-trip live re-orient is deferred to BL-008.

/**
 * Wire the inspector controls. `state` is the shared dress state
 * ({ robotId, toolId, orient, hull }); `onDress` runs a dress + reload.
 */
export function installControls(state, onDress) {
  const hullEl = document.getElementById("hull");
  hullEl.addEventListener("change", () => {
    state.hull = hullEl.value;
  });

  // X/Y/Z ±90 buttons. Orientation is constrained to {0,90,180,270} (NFR-011).
  for (const row of document.querySelectorAll(".orient-row")) {
    const axis = Number(row.dataset.axis);
    for (const btn of row.querySelectorAll("button")) {
      const dir = Number(btn.dataset.dir);
      btn.addEventListener("click", () => {
        state.orient[axis] = (((state.orient[axis] + dir * 90) % 360) + 360) % 360;
        refreshOrientLabels(state);
        // Re-dress to reflect the new orientation if a full pair is selected.
        if (state.robotId && state.toolId) onDress();
      });
    }
  }

  document.getElementById("btn-dress").addEventListener("click", () => onDress());
}

/** Sync the ±90 button labels to state.orient. */
export function refreshOrientLabels(state) {
  for (const el of document.querySelectorAll(".orient-val")) {
    const axis = Number(el.dataset.axis);
    el.textContent = `${state.orient[axis]}°`;
  }
}
