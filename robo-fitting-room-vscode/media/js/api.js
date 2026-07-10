// Thin RPC wrappers around the robo-fitting-room backend.
//
// Historically this webview called the server's `/api/v1` REST surface via
// cross-origin `fetch()`. It now delegates every call to the extension host
// through `bridge.rpc(method, params)`, which forwards the request to the
// `stdio-serve` child process over its newline-delimited JSON transport and
// returns the method's `result` (or throws on an `{error}` envelope).
//
// Every export keeps its exact name and parameter signature so `app.js`
// needs no changes.

import * as bridge from "./bridge.js";

/** Read a File as base64 (no data-URL prefix) for transport through the
 *  JSON envelope. Used only by `uploadObj`, whose payloads are small,
 *  user-picked single files. */
function fileToBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result;
      // result is a data URL: "data:<mime>;base64,<data>"; strip the prefix.
      const comma = result.indexOf(",");
      resolve(comma >= 0 ? result.slice(comma + 1) : result);
    };
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(file);
  });
}

export async function listRobots() {
  return bridge.rpc("assets.listRobots", {});
}

export async function listFixtures() {
  return bridge.rpc("assets.listFixtures", {});
}

export async function listFloorPlans() {
  return bridge.rpc("assets.listFloorPlans", {});
}

export async function urdfJoints(path) {
  return bridge.rpc("urdf.joints", { path });
}

export async function floorPlanSegments(path) {
  return bridge.rpc("floorPlan.segments", { path });
}

export async function validateYaml(yamlText) {
  return bridge.rpc("compose.validate", { yaml: yamlText });
}

export async function saveScene(path, yamlText) {
  return bridge.rpc("scene.save", { path, yaml: yamlText });
}

export async function loadScene(path) {
  const res = await bridge.rpc("scene.load", { path });
  return res.yaml;
}

export async function uploadObj(file, filename) {
  const dataBase64 = await fileToBase64(file);
  return bridge.rpc("assets.uploadFloorPlanObj", { filename, dataBase64 });
}

// NOTE: compose + compose-urdf-bundle are intentionally NOT called from
// the webview. Those responses can be 100+ MB zips; streaming them through
// the webview and back out to disk is wasteful and hits postMessage size
// limits. Instead app.js delegates compose to the extension host (Node),
// which drives the same operations and writes straight to a file the user
// picks. See bridge.compose().
