/**
 * Pure HTML generator for the RoboDresserViewerPanel.
 *
 * Modeled on `cell-runner-frontend`'s `get3DViewerHtml` (an iframe-based webview
 * pointed at a locally-spawned HTTP server): the Robo Dresser server already
 * ships a complete browser SPA, so the panel embeds it via an iframe instead of
 * re-implementing any UI.
 */

/**
 * VS Code/Kiro theme colors forwarded into the dresser SPA's iframe, keyed by
 * the CSS custom property names the SPA already declares defaults for in its
 * own `:root` (see `urdf-robo-dresser/web/style.css`). The iframe is a
 * cross-origin document (the loopback server, not `vscode-webview:`), so it
 * gets none of the `--vscode-*` variables VS Code injects into this wrapper
 * document automatically — they have to be relayed explicitly via
 * `postMessage`.
 */
const THEME_COLOR_MAP: Record<string, string> = {
  "--bg": "--vscode-editor-background",
  "--bg-pane": "--vscode-sideBar-background",
  "--bg-elev": "--vscode-editorWidget-background",
  "--fg": "--vscode-foreground",
  "--fg-muted": "--vscode-descriptionForeground",
  "--accent": "--vscode-button-background",
  "--accent-fg": "--vscode-button-foreground",
  "--line": "--vscode-panel-border",
  "--selected": "--vscode-list-activeSelectionForeground",
  "--ok": "--vscode-charts-green",
  "--warn": "--vscode-editorWarning-foreground",
  "--err": "--vscode-editorError-foreground",
};

/**
 * Req 4.1: given the running Robo_Dresser_Server's base URL (`http://<host>:<port>`,
 * no trailing slash), generate the webview panel HTML. The iframe `src` attribute
 * is exactly `http://<host>:<port>/`.
 *
 * The wrapper also relays the active Kiro/VS Code theme's colors into the
 * iframe (see `THEME_COLOR_MAP` above) so the dresser SPA matches the editor
 * instead of always rendering its own fixed dark palette. This requires
 * `script-src 'unsafe-inline'` in the CSP below, alongside the pre-existing
 * `style-src 'unsafe-inline'` — the bridge script only reads this document's
 * own computed CSS variables and forwards color strings to the already-
 * trusted, locally-spawned dresser server's iframe; it accepts no external
 * input and loads no remote code.
 */
export function getRoboDresserViewerHtml(baseUrl: string): string {
  const iframeSrc = `${baseUrl}/`;

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"/>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; frame-src http://127.0.0.1:* http://localhost:*;"/>
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
body { display: flex; height: 100vh; overflow: hidden; }
.viewer-frame { flex: 1; border: none; width: 100%; height: 100%; }
</style>
</head>
<body>
<iframe class="viewer-frame" src="${iframeSrc}"></iframe>
<script>
(function () {
  var COLOR_MAP = ${JSON.stringify(THEME_COLOR_MAP)};
  var targetOrigin = ${JSON.stringify(baseUrl)}; // postMessage wants an origin, not the "/" path
  var iframe = document.querySelector(".viewer-frame");

  function readTheme() {
    var cs = getComputedStyle(document.documentElement);
    var colors = {};
    for (var target in COLOR_MAP) {
      var value = cs.getPropertyValue(COLOR_MAP[target]).trim();
      if (value) colors[target] = value;
    }
    return colors;
  }

  function postTheme() {
    if (!iframe.contentWindow) return;
    iframe.contentWindow.postMessage({ type: "robo-dresser:theme", colors: readTheme() }, targetOrigin);
  }

  iframe.addEventListener("load", postTheme);
  window.addEventListener("message", function (event) {
    if (event.source !== iframe.contentWindow) return; // only trust our own iframe
    if (event.data && event.data.type === "robo-dresser:request-theme") postTheme();
  });

  // Kiro/VS Code toggles a "vscode-light"/"vscode-dark"/"vscode-high-contrast"
  // class on <body> whenever the active color theme changes; re-forward the
  // (now updated) computed colors when that happens.
  new MutationObserver(postTheme).observe(document.body, { attributes: true, attributeFilter: ["class"] });
})();
</script>
</body>
</html>`;
}
