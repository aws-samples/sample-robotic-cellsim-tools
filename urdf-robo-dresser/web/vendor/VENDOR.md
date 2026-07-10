# Vendored runtime dependencies (NFR-014)

These files are committed so the SPA renders with **no CDN / network fetch at
runtime** (FR-052 "without external dependencies"). They are fetched once at
authoring time and pinned here. Do not add a `<script src="https://...">` or an
`import` from a URL to `web/` — resolve everything through the import map in
`web/index.html`.

| Path | Package | Version | License |
|------|---------|---------|---------|
| `three/three.module.js` | [three](https://github.com/mrdoob/three.js) | r0.169.0 | MIT |
| `three/addons/controls/OrbitControls.js` | three examples/jsm | r0.169.0 | MIT |
| `three/addons/loaders/OBJLoader.js` | three examples/jsm | r0.169.0 | MIT |
| `three/addons/loaders/MTLLoader.js` | three examples/jsm | r0.169.0 | MIT |
| `three/addons/loaders/STLLoader.js` | three examples/jsm | r0.169.0 | MIT |
| `three/addons/loaders/ColladaLoader.js` | three examples/jsm | r0.169.0 | MIT |
| `three/addons/loaders/TGALoader.js` | three examples/jsm (ColladaLoader dep) | r0.169.0 | MIT |
| `three/addons/helpers/ViewHelper.js` | three examples/jsm (corner axis gizmo) | r0.169.0 | MIT |
| `urdf-loader/URDFLoader.js` | [urdf-loader](https://github.com/gkjohnson/urdf-loaders) | 0.12.6 | Apache-2.0 |
| `urdf-loader/URDFClasses.js` | urdf-loader | 0.12.6 | Apache-2.0 |
| `urdf-loader/URDFDragControls.js` | urdf-loader (pointer joint dragging) | 0.12.6 | Apache-2.0 |

Import-map specifiers (see `web/index.html`):
- `three` → `three/three.module.js`
- `three/addons/` → `three/addons/` (this crate's own imports)
- `three/examples/jsm/` → `three/addons/` (urdf-loader imports STL/Collada loaders under this path)
- `urdf-loader` → `urdf-loader/URDFLoader.js`

To refresh: re-fetch each file from `https://cdn.jsdelivr.net/npm/<pkg>@<ver>/<path>`
at the versions above, then confirm every `import ... from '...'` still resolves
to one of the specifiers listed here (grep for `http`/`https` — there must be none).
