// Node 18 shim: @vscode/vsce's bundled `undici` references the global
// `File`, which only became global in Node 20. Node 18 still ships the
// class under `node:buffer`, so we hoist it onto globalThis. Loaded via
// NODE_OPTIONS=--require by build-vsix.sh, only when running on Node < 20.
if (typeof globalThis.File === "undefined") {
  try {
    const { File } = require("node:buffer");
    if (File) {
      globalThis.File = File;
    }
  } catch {
    // Older runtime without node:buffer File — nothing we can do; let vsce
    // surface its own error.
  }
}
