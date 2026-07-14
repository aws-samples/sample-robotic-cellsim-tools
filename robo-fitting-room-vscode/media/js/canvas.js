// Canvas2D top-down render. Coordinates are world-space in metres; the
// canvas maps `view.center_world` to the canvas centre with `view.scale`
// pixels per metre. Wheel zooms; right-click drag pans.

const ROBOT_RADIUS_M = 0.4;        // base footprint radius in metres
const FIXTURE_HALF_M = 0.5;        // default fixture half-extent

/**
 * Read a `--vscode-*` theme custom property injected by the IDE (VS Code /
 * Kiro set these on the webview's root element automatically), falling back
 * to `fallback` when the property is unset (e.g. running outside a themed
 * webview). Canvas 2D drawing calls (fillStyle/strokeStyle) need concrete
 * color strings, not CSS var() references, so palette colors are resolved
 * once here rather than inlined as var(...) throughout the drawing code.
 */
function themeColor(varName, fallback) {
  const v = getComputedStyle(document.documentElement).getPropertyValue(varName).trim();
  return v || fallback;
}

/** Canvas drawing palette, resolved from the IDE's current theme. Recomputed
 *  on construction and whenever VS Code signals a theme change, so switching
 *  between e.g. Kiro's light/dark themes updates the render without reload. */
function buildPalette() {
  return {
    // --vscode-panel-border is meant for dividers between adjacent panels
    // (e.g. the palette/inspector borders) and in many themes is near-
    // invisible against a flat background of the same panel — which is
    // exactly why the grid disappeared. --vscode-editorWhitespace-foreground
    // is themes' dedicated token for faint guide/whitespace overlays drawn
    // directly on the editor background, so it stays visible while still
    // reading as a subtle grid.
    gridLine: themeColor("--vscode-editorWhitespace-foreground", "#2a2e32"),
    floorLine: themeColor("--vscode-descriptionForeground", "#5a6268"),
    footprintLine: themeColor("--vscode-focusBorder", "#ff8a3d"),
    originRing: themeColor("--vscode-focusBorder", "#ff8a3d"),
    axisX: themeColor("--vscode-charts-red", "#e04444"),
    axisY: themeColor("--vscode-charts-green", "#3fb443"),
    axisOrigin: themeColor("--vscode-editor-foreground", "#ffffff"),
    robotFill: themeColor("--vscode-charts-blue", "#5e8db8"),
    robotStroke: themeColor("--vscode-charts-blue", "#90b4d6"),
    robotFillSelected: themeColor("--vscode-list-activeSelectionBackground", "#ffa566"),
    robotStrokeSelected: themeColor("--vscode-focusBorder", "#ffd9b3"),
    fixtureFill: themeColor("--vscode-charts-green", "#6f7d56"),
    fixtureStroke: themeColor("--vscode-charts-green", "#8c9c70"),
    fixtureFillSelected: themeColor("--vscode-list-activeSelectionBackground", "#a5b88a"),
    fixtureStrokeSelected: themeColor("--vscode-focusBorder", "#cfdbb1"),
    label: themeColor("--vscode-editor-foreground", "#ffffff"),
  };
}

export class CanvasView {
  constructor(canvas, getState, onSelect) {
    this.canvas = canvas;
    this.ctx = canvas.getContext("2d");
    this.getState = getState;
    this.onSelect = onSelect; // ({type, index}) — selection callback

    this.view = {
      center: { x: 0, y: 0 }, // world-space metres
      scale: 60,                // pixels per metre
    };
    this.floorSegments = { segments: [], footprint_segments: [] };
    this.cellOriginDxf = null;
    this.floorOffset = [0, 0, 0];   // world-frame offset to add to OBJ-local segment coords
    this.floorScale = 1.0;
    this.onMouseWorld = null;        // optional callback (worldX, worldY) for status readout
    this.selected = null;     // {type:"robot"|"fixture", index:number}
    this.hovered = null;
    this.dragging = null;     // {type, index, start: {x, y}}
    this.panning = null;      // {start_screen, start_center}
    this.palette = buildPalette();
    // VS Code (and Kiro) toggle a `vscode-high-contrast`/theme-kind class and
    // re-inject the --vscode-* custom properties on `<body>` when the user
    // switches themes; re-resolve our cached colors so the canvas follows
    // along without needing a reload.
    new MutationObserver(() => {
      this.palette = buildPalette();
      this.render();
    }).observe(document.body, { attributes: true, attributeFilter: ["class"] });

    this._installEvents();
    this._fitDpr();
    // A window 'resize' listener alone misses most of the ways this canvas's
    // container can change size in a VS Code webview: the initial CSS grid
    // layout settling a tick after mount (leaving the constructor's first
    // _fitDpr() call with a stale, too-small rect), the palette/inspector
    // aside collapse/expand transition (a grid-template-columns change, not a
    // window resize), and editor-group/panel resizes. ResizeObserver fires
    // reliably for all of these — including once immediately with the correct
    // post-layout size — so the canvas always tracks its actual available
    // space instead of getting stuck at whatever size it happened to measure
    // first.
    this._resizeObserver = new ResizeObserver(() => this._fitDpr());
    this._resizeObserver.observe(this.canvas.parentElement || this.canvas);
  }

  setFloor(segments, cellOriginDxf, floorOffset, floorScale) {
    this.floorSegments = segments || { segments: [], footprint_segments: [] };
    this.cellOriginDxf = cellOriginDxf || null;
    this.floorOffset = floorOffset || [0, 0, 0];
    this.floorScale = (floorScale && floorScale > 0) ? floorScale : 1.0;
    this.render();
  }

  setSelected(sel) { this.selected = sel; this.render(); }

  /** Compute a world-space AABB over the floor plan + robots + fixtures
   *  and adjust `view.center` and `view.scale` so it all fits in the
   *  canvas with ~10% margin. No-op if there's nothing to fit. */
  fitToScene() {
    const state = this.getState();
    const fs = this.floorScale, [fox, foy] = this.floorOffset;
    let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity;
    const consider = (x, y) => {
      if (!Number.isFinite(x) || !Number.isFinite(y)) return;
      if (x < minX) minX = x;
      if (y < minY) minY = y;
      if (x > maxX) maxX = x;
      if (y > maxY) maxY = y;
    };
    for (const seg of this.floorSegments.segments || []) {
      consider(seg[0][0] * fs + fox, seg[0][1] * fs + foy);
      consider(seg[1][0] * fs + fox, seg[1][1] * fs + foy);
    }
    for (const seg of this.floorSegments.footprint_segments || []) {
      consider(seg[0][0] * fs + fox, seg[0][1] * fs + foy);
      consider(seg[1][0] * fs + fox, seg[1][1] * fs + foy);
    }
    for (const r of state.robots || []) consider(r.position?.x, r.position?.y);
    for (const f of state.fixtures || []) consider(f.position?.x, f.position?.y);
    if (!Number.isFinite(minX)) return; // nothing to fit
    // Pad the AABB; if degenerate (single point), give it a 1m halo.
    const dx = Math.max(maxX - minX, 1);
    const dy = Math.max(maxY - minY, 1);
    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;
    const w = this.canvas.clientWidth || 1;
    const h = this.canvas.clientHeight || 1;
    // Leave a 10% margin on each side
    const sx = (w * 0.9) / dx;
    const sy = (h * 0.9) / dy;
    this.view.center.x = cx;
    this.view.center.y = cy;
    this.view.scale = Math.max(2, Math.min(800, Math.min(sx, sy)));
    this.render();
  }

  /** Convert a screen coord (px, relative to canvas) to world metres. */
  screenToWorld(sx, sy) {
    const w = this.canvas.clientWidth, h = this.canvas.clientHeight;
    return {
      x: this.view.center.x + (sx - w / 2) / this.view.scale,
      y: this.view.center.y - (sy - h / 2) / this.view.scale,  // y up
    };
  }
  worldToScreen(wx, wy) {
    const w = this.canvas.clientWidth, h = this.canvas.clientHeight;
    return {
      x: w / 2 + (wx - this.view.center.x) * this.view.scale,
      y: h / 2 - (wy - this.view.center.y) * this.view.scale,
    };
  }

  render() {
    const { ctx } = this;
    const w = this.canvas.clientWidth, h = this.canvas.clientHeight;
    const dpr = window.devicePixelRatio || 1;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    // Background grid
    this._drawGrid(w, h);

    // Floor plan
    ctx.strokeStyle = this.palette.floorLine;
    ctx.lineWidth = 1;
    for (const seg of this.floorSegments.segments || []) {
      const ax = seg[0][0] * this.floorScale + this.floorOffset[0];
      const ay = seg[0][1] * this.floorScale + this.floorOffset[1];
      const bx = seg[1][0] * this.floorScale + this.floorOffset[0];
      const by = seg[1][1] * this.floorScale + this.floorOffset[1];
      const a = this.worldToScreen(ax, ay);
      const b = this.worldToScreen(bx, by);
      ctx.beginPath();
      ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y);
      ctx.stroke();
    }
    ctx.strokeStyle = this.palette.footprintLine;
    ctx.lineWidth = 1.2;
    for (const seg of this.floorSegments.footprint_segments || []) {
      const ax = seg[0][0] * this.floorScale + this.floorOffset[0];
      const ay = seg[0][1] * this.floorScale + this.floorOffset[1];
      const bx = seg[1][0] * this.floorScale + this.floorOffset[0];
      const by = seg[1][1] * this.floorScale + this.floorOffset[1];
      const a = this.worldToScreen(ax, ay);
      const b = this.worldToScreen(bx, by);
      ctx.beginPath();
      ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y);
      ctx.stroke();
    }

    // World origin axes
    this._drawAxes();

    // Cell origin marker
    if (this.cellOriginDxf) {
      const p = this.worldToScreen(0, 0);
      ctx.strokeStyle = this.palette.originRing;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.arc(p.x, p.y, 8, 0, Math.PI * 2);
      ctx.stroke();
    }

    // Robots and fixtures
    const state = this.getState();
    (state.fixtures || []).forEach((f, i) => this._drawFixture(f, this._isSelected("fixture", i)));
    (state.robots || []).forEach((r, i) => this._drawRobot(r, this._isSelected("robot", i)));
  }

  _isSelected(type, index) {
    return this.selected && this.selected.type === type && this.selected.index === index;
  }

  _drawAxes() {
    const ctx = this.ctx;
    const o = this.worldToScreen(0, 0);
    const xEnd = this.worldToScreen(1, 0);
    const yEnd = this.worldToScreen(0, 1);
    ctx.lineWidth = 2;
    // X
    ctx.strokeStyle = this.palette.axisX;
    ctx.beginPath(); ctx.moveTo(o.x, o.y); ctx.lineTo(xEnd.x, xEnd.y); ctx.stroke();
    this._drawArrowhead(xEnd, o, this.palette.axisX);
    ctx.fillStyle = this.palette.axisX;
    ctx.font = "11px JetBrains Mono, monospace";
    ctx.textAlign = "left";
    ctx.fillText("X", xEnd.x + 4, xEnd.y + 4);
    // Y
    ctx.strokeStyle = this.palette.axisY;
    ctx.beginPath(); ctx.moveTo(o.x, o.y); ctx.lineTo(yEnd.x, yEnd.y); ctx.stroke();
    this._drawArrowhead(yEnd, o, this.palette.axisY);
    ctx.fillStyle = this.palette.axisY;
    ctx.fillText("Y", yEnd.x + 4, yEnd.y - 2);
    // origin ring
    ctx.strokeStyle = this.palette.axisOrigin;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(o.x, o.y, 4, 0, Math.PI * 2);
    ctx.stroke();
  }

  _drawArrowhead(tip, from, color) {
    const ctx = this.ctx;
    const dx = tip.x - from.x;
    const dy = tip.y - from.y;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len, uy = dy / len;
    const size = 7;
    const ax = tip.x - ux * size + (-uy) * (size * 0.4);
    const ay = tip.y - uy * size + ux * (size * 0.4);
    const bx = tip.x - ux * size - (-uy) * (size * 0.4);
    const by = tip.y - uy * size - ux * (size * 0.4);
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.moveTo(tip.x, tip.y);
    ctx.lineTo(ax, ay);
    ctx.lineTo(bx, by);
    ctx.closePath();
    ctx.fill();
  }

  _drawGrid(w, h) {
    const { ctx, view } = this;
    const stepM = pickGridStep(view.scale);
    const stepPx = stepM * view.scale;
    const tl = this.screenToWorld(0, 0);
    const br = this.screenToWorld(w, h);
    const x0 = Math.floor(tl.x / stepM) * stepM;
    const x1 = Math.ceil(br.x / stepM) * stepM;
    const y0 = Math.floor(br.y / stepM) * stepM;
    const y1 = Math.ceil(tl.y / stepM) * stepM;
    ctx.strokeStyle = this.palette.gridLine;
    ctx.lineWidth = 1;
    for (let x = x0; x <= x1; x += stepM) {
      const sx = this.worldToScreen(x, 0).x;
      ctx.beginPath(); ctx.moveTo(sx, 0); ctx.lineTo(sx, h); ctx.stroke();
    }
    for (let y = y0; y <= y1; y += stepM) {
      const sy = this.worldToScreen(0, y).y;
      ctx.beginPath(); ctx.moveTo(0, sy); ctx.lineTo(w, sy); ctx.stroke();
    }
    void stepPx;
  }

  _drawRobot(r, selected) {
    const ctx = this.ctx;
    const c = this.worldToScreen(r.position.x, r.position.y);
    const r_px = ROBOT_RADIUS_M * this.view.scale;
    ctx.fillStyle = selected ? this.palette.robotFillSelected : this.palette.robotFill;
    ctx.strokeStyle = selected ? this.palette.robotStrokeSelected : this.palette.robotStroke;
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(c.x, c.y, r_px, 0, Math.PI * 2);
    ctx.fill(); ctx.stroke();
    // Heading indicator (orientation.c is yaw around Z = ABC's c)
    // Manifest convention: A=Z yaw degrees. We draw the heading using `a`.
    const yawDeg = r.orientation.a || 0;
    const yaw = (yawDeg * Math.PI) / 180;
    const tipX = c.x + Math.cos(yaw) * r_px * 1.3;
    const tipY = c.y - Math.sin(yaw) * r_px * 1.3;
    ctx.beginPath();
    ctx.moveTo(c.x, c.y); ctx.lineTo(tipX, tipY);
    ctx.stroke();

    ctx.fillStyle = this.palette.label;
    ctx.font = "11px JetBrains Mono, monospace";
    ctx.textAlign = "center";
    ctx.fillText(r.robot_id, c.x, c.y - r_px - 4);
  }

  _drawFixture(f, selected) {
    const ctx = this.ctx;
    const c = this.worldToScreen(f.position.x, f.position.y);
    const half_px = FIXTURE_HALF_M * this.view.scale;
    ctx.save();
    ctx.translate(c.x, c.y);
    const yawDeg = f.orientation.a || 0;
    ctx.rotate((-yawDeg * Math.PI) / 180);
    ctx.fillStyle = selected ? this.palette.fixtureFillSelected : this.palette.fixtureFill;
    ctx.strokeStyle = selected ? this.palette.fixtureStrokeSelected : this.palette.fixtureStroke;
    ctx.lineWidth = 1.2;
    ctx.fillRect(-half_px, -half_px, 2 * half_px, 2 * half_px);
    ctx.strokeRect(-half_px, -half_px, 2 * half_px, 2 * half_px);
    ctx.restore();
    ctx.fillStyle = this.palette.label;
    ctx.font = "11px JetBrains Mono, monospace";
    ctx.textAlign = "center";
    ctx.fillText(f.id, c.x, c.y - half_px - 4);
  }

  _installEvents() {
    this.canvas.addEventListener("wheel", (e) => {
      e.preventDefault();
      const w = this.screenToWorld(e.offsetX, e.offsetY);
      const factor = Math.exp(-e.deltaY * 0.001);
      this.view.scale = Math.max(2, Math.min(800, this.view.scale * factor));
      const w2 = this.screenToWorld(e.offsetX, e.offsetY);
      this.view.center.x += w.x - w2.x;
      this.view.center.y += w.y - w2.y;
      this.render();
    }, { passive: false });

    this.canvas.addEventListener("mousedown", (e) => {
      if (e.button === 2) {
        // right click -> pan
        this.panning = {
          start_screen: { x: e.offsetX, y: e.offsetY },
          start_center: { ...this.view.center },
        };
        return;
      }
      if (e.button !== 0) return;
      const hit = this._hitTest(e.offsetX, e.offsetY);
      if (hit) {
        this.selected = hit;
        this.dragging = { ...hit, start: { x: e.offsetX, y: e.offsetY } };
        this.onSelect && this.onSelect(hit);
      } else {
        this.selected = null;
        this.onSelect && this.onSelect(null);
      }
      this.render();
    });
    this.canvas.addEventListener("contextmenu", (e) => e.preventDefault());

    this.canvas.addEventListener("mousemove", (e) => {
      if (this.onMouseWorld) {
        const w = this.screenToWorld(e.offsetX, e.offsetY);
        this.onMouseWorld(w.x, w.y);
      }
      if (this.panning) {
        const dx = e.offsetX - this.panning.start_screen.x;
        const dy = e.offsetY - this.panning.start_screen.y;
        this.view.center.x = this.panning.start_center.x - dx / this.view.scale;
        this.view.center.y = this.panning.start_center.y + dy / this.view.scale;
        this.render();
        return;
      }
      if (this.dragging) {
        const w = this.screenToWorld(e.offsetX, e.offsetY);
        const state = this.getState();
        const list = this.dragging.type === "robot" ? state.robots : state.fixtures;
        const item = list[this.dragging.index];
        if (item) {
          item.position.x = w.x;
          item.position.y = w.y;
          this.render();
          if (this.onItemMoved) this.onItemMoved();
        }
      }
    });

    window.addEventListener("mouseup", () => {
      this.panning = null;
      this.dragging = null;
    });
  }

  _hitTest(sx, sy) {
    const state = this.getState();
    // Hit-test robots first (smaller, drawn on top)
    for (let i = state.robots.length - 1; i >= 0; i--) {
      const r = state.robots[i];
      const c = this.worldToScreen(r.position.x, r.position.y);
      const r_px = ROBOT_RADIUS_M * this.view.scale;
      const dx = sx - c.x, dy = sy - c.y;
      if (dx * dx + dy * dy <= r_px * r_px) return { type: "robot", index: i };
    }
    for (let i = state.fixtures.length - 1; i >= 0; i--) {
      const f = state.fixtures[i];
      const c = this.worldToScreen(f.position.x, f.position.y);
      const half_px = FIXTURE_HALF_M * this.view.scale;
      if (Math.abs(sx - c.x) <= half_px && Math.abs(sy - c.y) <= half_px) {
        return { type: "fixture", index: i };
      }
    }
    return null;
  }

  _fitDpr() {
    const dpr = window.devicePixelRatio || 1;
    // Measure the CONTAINER, not the canvas itself: _fitDpr sets the canvas's
    // inline style.width/height to a fixed px value below, so if we measured
    // canvas.getBoundingClientRect() we'd just read back whatever we last set
    // it to — a self-reference that can never detect the container actually
    // growing (e.g. the window/panel resizing, an aside collapsing). The
    // container's box is driven by the real CSS grid layout, so it always
    // reflects the true available space.
    const container = this.canvas.parentElement || this.canvas;
    const rect = container.getBoundingClientRect();
    this.canvas.width = rect.width * dpr;
    this.canvas.height = rect.height * dpr;
    this.canvas.style.width = rect.width + "px";
    this.canvas.style.height = rect.height + "px";
    this.render();
  }
}

function pickGridStep(scale) {
  // pick a grid step in metres so cells are between 30 and 200 px on screen
  const candidates = [0.1, 0.25, 0.5, 1, 2, 5, 10, 25, 50, 100];
  for (const c of candidates) {
    const px = c * scale;
    if (px >= 30 && px <= 200) return c;
  }
  return scale > 60 ? 0.1 : 100;
}
