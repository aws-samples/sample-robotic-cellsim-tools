// Applies Kiro/VS Code editor theme colors to this document when embedded in
// the `urdf-robo-dresser-vscode` extension's webview panel (see
// `viewerHtml.ts`'s inline bridge script, which reads its own
// `--vscode-*` CSS variables and relays them here via postMessage — this
// document is a separate origin/iframe and has no direct access to them).
//
// Standalone (plain browser, no wrapper) is unaffected: no message ever
// arrives, so `style.css`'s built-in `:root` defaults keep applying.

const THEME_VARS = [
  "--bg", "--bg-pane", "--bg-elev", "--fg", "--fg-muted",
  "--accent", "--accent-fg", "--line", "--selected", "--ok", "--warn", "--err",
];

function applyTheme(colors) {
  const root = document.documentElement.style;
  let changed = false;
  for (const name of THEME_VARS) {
    const value = colors && colors[name];
    if (value) {
      root.setProperty(name, value);
      changed = true;
    }
  }
  // Let app.js (which owns the Viewer instance) re-color the WebGL scene —
  // canvas colors are drawn once at load time and don't repaint just because
  // a CSS variable changed, unlike the rest of the (CSS-driven) UI.
  if (changed) window.dispatchEvent(new CustomEvent("robo-dresser:theme-applied"));
}

window.addEventListener("message", (event) => {
  if (event.source !== window.parent || event.source === window) return; // only trust our embedding parent
  const data = event.data;
  if (data && data.type === "robo-dresser:theme") applyTheme(data.colors);
});

// In case the wrapper's iframe "load" handler fired before this script ran
// (unlikely given script placement, but harmless either way), ask for the
// theme explicitly once this document is ready. Ignored by standalone
// browser use, since there's no parent listening for it.
if (window.parent !== window) {
  window.parent.postMessage({ type: "robo-dresser:request-theme" }, "*");
}
