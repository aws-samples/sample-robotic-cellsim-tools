#!/usr/bin/env bash
#
# Build an installable .vsix for the robo-fitting-room VS Code extension,
# with the native `robo-fitting-room` Rust binary bundled inside it.
#
# The produced .vsix is ARCHITECTURE-SPECIFIC: it contains a binary built
# for the host platform, and its filename encodes that platform (e.g.
# `robo-fitting-room-vscode-darwin-arm64-<version>.vsix`) so you know which
# one to install on which machine.
#
# Usage:
#   ./build-vsix.sh              # build the arch-specific .vsix into this dir
#   ./build-vsix.sh --install    # build, then install into VS Code via `code`
#
# Requires: node + npm + a Rust toolchain (cargo). @vscode/vsce is fetched on
# demand with npx, so there's nothing to install globally.

set -euo pipefail

# Always run from the extension directory, regardless of caller's cwd.
cd "$(dirname "$0")"

# Path to the Rust crate that produces the `robo-fitting-room` binary.
RUST_CRATE_DIR="../robo-fitting-room"

INSTALL=0
for arg in "$@"; do
  case "$arg" in
    --install) INSTALL=1 ;;
    *) echo "Unknown argument: $arg" >&2; exit 2 ;;
  esac
done

# --------------------------------------------------------------------------
# Detect the host platform and map it to (a) the Rust binary name and (b) the
# VS Code "target" triple vsce uses to name/scope a platform-specific .vsix.
# https://code.visualstudio.com/api/working-with-extensions/publishing-extension#platformspecific-extensions
# --------------------------------------------------------------------------
UNAME_S="$(uname -s)"
UNAME_M="$(uname -m)"
BIN_NAME="robo-fitting-room"
case "${UNAME_S}/${UNAME_M}" in
  Darwin/arm64)        VSCE_TARGET="darwin-arm64" ;;
  Darwin/x86_64)       VSCE_TARGET="darwin-x64" ;;
  Linux/x86_64)        VSCE_TARGET="linux-x64" ;;
  Linux/aarch64)       VSCE_TARGET="linux-arm64" ;;
  Linux/arm64)         VSCE_TARGET="linux-arm64" ;;
  Linux/armv7l)        VSCE_TARGET="linux-armhf" ;;
  MINGW*/x86_64|MSYS*/x86_64|CYGWIN*/x86_64) VSCE_TARGET="win32-x64"; BIN_NAME="robo-fitting-room.exe" ;;
  *)
    echo "ERROR: unsupported host platform '${UNAME_S}/${UNAME_M}'." >&2
    echo "       Add a mapping to a VS Code target in build-vsix.sh." >&2
    exit 1
    ;;
esac
echo "==> Host platform: ${UNAME_S}/${UNAME_M} → VS Code target '${VSCE_TARGET}'"

echo "==> Building native robo-fitting-room binary (cargo build --release)"
# Default features (server + stdio) so the bundled binary supports both the
# extension's `stdio-serve` transport and a manual `serve`. Override the
# feature set with RFR_CARGO_ARGS if you want a slimmer stdio-only binary,
# e.g. RFR_CARGO_ARGS="--no-default-features --features stdio".
cargo build --release --manifest-path "${RUST_CRATE_DIR}/Cargo.toml" ${RFR_CARGO_ARGS:-}

BUILT_BIN="${RUST_CRATE_DIR}/target/release/${BIN_NAME}"
if [[ ! -f "${BUILT_BIN}" ]]; then
  echo "ERROR: expected built binary not found at ${BUILT_BIN}" >&2
  exit 1
fi

echo "==> Bundling binary into ./bin/${BIN_NAME}"
rm -rf ./bin
mkdir -p ./bin
cp "${BUILT_BIN}" "./bin/${BIN_NAME}"
chmod +x "./bin/${BIN_NAME}" || true

echo "==> Installing npm dependencies"
npm install

echo "==> Compiling TypeScript"
npm run compile

echo "==> Packaging platform-specific .vsix (target: ${VSCE_TARGET})"
# @vscode/vsce v3 requires Node >= 20 (its bundled undici references the
# `File` global). On Node < 20, preload a shim that hoists `File` from
# node:buffer so packaging works without upgrading Node.
NODE_MAJOR="$(node -p 'process.versions.node.split(".")[0]')"
if [[ "${NODE_MAJOR}" -lt 20 ]]; then
  echo "    (Node ${NODE_MAJOR} detected → enabling File shim for vsce)"
  SHIM="$(pwd)/scripts/node18-file-shim.js"
  export NODE_OPTIONS="${NODE_OPTIONS:-} --require ${SHIM}"
fi
# --target: produce a platform-specific package; vsce encodes the target into
#           the .vsix filename so the right one is obvious per machine.
# --no-dependencies: js-yaml is vendored under media/vendor as a static asset,
#           so there's no runtime dependency tree for vsce to walk.
npx --yes @vscode/vsce package --target "${VSCE_TARGET}" --no-dependencies

# Prefer the target-tagged artifact if present; fall back to newest .vsix.
VSIX="$(ls -t ./*"${VSCE_TARGET}"*.vsix 2>/dev/null | head -n1 || true)"
if [[ -z "${VSIX}" ]]; then
  VSIX="$(ls -t ./*.vsix 2>/dev/null | head -n1 || true)"
fi
if [[ -z "${VSIX}" ]]; then
  echo "ERROR: no .vsix was produced" >&2
  exit 1
fi

echo ""
echo "Built: ${VSIX}"
echo "  (bundles bin/${BIN_NAME} for ${VSCE_TARGET})"

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
