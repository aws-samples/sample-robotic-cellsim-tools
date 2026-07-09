#!/usr/bin/env bash
#
# Build an installable .vsix for the robo-fitting-room VS Code extension.
#
# Usage:
#   ./build-vsix.sh              # build the .vsix into the package dir
#   ./build-vsix.sh --install    # build, then install into VS Code via `code`
#
# Requires: node + npm. @vscode/vsce is fetched on demand with npx, so
# there's nothing to install globally.

set -euo pipefail

# Always run from the extension directory, regardless of caller's cwd.
cd "$(dirname "$0")"

INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    *) echo "Unknown argument: $arg" >&2; exit 2 ;;
  esac
done

echo "==> Installing npm dependencies"
npm install

echo "==> Compiling TypeScript"
npm run compile

echo "==> Packaging .vsix"
# @vscode/vsce v3 requires Node >= 20 (its bundled undici references the
# `File` global). Pin to the 2.x line on Node 18; use the latest on Node 20+.
# On Node < 20, vsce's bundled undici crashes referencing the global
# `File`. Preload a shim that hoists it from node:buffer so packaging works
# without upgrading Node.
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
if [[ "${NODE_MAJOR}" -lt 20 ]]; then
  echo "    (Node ${NODE_MAJOR} detected → enabling File shim for vsce)"
  SHIM="$(pwd)/scripts/node18-file-shim.js"
  export NODE_OPTIONS="${NODE_OPTIONS:-} --require ${SHIM}"
fi
# --no-dependencies: js-yaml is vendored under media/vendor as a static
# asset, so there's no runtime dependency tree for vsce to walk.
npx --yes @vscode/vsce package --no-dependencies

VSIX="$(ls -t ./*.vsix 2>/dev/null | head -n1 || true)"
if [[ -z "${VSIX}" ]]; then
  echo "ERROR: no .vsix was produced" >&2
  exit 1
fi

echo ""
echo "Built: ${VSIX}"

if [[ "${INSTALL}" -eq 1 ]]; then
  if command -v code >/dev/null 2>&1; then
    echo "==> Installing into VS Code"
    code --install-extension "${VSIX}" --force
    echo "Installed. Reload VS Code, then run 'robo-fitting-room: Open Scene Editor'."
  else
    echo "WARNING: 'code' CLI not found on PATH; skipping install." >&2
    echo "Install manually: Extensions view → ... → Install from VSIX → ${VSIX}"
  fi
else
  echo ""
  echo "Install it with either:"
  echo "  code --install-extension \"${VSIX}\""
  echo "  or: VS Code → Extensions → ... → Install from VSIX…"
fi
