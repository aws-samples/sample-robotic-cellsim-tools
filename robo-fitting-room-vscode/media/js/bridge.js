// Bridge between the webview and the VS Code extension host.
//
// The browser primitives the original SPA relied on — a hidden <input
// type=file> for loading, window.prompt() for a filename, and an anchor
// download for the composed zip — don't work (or don't behave well)
// inside a webview. Each is replaced by a request/response round-trip to
// the extension host, which uses native VS Code dialogs and Node file I/O.
//
// Protocol: every request carries a monotonically increasing `requestId`;
// the host echoes it back on the matching `*Result` message so we can
// resolve the right promise.

const vscode = acquireVsCodeApi();

let seq = 0;
const pending = new Map(); // requestId -> { resolve, reject }

window.addEventListener("message", (event) => {
  const msg = event.data;
  if (!msg || typeof msg.requestId !== "number") return;
  const entry = pending.get(msg.requestId);
  if (!entry) return;
  pending.delete(msg.requestId);
  entry.resolve(msg);
});

function request(payload) {
  const requestId = ++seq;
  return new Promise((resolve, reject) => {
    pending.set(requestId, { resolve, reject });
    vscode.postMessage({ ...payload, requestId });
  });
}

/** Open a native file dialog and read the chosen YAML.
 *  Resolves to { cancelled:true } or { cancelled:false, filename, text }. */
export function openYaml() {
  return request({ type: "openYaml" });
}

/** Ask the host for a manifest filename via an input box.
 *  Resolves to a string, or null if the user dismissed it. */
export async function promptSaveName(current) {
  const res = await request({ type: "promptSaveName", current });
  return res.name;
}

/** Delegate compose (+ optional URDF bundle) to the extension host, which
 *  fetches the zip(s) and writes them to a user-picked path.
 *  Resolves to { ok, message }. */
export function compose(params) {
  return request({ type: "compose", ...params });
}

/** Export the self-contained URDF source bundle as an extracted folder
 *  tree (scene.yaml + robots/ + fixtures/ + floorplans/). The host fetches
 *  the bundle and unpacks it under a user-picked directory.
 *  Resolves to { ok, message }. */
export function exportBundle(params) {
  return request({ type: "exportBundle", ...params });
}
