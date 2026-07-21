// FR-053: WebGL 3D URDF viewer. three.js scene + OrbitControls (the
// rotate/zoom/pan "xyz-rotate" feature) + urdf-loader with a custom
// loadMeshCb because assets are ~99% .obj (urdf-loader's default handles
// only .stl/.dae/glTF).

import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { OBJLoader } from "three/addons/loaders/OBJLoader.js";
import { MTLLoader } from "three/addons/loaders/MTLLoader.js";
import { STLLoader } from "three/addons/loaders/STLLoader.js";
import { ColladaLoader } from "three/addons/loaders/ColladaLoader.js";
import { ViewHelper } from "three/addons/helpers/ViewHelper.js";
import URDFLoader from "urdf-loader";
import { PointerURDFDragControls } from "urdf-loader/URDFDragControls.js";

const ROBOT_COLOR = 0xb8bfc6; // neutral grey — robot geometry is themed by its own MTL colors, not the UI theme

/** Read a `--css-var` from `:root` (kept live so theme.js's overrides apply immediately). */
function cssVar(name, fallback) {
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

/** True if node (or any ancestor) was loaded with MTL materials. */
function hasMtlAncestor(node) {
  for (let n = node; n; n = n.parent) {
    if (n.userData?.hasMtl) return true;
  }
  return false;
}
/** Fresnel edge-glow shader for the reach envelope — transparent when viewed
 *  head-on, glowing blue/cyan at grazing angles (like a holographic boundary). */
function createReachMaterial() {
  return new THREE.ShaderMaterial({
    transparent: true,
    depthWrite: false,
    side: THREE.DoubleSide,
    uniforms: {
      glowColor: { value: new THREE.Color(0x00ccff) },
      edgeColor: { value: new THREE.Color(0x4488ff) },
    },
    vertexShader: /* glsl */ `
      varying vec3 vNormal;
      varying vec3 vViewDir;
      void main() {
        vec4 mvPos = modelViewMatrix * vec4(position, 1.0);
        vNormal = normalize(normalMatrix * normal);
        vViewDir = normalize(-mvPos.xyz);
        gl_Position = projectionMatrix * mvPos;
      }
    `,
    fragmentShader: /* glsl */ `
      uniform vec3 glowColor;
      uniform vec3 edgeColor;
      varying vec3 vNormal;
      varying vec3 vViewDir;
      void main() {
        float fresnel = 1.0 - abs(dot(vNormal, vViewDir));
        fresnel = pow(fresnel, 2.5);
        vec3 color = mix(edgeColor, glowColor, fresnel);
        float alpha = fresnel * 0.7 + 0.04; // near-invisible center, bright edges
        gl_FragColor = vec4(color, alpha);
      }
    `,
  });
}

// A dressed robot can reference thousands of meshes (per-triangle collision
// hulls). Firing them all as concurrent fetches exhausts the browser's
// socket/memory pool (ERR_INSUFFICIENT_RESOURCES), so mesh loads run through
// a small concurrency-limited queue.
const MAX_CONCURRENT_LOADS = 6;

/** Minimal FIFO task queue capping how many async tasks run at once. */
class LoadQueue {
  constructor(limit) {
    this.limit = limit;
    this.active = 0;
    this.pending = [];
  }
  /** task: (done) => void — must call done() exactly once when finished. */
  push(task) {
    this.pending.push(task);
    this._pump();
  }
  _pump() {
    while (this.active < this.limit && this.pending.length) {
      const task = this.pending.shift();
      this.active++;
      let settled = false;
      const done = () => {
        if (settled) return;
        settled = true;
        this.active--;
        this._pump();
      };
      try {
        task(done);
      } catch (e) {
        console.error("mesh load task threw", e);
        done();
      }
    }
  }
}

export class Viewer {
  constructor(canvasEl) {
    this.canvas = canvasEl;
    this.robot = null;
    this.showCollision = false; // collision-hull overlay toggle (FR-058)
    // Callbacks the app wires: (name, value) on drag, (name|null) on hover.
    this.onJointChange = null;
    this.onJointHover = null;
    this._loadQueue = new LoadQueue(MAX_CONCURRENT_LOADS);
    this._collisionUrl = null;    // URDF url, for lazy collision loading
    this._collisionGroup = null;  // separate parseCollision=true robot overlay
    this._collisionLoaded = false;
    this._reachGroup = null;      // reach-envelope hull overlay (FR-028)
    this.showReach = false;

    this.scene = new THREE.Scene();
    this.scene.background = new THREE.Color().setStyle(cssVar("--bg", "#1a1c1e"));

    // Work natively in URDF's Z-up frame so the models, the origin axes, and
    // the corner gizmo all agree. (Rotating a Y-up scene instead makes the
    // gizmo — which reflects true world orientation — disagree with the view.)
    this.camera = new THREE.PerspectiveCamera(50, 1, 0.01, 1000);
    this.camera.up.set(0, 0, 1);
    this.camera.position.set(2, -2, 1.6);

    this.renderer = new THREE.WebGLRenderer({ canvas: canvasEl, antialias: true });
    this.renderer.setPixelRatio(window.devicePixelRatio || 1);
    this.renderer.autoClear = false; // ViewHelper renders as a second pass

    // Identity group: models load directly in their Z-up frame, no rotation.
    this.world = new THREE.Group();
    this.scene.add(this.world);

    this.controls = new OrbitControls(this.camera, this.renderer.domElement);
    this.controls.enableDamping = true;

    this._initSceneDecor();
    this._initAxisGizmo();
    this._initDragControls();
    this._installResize();
    this._animate();
  }

  _initSceneDecor() {
    // GridHelper lies in the XZ plane by default; rotate it into the Z-up
    // XY plane so it reads as the floor in the URDF frame.
    const grid = new THREE.GridHelper(
      10, 20,
      new THREE.Color().setStyle(cssVar("--line", "#3a3f44")),
      new THREE.Color().setStyle(cssVar("--bg-elev", "#2c3034")),
    );
    grid.rotation.x = Math.PI / 2;
    this.scene.add(grid);
    this._grid = grid; // kept so refreshTheme() can recolor it live

    const axes = new THREE.AxesHelper(0.5); // X=red, Y=green, Z=blue (Z up)
    this.scene.add(axes);

    this.scene.add(new THREE.HemisphereLight(0xffffff, 0x444444, 1.1));
    const dir = new THREE.DirectionalLight(0xffffff, 1.5);
    dir.position.set(3, -5, 4);
    this.scene.add(dir);
  }

  /**
   * Corner XYZ axis gizmo (bottom-right) — a passive orientation indicator so
   * you can always read which way X/Y/Z point. Click-to-snap is intentionally
   * NOT wired: ViewHelper's snap is hard-coded for Y-up and conflicts with our
   * Z-up OrbitControls (snapping to +Z hits the control's gimbal pole).
   */
  _initAxisGizmo() {
    this.viewHelper = new ViewHelper(this.camera, this.renderer.domElement);
    this.viewHelper.setLabels("X", "Y", "Z");
  }

  /**
   * Pointer joint dragging (like the urdf-loader demo): hovering a joint
   * suppresses OrbitControls so a left-drag articulates that joint instead of
   * rotating the camera; the hovered joint's name shows in the viewport.
   * (No mesh recolor — it flickered on hover.)
   */
  _initDragControls() {
    const dc = new PointerURDFDragControls(this.scene, this.camera, this.renderer.domElement);

    dc.onHover = (joint) => {
      this.controls.enabled = false; // drag will move the joint, not the camera
      if (this.onJointHover) this.onJointHover(joint.name);
    };
    dc.onUnhover = () => {
      this.controls.enabled = true;
      if (this.onJointHover) this.onJointHover(null);
    };
    dc.onDragStart = () => { this.controls.enabled = false; };
    dc.onDragEnd = () => { this.controls.enabled = true; };
    dc.updateJoint = (joint, angle) => {
      joint.setJointValue(angle);
      if (this.onJointChange) this.onJointChange(joint.name, joint.angle);
    };

    this.dragControls = dc;
  }

  _installResize() {
    const ro = new ResizeObserver(() => this.resize());
    ro.observe(this.canvas.parentElement || this.canvas);
    this.resize();
  }

  resize() {
    const el = this.canvas;
    const w = el.clientWidth || 1;
    const h = el.clientHeight || 1;
    this.renderer.setSize(w, h, false);
    this.camera.aspect = w / h;
    this.camera.updateProjectionMatrix();
  }

  _animate() {
    const loop = () => {
      requestAnimationFrame(loop);
      this.controls.update();
      this.renderer.clear();
      this.renderer.render(this.scene, this.camera);
      this.viewHelper.render(this.renderer); // corner gizmo, second pass
    };
    loop();
  }

  /** Remove the current robot and free its GPU resources. */
  clear() {
    if (!this.robot) return;
    if (this.dragControls) {
      this.dragControls.hovered = null;
      this.dragControls.manipulating = null;
    }
    this.controls.enabled = true;
    const dispose = (root) => {
      this.world.remove(root);
      root.traverse((o) => {
        if (o.geometry) o.geometry.dispose();
        if (o.material) {
          const mats = Array.isArray(o.material) ? o.material : [o.material];
          mats.forEach((m) => m.dispose());
        }
      });
    };
    dispose(this.robot);
    if (this._collisionGroup) dispose(this._collisionGroup);
    if (this._reachGroup) dispose(this._reachGroup);
    this.robot = null;
    this._collisionGroup = null;
    this._collisionLoaded = false;
    this._reachGroup = null;
  }

  /**
   * Load a URDF from `url` (an /assets/... path). Returns the URDFRobot.
   * urdf-loader resolves each relative <mesh filename> against `url`.
   *
   * Only VISUAL geometry is loaded here — collision hulls (which can number
   * in the thousands for per-triangle hulls) are fetched lazily the first
   * time the collision toggle is turned on. All mesh loads run through a
   * concurrency-limited queue to avoid flooding the browser.
   */
  loadUrdf(url) {
    this._collisionUrl = url; // remembered for lazy collision loading
    return new Promise((resolve, reject) => {
      const manager = new THREE.LoadingManager();
      const loader = new URDFLoader(manager);
      loader.packages = ""; // meshes are ./-relative to the URDF URL
      loader.parseVisual = true;
      loader.parseCollision = false; // lazy — see setCollisionVisible()
      loader.loadMeshCb = this._meshCb();

      loader.load(
        url,
        (robot) => {
          this.clear();
          this.robot = robot;
          this._collisionGroup = null;
          this._collisionLoaded = false;
          this.world.add(robot);
          this._decorate(robot);
          this.fitCamera(robot);
          resolve(robot);
        },
        undefined,
        (err) => reject(err instanceof Error ? err : new Error(String(err))),
      );
    });
  }

  /** Build a throttled loadMeshCb that dispatches by extension. */
  _meshCb() {
    return (path, mgr, done) => {
      this._loadQueue.push((taskDone) => {
        const finishTask = (obj) => { done(obj); taskDone(); };
        const ext = path.split(/[?#]/)[0].split(".").pop().toLowerCase();
        const isTool = path.includes("/meshes/tool/");
        const finish = (obj) => {
          if (obj) obj.userData.isTool = isTool;
          finishTask(obj);
        };
        if (ext === "obj") {
          // Try the paired .mtl first (same basename, served at same path).
          // On success: real material colors. On 404: plain OBJ fallback.
          const mtlUrl = path.replace(/\.obj$/i, ".mtl");
          new MTLLoader(mgr).load(
            mtlUrl,
            (materials) => {
              materials.preload();
              new OBJLoader(mgr)
                .setMaterials(materials)
                .load(path, (obj) => {
                  if (obj) obj.userData.hasMtl = true;
                  finish(obj);
                }, undefined, () => finish(null));
            },
            undefined,
            () => {
              // No .mtl found — load OBJ without materials (fallback to _decorate).
              new OBJLoader(mgr).load(path, finish, undefined, () => finish(null));
            },
          );
        } else if (ext === "stl") {
          new STLLoader(mgr).load(path, (geo) => finish(new THREE.Mesh(geo)), undefined, () => finish(null));
        } else if (ext === "dae") {
          new ColladaLoader(mgr).load(path, (dae) => finish(dae.scene), undefined, () => finish(null));
        } else {
          finish(null); // unknown ext: skip this link, keep the robot assembling
        }
      });
    };
  }

  /** Material pass for the visual robot. Meshes with MTL materials keep their
   *  real colors; meshes without (STL/DAE/generated hulls) get the neutral
   *  grey/orange fallback. */
  _decorate(robot) {
    robot.traverse((node) => {
      if (!node.isMesh) return;
      // Walk up to find the mesh-group tagged by loadMeshCb.
      if (hasMtlAncestor(node)) return; // real MTL colors — don't override
      const color = node.userData.isTool
        ? new THREE.Color().setStyle(cssVar("--accent", "#ff8a3d"))
        : ROBOT_COLOR;
      node.material = new THREE.MeshStandardMaterial({ color, metalness: 0.1, roughness: 0.7 });
    });
  }

  /**
   * FR-058: show/hide the collision-hull overlay. The hulls are loaded
   * lazily on first enable (a separate parseCollision=true pass) so a robot
   * with thousands of hull meshes doesn't fetch them until asked.
   */
  setCollisionVisible(visible) {
    this.showCollision = visible;
    if (!this.robot || !this._collisionUrl) return;

    if (visible && !this._collisionLoaded) {
      this._collisionLoaded = true; // guard against double-loading
      const manager = new THREE.LoadingManager();
      const loader = new URDFLoader(manager);
      loader.packages = "";
      loader.parseVisual = false;
      loader.parseCollision = true; // load ONLY the hulls this time
      loader.loadMeshCb = this._meshCb();
      loader.load(this._collisionUrl, (collision) => {
        collision.traverse((node) => {
          if (node.isMesh) {
            node.material = new THREE.MeshBasicMaterial({
              color: new THREE.Color().setStyle(cssVar("--ok", "#5fd07a")),
              wireframe: true, transparent: true, opacity: 0.6,
            });
          }
        });
        // Match the visual robot's current pose.
        if (this.robot?.joints) {
          for (const [n, j] of Object.entries(this.robot.joints)) {
            if (collision.joints?.[n]) collision.setJointValue(n, j.angle);
          }
        }
        collision.visible = this.showCollision; // may have been toggled off meanwhile
        this._collisionGroup = collision;
        this.world.add(collision);
      });
      return;
    }

    if (this._collisionGroup) this._collisionGroup.visible = visible;
  }

  /** Frame the model: place the camera on the model's bounding sphere. */
  fitCamera(object3d) {
    object3d.updateMatrixWorld(true);
    const box = new THREE.Box3().setFromObject(object3d);
    if (box.isEmpty()) return;
    const size = box.getSize(new THREE.Vector3());
    const center = box.getCenter(new THREE.Vector3());
    const radius = Math.max(size.x, size.y, size.z) * 0.5 || 1;
    const dist = radius / Math.tan((this.camera.fov * Math.PI) / 360) * 1.6;
    this.controls.target.copy(center);
    // Z-up 3/4 view: from +X, -Y, and above (+Z).
    this.camera.position.copy(center).add(new THREE.Vector3(1, -1, 0.8).normalize().multiplyScalar(dist));
    this.camera.near = radius / 100;
    this.camera.far = radius * 100;
    this.camera.updateProjectionMatrix();
    this.controls.update();
  }

  /**
   * Re-read the `--bg` / `--line` / `--bg-elev` / `--accent` / `--ok` CSS
   * variables and re-apply them to the scene background, floor grid, tool
   * meshes, and any loaded collision-hull overlay — called by `theme.js`
   * whenever the host (Kiro/VS Code) theme changes, so an already-open
   * viewer updates live instead of only picking up the new theme on the
   * next URDF load.
   */
  refreshTheme() {
    this.scene.background = new THREE.Color().setStyle(cssVar("--bg", "#1a1c1e"));

    if (this._grid) {
      const gridColors = this._grid.material;
      const mats = Array.isArray(gridColors) ? gridColors : [gridColors];
      // GridHelper's two colors are baked into its vertex colors at
      // construction, not swappable via material — simplest correct fix is
      // rebuilding it in place.
      this.scene.remove(this._grid);
      this._grid.geometry.dispose();
      mats.forEach((m) => m.dispose());
      const grid = new THREE.GridHelper(
        10, 20,
        new THREE.Color().setStyle(cssVar("--line", "#3a3f44")),
        new THREE.Color().setStyle(cssVar("--bg-elev", "#2c3034")),
      );
      grid.rotation.x = Math.PI / 2;
      this.scene.add(grid);
      this._grid = grid;
    }

    if (this.robot) {
      const toolColor = new THREE.Color().setStyle(cssVar("--accent", "#ff8a3d"));
      this.robot.traverse((node) => {
        if (node.isMesh && node.userData.isTool && !hasMtlAncestor(node)) {
          node.material.color.copy(toolColor);
        }
      });
    }
    if (this._collisionGroup) {
      const hullColor = new THREE.Color().setStyle(cssVar("--ok", "#5fd07a"));
      this._collisionGroup.traverse((node) => {
        if (node.isMesh) node.material.color.copy(hullColor);
      });
    }
  }

  /** FR-055: drive a joint; urdf-loader propagates to downstream links. */
  setJointValue(name, value) {
    if (this.robot) this.robot.setJointValue(name, value);
    // Keep the collision overlay (if loaded) posed with the visual robot.
    if (this._collisionGroup) this._collisionGroup.setJointValue(name, value);
  }

  /**
   * FR-028: toggle the reach-envelope hull overlay. Lazy-loaded on first enable
   * from the given OBJ URL, rendered as a translucent blue shell. The envelope
   * represents the full workspace and doesn't change with joint articulation.
   */
  setReachVisible(visible, hullUrl) {
    this.showReach = visible;
    if (visible && !this._reachGroup && hullUrl) {
      this._loadQueue.push((done) => {
        new OBJLoader(new THREE.LoadingManager()).load(hullUrl, (obj) => {
          const mat = createReachMaterial();
          obj.traverse((n) => {
            if (n.isMesh) n.material = mat;
          });
          obj.visible = this.showReach;
          this._reachGroup = obj;
          this.world.add(obj);
          done();
        }, undefined, () => done());
      });
      return;
    }
    if (this._reachGroup) this._reachGroup.visible = visible;
  }
}
