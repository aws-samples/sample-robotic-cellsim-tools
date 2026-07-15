/**
 * Pure HTML generator for the RoboDresserViewerPanel.
 *
 * Modeled on `cell-runner-frontend`'s `get3DViewerHtml` (an iframe-based webview
 * pointed at a locally-spawned HTTP server): the Robo Dresser server already
 * ships a complete browser SPA, so the panel embeds it via an iframe instead of
 * re-implementing any UI.
 */

/**
 * Req 4.1: given the running Robo_Dresser_Server's base URL (`http://<host>:<port>`,
 * no trailing slash), generate the webview panel HTML. The iframe `src` attribute
 * is exactly `http://<host>:<port>/`.
 */
export function getRoboDresserViewerHtml(baseUrl: string): string {
  const iframeSrc = `${baseUrl}/`;

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8"/>
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; frame-src http://127.0.0.1:* http://localhost:*;"/>
<style>
* { box-sizing: border-box; margin: 0; padding: 0; }
body { display: flex; height: 100vh; overflow: hidden; }
.viewer-frame { flex: 1; border: none; width: 100%; height: 100%; }
</style>
</head>
<body>
<iframe class="viewer-frame" src="${iframeSrc}"></iframe>
</body>
</html>`;
}
