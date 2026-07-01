use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Seek, Write};
use std::path::{Path, PathBuf};

use urdf_robo_dresser::{
    parse_urdf, Collision, Geometry, Inertial, Joint, JointType, Link, Origin, UrdfRobot, Visual,
};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::error::FittingRoomError;
use crate::floor_plan::FloorPlan;
use crate::math3::{self, Mat4};
use crate::mtl::{parse_mtl, Material};
use crate::obj::{parse_obj, ObjMesh};
use crate::scene::{FixturePlacement, RobotPlacement, SceneDefinition};

#[derive(Debug, Clone)]
pub struct ComposeReport {
    pub zip_path: PathBuf,
    pub usda_name: String,
    pub manifest_name: String,
    pub robot_count: usize,
    pub fixture_count: usize,
    pub floor_plan_segments: usize,
    pub mesh_count: usize,
    pub total_vertices: usize,
    pub total_faces: usize,
    pub usda_bytes: u64,
    pub zip_bytes: u64,
}

/// Caches parsed OBJs by canonical source path so the same mesh used by
/// MeshCache parses each OBJ once. Geometry is then emitted inline at every
/// visual/collision — same OBJ used in 200 places writes the points/indices
/// 200 times. Larger USDA, but each prim is fully self-contained which
/// avoids reference resolution issues we saw earlier.
#[derive(Default)]
struct MeshCache {
    map: BTreeMap<PathBuf, ObjMesh>,
}

impl MeshCache {
    fn load(&mut self, urdf_dir: &Path, mesh_filename: &str) -> Result<&ObjMesh, FittingRoomError> {
        let abs = urdf_dir.join(mesh_filename);
        let key = std::fs::canonicalize(&abs).unwrap_or_else(|_| abs.clone());
        if !self.map.contains_key(&key) {
            let mesh = parse_obj(&abs)?;
            self.map.insert(key.clone(), mesh);
        }
        Ok(self.map.get(&key).unwrap())
    }
}

/// Tracks materials encountered while emitting visuals. Keyed by globally-unique
/// material name (`<urdf_stem>__<mtl_name>` sanitised) so the same RAL colour
/// from different URDFs is emitted as separate Material prims (in case Kd
/// values differ slightly across URDFs).
#[derive(Default)]
struct MaterialRegistry {
    /// usd-prim-name → material data, ordered by insertion (BTreeMap = sorted = deterministic).
    materials: BTreeMap<String, Material>,
    /// Cache of parsed .mtl files by canonical path → material lookup table.
    mtl_cache: BTreeMap<PathBuf, std::collections::HashMap<String, Material>>,
}

impl MaterialRegistry {
    /// Resolve and register the material referenced by an OBJ.
    /// Returns the USD prim name (under `/World/Looks/...`) to bind via
    /// `material:binding`, or None if the OBJ has no material.
    ///
    /// `obj_path` is the absolute path to the OBJ file — the MTL filename
    /// in `mesh.mtllib` is resolved relative to the OBJ's directory.
    fn register_for_mesh(
        &mut self,
        mesh: &ObjMesh,
        obj_path: &Path,
        urdf_stem: &str,
    ) -> Option<String> {
        let mtllib = mesh.mtllib.as_deref()?;
        let usemtl = mesh.usemtl.as_deref()?;
        let obj_dir = obj_path.parent()?;
        let mtl_path = obj_dir.join(mtllib);
        let key = std::fs::canonicalize(&mtl_path).unwrap_or_else(|_| mtl_path.clone());
        if !self.mtl_cache.contains_key(&key) {
            match parse_mtl(&mtl_path) {
                Ok(map) => {
                    self.mtl_cache.insert(key.clone(), map);
                }
                Err(e) => {
                    log::warn!(
                        "MTL parse failed for {}: {}; using placeholder material",
                        mtl_path.display(),
                        e
                    );
                    self.mtl_cache
                        .insert(key.clone(), std::collections::HashMap::new());
                }
            }
        }
        let mtl_table = self.mtl_cache.get(&key).unwrap();
        let material = mtl_table
            .get(usemtl)
            .cloned()
            .unwrap_or_else(Material::placeholder);

        let prim_name = sanitize_prim_name(&format!("{}__{}", urdf_stem, usemtl));
        self.materials.entry(prim_name.clone()).or_insert(material);
        Some(prim_name)
    }
}

/// Optional knobs for the compose pipeline.
#[derive(Debug, Clone)]
pub struct ComposeOptions {
    /// When true, emit full visual meshes for robots. When false (default),
    /// robots are emitted with only their collision hulls visible — leaner
    /// scene, identical articulation behaviour. Fixtures are unaffected.
    pub full_robots: bool,
    /// When true (default), keep full visual meshes for tool geometry
    /// merged into a robot URDF. When false, drop tool visuals and emit
    /// only their collision hulls. Tool meshes are detected by the
    /// `meshes/tool/` path prefix that urdf-robo-dresser writes; visuals
    /// without that prefix are treated as robot geometry and follow the
    /// `full_robots` flag.
    pub full_tools: bool,
}

impl Default for ComposeOptions {
    fn default() -> Self {
        Self {
            full_robots: false,
            full_tools: true,
        }
    }
}

// FR-140
pub fn compose_stage(
    scene: &SceneDefinition,
    manifest_path: &Path,
    out_path: &Path,
) -> Result<ComposeReport, FittingRoomError> {
    compose_stage_with_options(scene, manifest_path, out_path, &ComposeOptions::default())
}

pub fn compose_stage_with_options(
    scene: &SceneDefinition,
    manifest_path: &Path,
    out_path: &Path,
    options: &ComposeOptions,
) -> Result<ComposeReport, FittingRoomError> {
    let ext = out_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "zip" {
        return Err(FittingRoomError::UnsupportedFormat(format!(
            ".{} (only .zip is supported)",
            ext
        )));
    }

    // Validate the output has a usable stem (we don't otherwise read it —
    // the USDA name inside the zip is derived from the MANIFEST stem so
    // transient names like `.__compose_*.zip` don't leak into the user-
    // visible filename).
    out_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| FittingRoomError::UnsupportedFormat("output path has no stem".into()))?;
    let manifest_stem = manifest_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim_start_matches('.').trim_start_matches("__compose_"))
        .map(|s| s.trim_start_matches('.'))
        .filter(|s| !s.is_empty())
        .unwrap_or("scene")
        .to_string();
    let usda_name = format!("{}.usda", manifest_stem);
    let manifest_name = manifest_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "manifest.yaml".to_string());

    let mut cache = MeshCache::default();
    let mut materials = MaterialRegistry::default();
    let mut stats = MeshStats::default();

    // Body buffer: holds robots / fixtures / materials / actiongraphs.
    // The Meshes scope is emitted FIRST in the final output so that
    // `prepend references = </World/Meshes/...>` resolves at parse time.
    let mut body_buf: Vec<u8> = Vec::new();
    {
        let mut w = BufWriter::new(&mut body_buf);

        if !scene.robots.is_empty() {
            writeln!(w, "    def Scope \"robots\"")?;
            writeln!(w, "    {{")?;
            for placement in &scene.robots {
                emit_robot(
                    &mut w,
                    placement,
                    &mut cache,
                    &mut materials,
                    &mut stats,
                    options.full_robots,
                    options.full_tools,
                )?;
            }
            writeln!(w, "    }}")?;
            writeln!(w)?;
        }

        if !scene.fixtures.is_empty() {
            writeln!(w, "    def Scope \"fixtures\"")?;
            writeln!(w, "    {{")?;
            for placement in &scene.fixtures {
                emit_fixture(&mut w, placement, &mut cache, &mut materials, &mut stats)?;
            }
            writeln!(w, "    }}")?;
        }

        // FR-119: static mounting pedestals beneath any robot with height > 0.
        let pedestal_robots: Vec<&RobotPlacement> = scene
            .robots
            .iter()
            .filter(|r| r.pedestal.map(|p| p.height_mm > 0.0).unwrap_or(false))
            .collect();
        if !pedestal_robots.is_empty() {
            writeln!(w, "    def Scope \"pedestals\"")?;
            writeln!(w, "    {{")?;
            for placement in pedestal_robots {
                emit_pedestal(&mut w, placement)?;
            }
            writeln!(w, "    }}")?;
            writeln!(w)?;
        }

        if scene.floor_plan.is_some() {
            log::info!("Floor plan present in manifest but emission is disabled");
        }

        emit_materials(&mut w, &materials)?;
        emit_action_graphs(&mut w, &scene.name, &scene.robots)?;

        w.flush()?;
    }

    let mut usda_buf: Vec<u8> = Vec::new();
    {
        let mut w = BufWriter::new(&mut usda_buf);
        write_header(&mut w, &scene.name, &usda_name)?;
        writeln!(w, "def Xform \"World\"")?;
        writeln!(w, "{{")?;
        emit_physics_scene(&mut w)?;

        w.write_all(&body_buf)?;

        writeln!(w, "}}")?;
        writeln!(w)?;

        emit_environment(&mut w)?;
        emit_render_block(&mut w)?;
        emit_flat_grid(&mut w)?;

        w.flush()?;
    }

    let usda_bytes = usda_buf.len() as u64;
    let floor_plan_segments = scene
        .floor_plan
        .as_ref()
        .map(|fp| fp.segments.len() + fp.footprint_segments.len())
        .unwrap_or(0);

    let zip_file = File::create(out_path).map_err(|e| {
        FittingRoomError::UsdWrite(format!("create {}: {}", out_path.display(), e))
    })?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    zip.start_file(&manifest_name, opts)
        .map_err(|e| FittingRoomError::UsdWrite(format!("zip manifest entry: {}", e)))?;
    let manifest_bytes = std::fs::read(manifest_path).map_err(|e| {
        FittingRoomError::UsdWrite(format!("read manifest {}: {}", manifest_path.display(), e))
    })?;
    zip.write_all(&manifest_bytes)
        .map_err(|e| FittingRoomError::UsdWrite(format!("write manifest: {}", e)))?;

    zip.start_file(&usda_name, opts)
        .map_err(|e| FittingRoomError::UsdWrite(format!("zip usda entry: {}", e)))?;
    zip.write_all(&usda_buf)
        .map_err(|e| FittingRoomError::UsdWrite(format!("write usda: {}", e)))?;

    zip.finish()
        .map_err(|e| FittingRoomError::UsdWrite(format!("zip finalize: {}", e)))?;

    let zip_bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);

    Ok(ComposeReport {
        zip_path: out_path.to_path_buf(),
        usda_name,
        manifest_name,
        robot_count: scene.robots.len(),
        fixture_count: scene.fixtures.len(),
        floor_plan_segments,
        mesh_count: stats.mesh_count,
        total_vertices: stats.total_vertices,
        total_faces: stats.total_faces,
        usda_bytes,
        zip_bytes,
    })
}

#[derive(Default)]
struct MeshStats {
    mesh_count: usize,
    total_vertices: usize,
    total_faces: usize,
}

// =============================================================================
// URDF-source bundle export (additive — does NOT touch USDA composition above).
// Produces a SECOND, self-contained zip: the scene YAML (rewritten to reference
// bundled URDFs by relative path) + every referenced source URDF copied
// byte-for-byte (each in its own folder with its `meshes/` subtree) + the
// floor-plan OBJ when present. Extract it anywhere and `parse_scene` on the
// bundled YAML resolves every asset locally.
// =============================================================================

#[derive(Debug, Clone)]
pub struct UrdfBundleReport {
    pub zip_path: PathBuf,
    pub scene_yaml_name: String,
    pub robot_urdf_count: usize,
    pub fixture_urdf_count: usize,
    pub bundle_dir_count: usize,
    pub floor_plan_bundled: bool,
    pub file_count: usize,
    pub zip_bytes: u64,
}

/// A distinct URDF directory to copy into the bundle, plus the zip folder it
/// was assigned. Keyed in the caller by the canonicalised URDF directory so
/// two placements sharing one dressed URDF collapse to a single entry.
struct BundleEntry {
    /// Directory to copy from (the URDF's own parent dir), as originally resolved.
    src_dir: PathBuf,
    /// Absolute path to the `.urdf` file itself.
    urdf_file: PathBuf,
    /// Assigned bundle folder name (unique within its section).
    folder: String,
}

// FR-147, FR-148
/// Build the URDF-source bundle zip. `manifest_path` is read verbatim to
/// recover the authored YAML (incl. floor-plan path + fields the in-memory
/// `SceneDefinition` doesn't retain); only path fields are rewritten.
pub fn compose_urdf_bundle(
    scene: &SceneDefinition,
    manifest_path: &Path,
    out_path: &Path,
) -> Result<UrdfBundleReport, FittingRoomError> {
    let ext = out_path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext != "zip" {
        return Err(FittingRoomError::UnsupportedFormat(format!(
            ".{} (only .zip is supported)",
            ext
        )));
    }

    let manifest_dir = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    let manifest_stem = manifest_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.trim_start_matches('.').trim_start_matches("__compose_"))
        .map(|s| s.trim_start_matches('.'))
        .filter(|s| !s.is_empty())
        .unwrap_or("scene")
        .to_string();
    let scene_yaml_name = format!("{}.yaml", manifest_stem);

    // Collect + dedup URDF dirs, keyed by canonical dir. Robots under `urdf/`,
    // fixtures under `fixtures/`, so the two sections never collide.
    let robot_urdfs: Vec<&Path> = scene.robots.iter().map(|r| r.dressed_urdf.as_path()).collect();
    let fixture_urdfs: Vec<&Path> = scene.fixtures.iter().map(|f| f.urdf.as_path()).collect();
    let robot_entries = collect_bundle_entries(&robot_urdfs)?;
    let fixture_entries = collect_bundle_entries(&fixture_urdfs)?;

    // Reject any URDF whose meshes escape its own directory — such a ref can't
    // be made self-contained by a whole-dir copy. Read-only inspection.
    for entry in robot_entries.values().chain(fixture_entries.values()) {
        check_urdf_meshes_local(&entry.urdf_file)?;
    }

    // canonical URDF dir -> zip-relative folder (e.g. "robots/foo"), used to
    // rewrite the YAML path fields to match where files actually landed.
    let mut robot_dir_to_folder: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (canon, e) in &robot_entries {
        robot_dir_to_folder.insert(canon.clone(), format!("robots/{}", e.folder));
    }
    let mut fixture_dir_to_folder: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (canon, e) in &fixture_entries {
        fixture_dir_to_folder.insert(canon.clone(), format!("fixtures/{}", e.folder));
    }

    // Floor plan: recover its source OBJ path from the manifest YAML (the
    // in-memory FloorPlan struct doesn't keep it) and bundle it under
    // `floorplans/<stem>/<basename>` so it mirrors the robots/ + fixtures/
    // per-item-subfolder layout.
    let manifest_bytes = std::fs::read(manifest_path).map_err(|e| {
        FittingRoomError::UsdWrite(format!("read manifest {}: {}", manifest_path.display(), e))
    })?;
    let manifest_yaml = String::from_utf8_lossy(&manifest_bytes).into_owned();
    let floor_plan_src = floor_plan_geometry_file(&manifest_yaml);
    let floor_plan_target = floor_plan_src.as_ref().and_then(|rel| {
        let name = Path::new(rel).file_name()?.to_string_lossy().into_owned();
        let stem = Path::new(rel)
            .file_stem()
            .map(|s| sanitize_path_segment(&s.to_string_lossy()))
            .unwrap_or_else(|| "plan".to_string());
        Some(format!("floorplans/{}/{}", stem, name))
    });

    // Rewrite the scene YAML path fields (only). Everything else is preserved.
    let rewritten_yaml = rewrite_scene_yaml_paths(
        &manifest_yaml,
        &robot_dir_to_folder,
        &fixture_dir_to_folder,
        manifest_dir,
        floor_plan_target.as_deref(),
    )?;

    let zip_file = File::create(out_path).map_err(|e| {
        FittingRoomError::UsdWrite(format!("create {}: {}", out_path.display(), e))
    })?;
    let mut zip = zip::ZipWriter::new(zip_file);
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut file_count = 0usize;

    // 1. Scene YAML first.
    zip.start_file(&scene_yaml_name, opts)
        .map_err(|e| FittingRoomError::UsdWrite(format!("zip scene yaml entry: {}", e)))?;
    zip.write_all(rewritten_yaml.as_bytes())
        .map_err(|e| FittingRoomError::UsdWrite(format!("write scene yaml: {}", e)))?;
    file_count += 1;

    // 2. Robot URDF dirs (BTreeMap = sorted = deterministic), then fixtures.
    for e in robot_entries.values() {
        file_count += add_dir_to_zip(&mut zip, opts, &e.src_dir, &format!("robots/{}", e.folder))?;
    }
    for e in fixture_entries.values() {
        file_count +=
            add_dir_to_zip(&mut zip, opts, &e.src_dir, &format!("fixtures/{}", e.folder))?;
    }

    // 3. Floor-plan OBJ last.
    let mut floor_plan_bundled = false;
    if let (Some(src_rel), Some(target)) = (&floor_plan_src, &floor_plan_target) {
        let src = manifest_dir.join(src_rel);
        let bytes = std::fs::read(&src).map_err(|e| {
            FittingRoomError::FloorPlanNotFound(format!("{}: {}", src.display(), e))
        })?;
        zip.start_file(target.as_str(), opts)
            .map_err(|e| FittingRoomError::UsdWrite(format!("zip floor-plan entry: {}", e)))?;
        zip.write_all(&bytes)
            .map_err(|e| FittingRoomError::UsdWrite(format!("write floor-plan: {}", e)))?;
        file_count += 1;
        floor_plan_bundled = true;
    }

    zip.finish()
        .map_err(|e| FittingRoomError::UsdWrite(format!("zip finalize: {}", e)))?;
    let zip_bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);

    Ok(UrdfBundleReport {
        zip_path: out_path.to_path_buf(),
        scene_yaml_name,
        robot_urdf_count: robot_entries.len(),
        fixture_urdf_count: fixture_entries.len(),
        bundle_dir_count: robot_entries.len() + fixture_entries.len(),
        floor_plan_bundled,
        file_count,
        zip_bytes,
    })
}

/// Dedup URDF paths by canonical directory and assign a deterministic,
/// path-safe folder name per distinct directory. Collisions between different
/// source dirs get a numeric suffix in sorted-key order.
fn collect_bundle_entries(
    urdfs: &[&Path],
) -> Result<BTreeMap<PathBuf, BundleEntry>, FittingRoomError> {
    // canonical dir -> (src_dir, urdf_file)
    let mut dirs: BTreeMap<PathBuf, (PathBuf, PathBuf)> = BTreeMap::new();
    for urdf in urdfs {
        let src_dir = urdf
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let canon_dir = std::fs::canonicalize(&src_dir).unwrap_or_else(|_| src_dir.clone());
        dirs.entry(canon_dir)
            .or_insert_with(|| (src_dir.clone(), urdf.to_path_buf()));
    }

    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: BTreeMap<PathBuf, BundleEntry> = BTreeMap::new();
    for (canon, (src_dir, urdf_file)) in dirs {
        // Base name from the dir; fall back to the URDF file stem.
        let base = src_dir
            .file_name()
            .and_then(|n| n.to_str())
            .or_else(|| urdf_file.file_stem().and_then(|s| s.to_str()))
            .unwrap_or("urdf");
        let sanitized = sanitize_path_segment(base);
        let mut folder = sanitized.clone();
        let mut n = 2;
        while used.contains(&folder) {
            folder = format!("{}-{}", sanitized, n);
            n += 1;
        }
        used.insert(folder.clone());
        out.insert(
            canon,
            BundleEntry {
                src_dir,
                urdf_file,
                folder,
            },
        );
    }
    Ok(out)
}

/// Verify every `<mesh>` in the URDF resolves inside the URDF's own directory.
/// A mesh referenced by an absolute path or via `..` can't be captured by a
/// whole-dir copy, so the bundle wouldn't be self-contained — reject it.
fn check_urdf_meshes_local(urdf_file: &Path) -> Result<(), FittingRoomError> {
    let urdf_dir = urdf_file.parent().unwrap_or_else(|| Path::new("."));
    let canon_dir = std::fs::canonicalize(urdf_dir).unwrap_or_else(|_| urdf_dir.to_path_buf());
    let robot = parse_urdf(urdf_file)
        .map_err(|e| FittingRoomError::UrdfParse(format!("{}: {}", urdf_file.display(), e)))?;
    for link in &robot.links {
        let metas = link
            .visuals
            .iter()
            .map(|v| &v.geometry)
            .chain(link.collisions.iter().map(|c| &c.geometry));
        for geom in metas {
            if let Geometry::Mesh { filename, .. } = geom {
                let p = Path::new(filename);
                if p.is_absolute() {
                    return Err(FittingRoomError::Validation(format!(
                        "URDF {} references mesh by absolute path ({}); cannot build a \
                         self-contained bundle",
                        urdf_file.display(),
                        filename
                    )));
                }
                let joined = urdf_dir.join(filename);
                let canon = std::fs::canonicalize(&joined).unwrap_or(joined);
                if !canon.starts_with(&canon_dir) {
                    return Err(FittingRoomError::Validation(format!(
                        "URDF {} references mesh outside its directory ({}); cannot build a \
                         self-contained bundle",
                        urdf_file.display(),
                        filename
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Recursively copy every file under `src_root` into the zip beneath
/// `zip_prefix`, byte-for-byte. Directory entries are sorted so the output is
/// deterministic. Symlinks are skipped with a warning. Returns files written.
fn add_dir_to_zip<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    opts: SimpleFileOptions,
    src_root: &Path,
    zip_prefix: &str,
) -> Result<usize, FittingRoomError> {
    let mut count = 0usize;
    // Stack of (dir, zip-prefix-for-dir). Sort children per directory.
    let mut stack: Vec<(PathBuf, String)> = vec![(src_root.to_path_buf(), zip_prefix.to_string())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut children: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir)
            .map_err(|e| {
                FittingRoomError::UsdWrite(format!("read dir {}: {}", dir.display(), e))
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| {
                FittingRoomError::UsdWrite(format!("read dir {}: {}", dir.display(), e))
            })?;
        children.sort_by_key(|e| e.file_name());
        // Process in sorted order. Push subdirs so they pop in order too; files
        // are written immediately.
        let mut subdirs: Vec<(PathBuf, String)> = Vec::new();
        for child in children {
            let name = child.file_name();
            let name = name.to_string_lossy();
            let meta = child.metadata().map_err(|e| {
                FittingRoomError::UsdWrite(format!("stat {}: {}", child.path().display(), e))
            })?;
            if meta.file_type().is_symlink() {
                log::warn!("skipping symlink in bundle: {}", child.path().display());
                continue;
            }
            let entry_zip_path = format!("{}/{}", prefix, name);
            if meta.is_dir() {
                subdirs.push((child.path(), entry_zip_path));
            } else {
                let bytes = std::fs::read(child.path()).map_err(|e| {
                    FittingRoomError::UsdWrite(format!(
                        "read {}: {}",
                        child.path().display(),
                        e
                    ))
                })?;
                zip.start_file(entry_zip_path.as_str(), opts).map_err(|e| {
                    FittingRoomError::UsdWrite(format!("zip entry {}: {}", entry_zip_path, e))
                })?;
                zip.write_all(&bytes).map_err(|e| {
                    FittingRoomError::UsdWrite(format!("write {}: {}", entry_zip_path, e))
                })?;
                count += 1;
            }
        }
        // Reverse so popping the stack visits subdirs in sorted order.
        for sd in subdirs.into_iter().rev() {
            stack.push(sd);
        }
    }
    Ok(count)
}

/// Extract `floor_plan.geometry_file` (a relative path string) from the raw
/// manifest YAML, if present. Uses a loose `serde_yaml::Value` walk so the rest
/// of the manifest shape is irrelevant.
fn floor_plan_geometry_file(manifest_yaml: &str) -> Option<String> {
    let doc: serde_yaml::Value = serde_yaml::from_str(manifest_yaml).ok()?;
    doc.get("floor_plan")?
        .get("geometry_file")?
        .as_str()
        .map(|s| s.to_string())
}

/// Re-serialise the manifest with only the URDF/fixture/floor-plan path fields
/// pointed at their bundled locations. Parsing to `serde_yaml::Value` (not the
/// Deserialize-only scene structs) preserves scene_origin, home_pose_deg,
/// pedestal, offsets, description, and any other keys verbatim.
fn rewrite_scene_yaml_paths(
    manifest_yaml: &str,
    robot_dir_to_folder: &BTreeMap<PathBuf, String>,
    fixture_dir_to_folder: &BTreeMap<PathBuf, String>,
    manifest_dir: &Path,
    floor_plan_target: Option<&str>,
) -> Result<String, FittingRoomError> {
    use serde_yaml::Value;

    let mut doc: Value = serde_yaml::from_str(manifest_yaml)
        .map_err(|e| FittingRoomError::SceneParse(format!("yaml parse: {}", e)))?;

    // Map an authored URDF path (relative to the manifest dir) to its bundled
    // location by resolving to the canonical directory and looking it up.
    let bundled_path = |raw: &str, map: &BTreeMap<PathBuf, String>| -> Option<String> {
        let joined = manifest_dir.join(raw);
        let dir = joined.parent()?.to_path_buf();
        let canon_dir = std::fs::canonicalize(&dir).unwrap_or(dir);
        let folder = map.get(&canon_dir)?;
        let file = Path::new(raw).file_name()?.to_string_lossy().into_owned();
        Some(format!("{}/{}", folder, file))
    };

    if let Some(robots) = doc.get_mut("robots").and_then(Value::as_sequence_mut) {
        for r in robots {
            if let Some(raw) = r.get("dressed_urdf").and_then(Value::as_str) {
                if let Some(new_path) = bundled_path(raw, robot_dir_to_folder) {
                    r["dressed_urdf"] = Value::String(new_path);
                }
            }
        }
    }
    if let Some(fixtures) = doc.get_mut("fixtures").and_then(Value::as_sequence_mut) {
        for f in fixtures {
            if let Some(raw) = f.get("urdf").and_then(Value::as_str) {
                if let Some(new_path) = bundled_path(raw, fixture_dir_to_folder) {
                    f["urdf"] = Value::String(new_path);
                }
            }
        }
    }
    if let Some(target) = floor_plan_target {
        if let Some(fp) = doc.get_mut("floor_plan") {
            if fp.get("geometry_file").is_some() {
                fp["geometry_file"] = Value::String(target.to_string());
            }
        }
    }

    serde_yaml::to_string(&doc)
        .map_err(|e| FittingRoomError::UsdWrite(format!("yaml serialize: {}", e)))
}

/// Sanitise a string into a single, filesystem/zip-safe path segment:
/// ASCII alphanumerics, `_`, `-` kept; everything else becomes `_`. Never
/// empty, never `.`/`..`.
fn sanitize_path_segment(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() || out == "." || out == ".." {
        out = "urdf".to_string();
    }
    out
}

/// PhysicsScene tuned for a multi-robot scene. Schema set and capacities mirror
/// the user-supplied base USDA so a reload in Isaac picks up identical solver
/// behaviour — solver iteration counts, GPU broadphase buffers, and the
/// `PhysxSchemaPhysxSceneAPI` schema needed for the physxScene:* attributes.
fn emit_physics_scene(w: &mut impl Write) -> Result<(), FittingRoomError> {
    writeln!(w, "    def PhysicsScene \"PhysicsScene\" (")?;
    writeln!(
        w,
        "        prepend apiSchemas = [\"PhysxSchemaPhysxSceneAPI\", \"PhysxSceneAPI\"]"
    )?;
    writeln!(w, "    )")?;
    writeln!(w, "    {{")?;
    writeln!(w, "        vector3f basic:gravityDirection = (0, 0, -1)")?;
    writeln!(w, "        float basic:gravityMagnitude = 9.81")?;
    writeln!(w, "        bool physxScene:enableGPUDynamics = 1")?;
    writeln!(w, "        bool physxScene:enableGpuSdfApproximation = 1")?;
    // Per recommendation Fix 1: scale up GPU buffer limits for dense
    // multi-robot scene graphs. gpuFoundLostAggregatePairsCapacity was
    // missing entirely from the previous emission.
    writeln!(w, "        int physxScene:gpuFoundLostPairsCapacity = 8192")?;
    writeln!(
        w,
        "        int physxScene:gpuFoundLostAggregatePairsCapacity = 8192"
    )?;
    writeln!(w, "        int physxScene:gpuTotalAggregatePairsCapacity = 4096")?;
    writeln!(w, "        int physxScene:gpuMaxRigidContactCount = 1048576")?;
    writeln!(w, "        int physxScene:gpuMaxRigidPatchCount = 16384")?;
    writeln!(w, "        uniform uint physxScene:maxPositionIterationCount = 16")?;
    writeln!(w, "        uniform uint physxScene:maxVelocityIterationCount = 4")?;
    writeln!(w, "        int physxScene:positionIterations = 16")?;
    writeln!(w, "        int physxScene:velocityIterations = 4")?;
    // Step rate: 120 Hz halves dt to ~8.3 ms, comfortably within the
    // stability envelope of the (k=1.5e5, m≈30..250 kg) drives used here.
    writeln!(w, "        float physxScene:timeStepsPerSecond = 120")?;
    writeln!(w, "    }}")?;
    writeln!(w)?;
    Ok(())
}


fn write_header(
    w: &mut impl Write,
    scene_name: &str,
    usda_name: &str,
) -> Result<(), FittingRoomError> {
    writeln!(w, "#usda 1.0")?;
    writeln!(w, "(")?;
    writeln!(w, "    customLayerData = {{")?;
    writeln!(w, "        dictionary cameraSettings = {{")?;
    writeln!(w, "            dictionary Front = {{")?;
    writeln!(w, "                double3 position = (5, 0, 0)")?;
    writeln!(w, "                double radius = 5")?;
    writeln!(w, "            }}")?;
    writeln!(w, "            dictionary Perspective = {{")?;
    writeln!(w, "                double3 position = (5, 5, 5)")?;
    writeln!(
        w,
        "                double3 target = (-3.978038609631085e-8, -3.978038609631085e-8, 7.956076952808644e-8)"
    )?;
    writeln!(w, "            }}")?;
    writeln!(w, "            dictionary Right = {{")?;
    writeln!(w, "                double3 position = (0, -5, 0)")?;
    writeln!(w, "                double radius = 5")?;
    writeln!(w, "            }}")?;
    writeln!(w, "            dictionary Top = {{")?;
    writeln!(w, "                double3 position = (0, 0, 5)")?;
    writeln!(w, "                double radius = 5")?;
    writeln!(w, "            }}")?;
    writeln!(w, "            string boundCamera = \"/OmniverseKit_Persp\"")?;
    writeln!(w, "        }}")?;
    writeln!(w, "        dictionary omni_layer = {{")?;
    writeln!(
        w,
        "            string authoring_layer = \"./{}\"",
        escape_str(usda_name)
    )?;
    writeln!(w, "            dictionary locked = {{")?;
    writeln!(w, "            }}")?;
    writeln!(w, "            dictionary muteness = {{")?;
    writeln!(w, "            }}")?;
    writeln!(w, "        }}")?;
    writeln!(w, "        dictionary renderSettings = {{")?;
    writeln!(w, "            bool \"rtx:post:aa:op:enabled\" = 0")?;
    writeln!(w, "            int \"app:renderFrameTimeout\" = 120")?;
    writeln!(w, "            bool \"rtx:directLighting:enabled\" = 1")?;
    writeln!(w, "            bool \"rtx:indirectLighting:enabled\" = 0")?;
    writeln!(w, "            bool \"rtx:ambientOcclusion:enabled\" = 0")?;
    writeln!(w, "            bool \"rtx:reflections:enabled\" = 0")?;
    writeln!(w, "            bool \"rtx:translucency:enabled\" = 0")?;
    writeln!(w, "            int \"rtx:pathTracing:maxBounces\" = 1")?;
    writeln!(w, "            int \"rtx:raytracing:maxBounces\" = 1")?;
    writeln!(w, "            bool \"rtx:pathtracing:denoiser:enabled\" = 0")?;
    writeln!(w, "            bool \"rtx:realtime:denoiser:enabled\" = 0")?;
    writeln!(w, "        }}")?;
    writeln!(w, "    }}")?;
    writeln!(w, "    defaultPrim = \"World\"")?;
    writeln!(w, "    endTimeCode = 1000000")?;
    writeln!(w, "    metersPerUnit = 1")?;
    writeln!(w, "    startTimeCode = 0")?;
    writeln!(w, "    timeCodesPerSecond = 60")?;
    writeln!(w, "    upAxis = \"Z\"")?;
    writeln!(w, "    doc = \"{}\"", escape_str(scene_name))?;
    writeln!(w, ")")?;
    writeln!(w)?;
    Ok(())
}

fn emit_environment(w: &mut impl Write) -> Result<(), FittingRoomError> {
    writeln!(w, "def Xform \"Environment\"")?;
    writeln!(w, "{{")?;
    writeln!(w, "    quatd xformOp:orient = (1, 0, 0, 0)")?;
    writeln!(w, "    double3 xformOp:scale = (1, 1, 1)")?;
    writeln!(w, "    double3 xformOp:translate = (0, 0, 0)")?;
    writeln!(
        w,
        "    uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\", \"xformOp:scale\"]"
    )?;
    writeln!(w)?;
    writeln!(w, "    def DistantLight \"defaultLight\" (")?;
    writeln!(w, "        prepend apiSchemas = [\"ShapingAPI\"]")?;
    writeln!(w, "    )")?;
    writeln!(w, "    {{")?;
    writeln!(w, "        float inputs:angle = 1")?;
    writeln!(w, "        float inputs:intensity = 3000")?;
    writeln!(w, "        float inputs:shaping:cone:angle = 180")?;
    writeln!(w, "        float inputs:shaping:cone:softness")?;
    writeln!(w, "        float inputs:shaping:focus")?;
    writeln!(w, "        color3f inputs:shaping:focusTint")?;
    writeln!(w, "        asset inputs:shaping:ies:file")?;
    writeln!(
        w,
        "        quatd xformOp:orient = (0.6532814824381883, 0.27059805007309856, 0.27059805007309856, 0.6532814824381883)"
    )?;
    writeln!(w, "        double3 xformOp:scale = (1, 1, 1)")?;
    writeln!(w, "        double3 xformOp:translate = (0, 0, 0)")?;
    writeln!(
        w,
        "        uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\", \"xformOp:scale\"]"
    )?;
    writeln!(w, "    }}")?;
    writeln!(w, "}}")?;
    writeln!(w)?;
    Ok(())
}

fn emit_render_block(w: &mut impl Write) -> Result<(), FittingRoomError> {
    writeln!(w, "def \"Render\" (")?;
    writeln!(w, "    hide_in_stage_window = true")?;
    writeln!(w, "    no_delete = true")?;
    writeln!(w, ")")?;
    writeln!(w, "{{")?;
    writeln!(w, "    def \"OmniverseKit\"")?;
    writeln!(w, "    {{")?;
    writeln!(w, "        def \"HydraTextures\" (")?;
    writeln!(w, "            hide_in_stage_window = true")?;
    writeln!(w, "            no_delete = true")?;
    writeln!(w, "        )")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            def RenderProduct \"omni_kit_widget_viewport_ViewportTexture_0\" ("
    )?;
    writeln!(
        w,
        "                prepend apiSchemas = [\"OmniRtxSettingsCommonAdvancedAPI_1\", \"OmniRtxSettingsRtAdvancedAPI_1\", \"OmniRtxSettingsPtAdvancedAPI_1\", \"OmniRtxPostColorGradingAPI_1\", \"OmniRtxPostChromaticAberrationAPI_1\", \"OmniRtxPostBloomPhysicalAPI_1\", \"OmniRtxPostMatteObjectAPI_1\", \"OmniRtxPostCompositingAPI_1\", \"OmniRtxPostDofAPI_1\", \"OmniRtxPostMotionBlurAPI_1\", \"OmniRtxPostTvNoiseAPI_1\", \"OmniRtxPostTonemapIrayReinhardAPI_1\", \"OmniRtxPostDebugSettingsAPI_1\", \"OmniRtxDebugSettingsAPI_1\"]"
    )?;
    writeln!(w, "                hide_in_stage_window = true")?;
    writeln!(w, "                no_delete = true")?;
    writeln!(w, "            )")?;
    writeln!(w, "            {{")?;
    writeln!(w, "                rel camera = </OmniverseKit_Persp>")?;
    writeln!(
        w,
        "                token omni:rtx:background:source:texture:textureMode = \"repeatMirrored\""
    )?;
    writeln!(
        w,
        "                token omni:rtx:background:source:type = \"domeLight\""
    )?;
    writeln!(w, "                bool omni:rtx:dlss:frameGeneration = 0")?;
    writeln!(
        w,
        "                bool omni:rtx:material:db:nonVisualMaterialCSV:enabled = 0"
    )?;
    writeln!(w, "                bool omni:rtx:material:db:syncLoads = 1")?;
    writeln!(
        w,
        "                token omni:rtx:post:dlss:execMode = \"performance\""
    )?;
    writeln!(
        w,
        "                bool omni:rtx:post:registeredCompositing:invertColorCorrection = 1"
    )?;
    writeln!(
        w,
        "                bool omni:rtx:post:registeredCompositing:invertToneMap = 1"
    )?;
    writeln!(
        w,
        "                bool omni:rtx:pt:lightcache:cached:dontResolveConflicts = 1"
    )?;
    writeln!(w, "                int omni:rtx:pt:maxSamplesPerLaunch = 2073600")?;
    writeln!(
        w,
        "                int omni:rtx:pt:mgpu:maxPixelsPerRegionExponent = 12"
    )?;
    writeln!(w, "                token omni:rtx:rendermode = \"PathTracing\"")?;
    writeln!(
        w,
        "                color3f omni:rtx:rt:ambientLight:color = (0.1, 0.1, 0.1)"
    )?;
    writeln!(w, "                bool omni:rtx:rt:demoire = 0")?;
    writeln!(
        w,
        "                bool omni:rtx:rt:lightcache:spatialCache:dontResolveConflicts = 1"
    )?;
    writeln!(
        w,
        "                bool omni:rtx:scene:hydra:materialSyncLoads = 1"
    )?;
    writeln!(w, "                bool omni:rtx:scene:hydra:mdlMaterialWarmup = 1")?;
    writeln!(w, "                uint omni:rtx:viewTile:limit = 4294967295")?;
    writeln!(w, "                rel orderedVars = </Render/Vars/LdrColor>")?;
    writeln!(w, "                custom bool overrideClipRange = 0")?;
    writeln!(w, "                uniform int2 resolution = (1280, 720)")?;
    writeln!(w, "            }}")?;
    writeln!(w, "        }}")?;
    writeln!(w, "    }}")?;
    writeln!(w)?;
    writeln!(
        w,
        "    def RenderSettings \"OmniverseGlobalRenderSettings\" ("
    )?;
    writeln!(
        w,
        "        prepend apiSchemas = [\"OmniRtxSettingsGlobalRtAdvancedAPI_1\", \"OmniRtxSettingsGlobalPtAdvancedAPI_1\"]"
    )?;
    writeln!(w, "        no_delete = true")?;
    writeln!(w, "    )")?;
    writeln!(w, "    {{")?;
    writeln!(
        w,
        "        rel products = </Render/OmniverseKit/HydraTextures/omni_kit_widget_viewport_ViewportTexture_0>"
    )?;
    writeln!(w, "    }}")?;
    writeln!(w)?;
    writeln!(w, "    def \"Vars\"")?;
    writeln!(w, "    {{")?;
    writeln!(w, "        def RenderVar \"LdrColor\" (")?;
    writeln!(w, "            hide_in_stage_window = true")?;
    writeln!(w, "            no_delete = true")?;
    writeln!(w, "        )")?;
    writeln!(w, "        {{")?;
    writeln!(w, "            uniform string sourceName = \"LdrColor\"")?;
    writeln!(w, "        }}")?;
    writeln!(w, "    }}")?;
    writeln!(w, "}}")?;
    writeln!(w)?;
    Ok(())
}

fn emit_flat_grid(w: &mut impl Write) -> Result<(), FittingRoomError> {
    writeln!(w, "def \"FlatGrid\" (")?;
    writeln!(
        w,
        "    prepend references = @https://omniverse-content-production.s3-us-west-2.amazonaws.com/Assets/Isaac/5.1/Isaac/Environments/Grid/default_environment.usd@"
    )?;
    writeln!(w, ")")?;
    writeln!(w, "{{")?;
    writeln!(w, "}}")?;
    writeln!(w)?;
    Ok(())
}

// FR-110, FR-111, FR-112, FR-113, FR-114, FR-115
fn emit_robot(
    w: &mut impl Write,
    placement: &RobotPlacement,
    cache: &mut MeshCache,
    materials: &mut MaterialRegistry,
    stats: &mut MeshStats,
    full_visuals: bool,
    full_tools: bool,
) -> Result<(), FittingRoomError> {
    let robot = parse_urdf(&placement.dressed_urdf).map_err(|e| {
        FittingRoomError::UrdfParse(format!("{}: {}", placement.dressed_urdf.display(), e))
    })?;
    let urdf_dir = placement
        .dressed_urdf
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let urdf_stem = placement
        .dressed_urdf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    let robot_id = sanitize_prim_name(&placement.robot_id);
    let quat = abc_deg_to_quat(placement.orientation_deg);

    // FR-119: when a pedestal is configured, raise the robot foot by its
    // height (mm → m) so the base sits on top of the mounting cylinder. The
    // raised z must be applied to BOTH the Xform translate and the
    // world_anchor localPos0 below, or PhysX snaps the joint back together.
    let raise_m = placement
        .pedestal
        .map(|p| p.height_mm / 1000.0)
        .filter(|h| *h > 0.0)
        .unwrap_or(0.0);
    let raised = [
        placement.position[0],
        placement.position[1],
        placement.position[2] + raise_m,
    ];

    // Apply both PhysicsArticulationRootAPI and PhysxArticulationAPI:
    // - PhysicsArticulationRootAPI marks the prim as the start of an articulation tree
    // - PhysxArticulationAPI exposes PhysX-specific solver attributes and is what
    //   Isaac Sim's articulation tools (joint sliders, articulation view) key off
    // Both together = full recognition by Isaac Sim's articulation panel.
    writeln!(w, "    def Xform \"{}\" (", robot_id)?;
    writeln!(
        w,
        "        prepend apiSchemas = [\"PhysicsArticulationRootAPI\", \"PhysxArticulationAPI\"]"
    )?;
    writeln!(w, "    )")?;
    writeln!(w, "    {{")?;
    writeln!(w, "        uniform bool physxArticulation:enabledSelfCollisions = 0")?;
    // Per recommendation Fix 3: position iterations 64 for heavy KR240
    // payload arms; velocity iterations at the recommended baseline of 4.
    writeln!(w, "        uniform int physxArticulation:solverPositionIterationCount = 64")?;
    writeln!(w, "        uniform int physxArticulation:solverVelocityIterationCount = 4")?;
    writeln!(w, "        uniform float physxArticulation:sleepThreshold = 0.005")?;
    writeln!(w, "        uniform float physxArticulation:stabilizationThreshold = 0.001")?;
    writeln!(
        w,
        "        double3 xformOp:translate = ({}, {}, {})",
        f(raised[0]),
        f(raised[1]),
        f(raised[2])
    )?;
    writeln!(
        w,
        "        quatd xformOp:orient = ({}, {}, {}, {})",
        f(quat[3]),
        f(quat[0]),
        f(quat[1]),
        f(quat[2])
    )?;
    writeln!(
        w,
        "        uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]"
    )?;
    writeln!(w)?;

    let home_override = placement.home_pose_deg.as_deref();
    let root_link_name = find_root_link(&robot).map(|s| s.to_string());
    let bind_poses = compute_link_bind_poses(&robot, home_override);
    for link in &robot.links {
        let is_root = root_link_name.as_deref() == Some(link.name.as_str());
        let pose = bind_poses
            .get(&link.name)
            .copied()
            .unwrap_or(math3::IDENTITY);
        emit_link(
            w,
            link,
            &urdf_dir,
            &urdf_stem,
            cache,
            materials,
            stats,
            8,
            is_root,
            &pose,
            full_visuals,
            full_tools,
        )?;
    }

    let mut next_actuated_idx: usize = 0;
    for joint in &robot.joints {
        let idx = if matches!(joint.joint_type, JointType::Fixed) {
            None
        } else {
            let i = next_actuated_idx;
            next_actuated_idx += 1;
            Some(i)
        };
        emit_joint(w, joint, &robot_id, 8, idx, home_override)?;
    }

    if let Some(root_link) = find_root_link(&robot) {
        let safe_root = sanitize_prim_name(root_link);
        // FixedJoint anchoring root link to world inertial frame.
        // - body0 = empty rel (the world inertial frame)
        // - body1 = root link (positioned via parent Xform's xformOp)
        // - localPos0/localRot0 must match the body1's current world pose so
        //   PhysX doesn't try to snap them together.
        writeln!(w, "        def PhysicsFixedJoint \"world_anchor\"")?;
        writeln!(w, "        {{")?;
        writeln!(w, "            rel physics:body0")?;
        writeln!(
            w,
            "            rel physics:body1 = </World/robots/{}/{}>",
            robot_id, safe_root
        )?;
        writeln!(
            w,
            "            point3f physics:localPos0 = ({}, {}, {})",
            f(raised[0]),
            f(raised[1]),
            f(raised[2])
        )?;
        writeln!(
            w,
            "            quatf physics:localRot0 = ({}, {}, {}, {})",
            f(quat[3]),
            f(quat[0]),
            f(quat[1]),
            f(quat[2])
        )?;
        writeln!(w, "            point3f physics:localPos1 = (0, 0, 0)")?;
        writeln!(w, "            quatf physics:localRot1 = (1, 0, 0, 0)")?;
        writeln!(w, "        }}")?;
    }

    writeln!(w, "    }}")?;
    writeln!(w)?;
    Ok(())
}

// FR-102
fn emit_fixture(
    w: &mut impl Write,
    placement: &FixturePlacement,
    cache: &mut MeshCache,
    materials: &mut MaterialRegistry,
    stats: &mut MeshStats,
) -> Result<(), FittingRoomError> {
    let robot = parse_urdf(&placement.urdf)
        .map_err(|e| FittingRoomError::UrdfParse(format!("{}: {}", placement.urdf.display(), e)))?;

    for joint in &robot.joints {
        if joint.joint_type != JointType::Fixed {
            return Err(FittingRoomError::Validation(format!(
                "Fixture '{}' has non-fixed joint '{}' (type: {}); fixtures must use fixed joints only",
                placement.id,
                joint.name,
                joint.joint_type.as_str()
            )));
        }
    }

    let urdf_dir = placement
        .urdf
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let urdf_stem = placement
        .urdf
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();
    let fixture_id = sanitize_prim_name(&placement.id);
    let quat = abc_deg_to_quat(placement.orientation_deg);

    writeln!(w, "        def Xform \"{}\"", fixture_id)?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            double3 xformOp:translate = ({}, {}, {})",
        f(placement.position[0]),
        f(placement.position[1]),
        f(placement.position[2])
    )?;
    writeln!(
        w,
        "            quatd xformOp:orient = ({}, {}, {}, {})",
        f(quat[3]),
        f(quat[0]),
        f(quat[1]),
        f(quat[2])
    )?;
    writeln!(
        w,
        "            uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]"
    )?;
    writeln!(w)?;

    // Fixtures have no actuated joints; the home-pose override is meaningless.
    let root_link_name = find_root_link(&robot).map(|s| s.to_string());
    let bind_poses = compute_link_bind_poses(&robot, None);
    for link in &robot.links {
        let is_root = root_link_name.as_deref() == Some(link.name.as_str());
        let pose = bind_poses
            .get(&link.name)
            .copied()
            .unwrap_or(math3::IDENTITY);
        // Fixtures always emit at full visual fidelity — the lean robot
        // and tool toggles don't apply to fixtures.
        emit_link(
            w,
            link,
            &urdf_dir,
            &urdf_stem,
            cache,
            materials,
            stats,
            12,
            is_root,
            &pose,
            /* full_visuals = */ true,
            /* full_tools   = */ true,
        )?;
    }

    if let Some(root_link) = find_root_link(&robot) {
        let safe_root = sanitize_prim_name(root_link);
        // FixedJoint pinning the fixture root to the world inertial frame.
        // Frame on body0 (world) must equal the body1 prim's world pose,
        // otherwise PhysX reports "disjointed body transforms" and snaps
        // both ends together at the origin.
        writeln!(w, "            def PhysicsFixedJoint \"world_anchor\"")?;
        writeln!(w, "            {{")?;
        writeln!(w, "                rel physics:body0")?;
        writeln!(
            w,
            "                rel physics:body1 = </World/fixtures/{}/{}>",
            fixture_id, safe_root
        )?;
        writeln!(
            w,
            "                point3f physics:localPos0 = ({}, {}, {})",
            f(placement.position[0]),
            f(placement.position[1]),
            f(placement.position[2])
        )?;
        writeln!(
            w,
            "                quatf physics:localRot0 = ({}, {}, {}, {})",
            f(quat[3]),
            f(quat[0]),
            f(quat[1]),
            f(quat[2])
        )?;
        writeln!(w, "                point3f physics:localPos1 = (0, 0, 0)")?;
        writeln!(w, "                quatf physics:localRot1 = (1, 0, 0, 0)")?;
        writeln!(w, "            }}")?;
    }

    writeln!(w, "        }}")?;
    writeln!(w)?;
    Ok(())
}

// FR-119
fn emit_pedestal(w: &mut impl Write, placement: &RobotPlacement) -> Result<(), FittingRoomError> {
    // Only called for robots with a positive-height pedestal.
    let ped = placement.pedestal.expect("pedestal present");
    let height_m = ped.height_mm / 1000.0;
    let radius_m = (ped.diameter_mm / 1000.0) / 2.0;
    let half_h = height_m / 2.0;
    let name = format!("pedestal_{}", sanitize_prim_name(&placement.robot_id));

    // A USD Cylinder is centred on its origin along the axis, so translating
    // the wrapping Xform to z = height/2 makes the solid span world z∈[0, h]
    // at the robot's XY, top face flush under the raised foot. Orientation is
    // identity — a vertical cylinder is rotationally symmetric about Z.
    writeln!(w, "        def Xform \"{}\"", name)?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            double3 xformOp:translate = ({}, {}, {})",
        f(placement.position[0]),
        f(placement.position[1]),
        f(half_h)
    )?;
    writeln!(w, "            quatd xformOp:orient = (1, 0, 0, 0)")?;
    writeln!(
        w,
        "            uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]"
    )?;
    writeln!(w)?;
    // Static collider: PhysicsCollisionAPI without a rigid-body API is
    // immovable in PhysX — exactly what a mounting fundament needs.
    writeln!(w, "            def Cylinder \"socket\" (")?;
    writeln!(
        w,
        "                prepend apiSchemas = [\"PhysicsCollisionAPI\"]"
    )?;
    writeln!(w, "            )")?;
    writeln!(w, "            {{")?;
    writeln!(w, "                uniform token axis = \"Z\"")?;
    writeln!(w, "                double height = {}", f(height_m))?;
    writeln!(w, "                double radius = {}", f(radius_m))?;
    writeln!(
        w,
        "                float3[] extent = [({}, {}, {}), ({}, {}, {})]",
        f(-radius_m),
        f(-radius_m),
        f(-half_h),
        f(radius_m),
        f(radius_m),
        f(half_h)
    )?;
    writeln!(w, "            }}")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;
    Ok(())
}

fn emit_link(
    w: &mut impl Write,
    link: &Link,
    urdf_dir: &Path,
    urdf_stem: &str,
    cache: &mut MeshCache,
    materials: &mut MaterialRegistry,
    stats: &mut MeshStats,
    indent: usize,
    is_root: bool,
    bind_pose: &Mat4,
    full_visuals: bool,
    full_tools: bool,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    let safe_name = sanitize_prim_name(&link.name);

    // Every articulated link — including the URDF root — must carry
    // PhysicsRigidBodyAPI; PhysX articulation walks bodies-by-link and treats
    // a link without RigidBodyAPI as broken. Fixed-base behaviour is
    // achieved by the world_anchor FixedJoint, not by omitting the API.
    // PhysicsMassAPI is only applied when we have mass data — either from
    // the URDF's inertial block, or from the steel-arm override table.
    // Every PhysicsRigidBodyAPI prim MUST also carry PhysicsMassAPI with
    // positive mass; otherwise PhysX warns "negative mass / small sphere
    // approximation" every frame and propagates an invalid inertia tensor
    // into the articulation solver. We always apply both schemas — the
    // emission below picks the best mass/inertia source available.
    //
    // FR-160 / FR-161 / FR-162: when the link has at least one
    // <collision> we ALSO apply PhysxContactReportAPI so PhysX queues
    // contact events for this body. Without it the IsaacReadContactSensor
    // reads empty arrays every frame, its `execOut` never fires, and the
    // ROS2PublishContactSensor downstream never advertises a topic — the
    // bridge silently no-ops. Threshold = 0.0 so every contact reports;
    // any FR-168-style force filtering happens in the cell-aggregator pod
    // (BL-24), not here.
    let has_collision = !link.collisions.is_empty();
    let mut apis: Vec<&str> = vec!["PhysicsRigidBodyAPI", "PhysicsMassAPI"];
    if has_collision {
        apis.push("PhysxContactReportAPI");
    }
    let api_list = apis
        .iter()
        .map(|a| format!("\"{}\"", a))
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(w, "{}def Xform \"{}\" (", pad, safe_name)?;
    writeln!(w, "{}    prepend apiSchemas = [{}]", pad, api_list)?;
    writeln!(w, "{})", pad)?;
    let _ = is_root;
    writeln!(w, "{}{{", pad)?;

    // Bind-pose as a single matrix4d xformOp:transform — more reliable than
    // split translate+orient when the prim is part of a PhysX articulation.
    emit_xform_transform(w, bind_pose, indent + 4)?;

    // PhysxContactReportAPI threshold attribute. 0.0 = report every contact;
    // anything positive means "only contacts with impulse magnitude above
    // threshold queue events". Keep at 0.0 — filtering is the aggregator's
    // job (FR-168 / BL-24).
    if has_collision {
        writeln!(
            w,
            "{}    float physxContactReport:threshold = 0",
            pad
        )?;
    }

    // FR-113. Mass + inertia priority (in order):
    //   1. KR240 steel-arm override (matches link name suffix)
    //   2. URDF inertial verbatim
    //   3. Generic positive-mass fallback for orphan rigid bodies
    //      (URDF root / fixture root with no <inertial>). Set to 10 kg
    //      per the Isaac Sim guidance for KR240-class arm bases — small
    //      enough to not skew articulation dynamics, large enough that
    //      PhysX never falls back to "negative mass small sphere" mode.
    if let Some((mass, inertia)) = steel_arm_inertia(&link.name) {
        emit_mass_api_override(w, link.inertial.as_ref(), mass, inertia, indent + 4)?;
    } else if let Some(inertial) = &link.inertial {
        emit_mass_api(w, inertial, indent + 4)?;
    } else {
        emit_mass_api_override(w, None, 10.0, [1.0, 1.0, 1.0], indent + 4)?;
    }

    // FR-111. Per-visual decision: tool meshes (path prefix
    // `meshes/tool/`) are gated by `full_tools`; everything else is gated
    // by `full_visuals`. Joints reference link Xform prims (not the mesh
    // children), so dropping any subset of visuals never breaks the
    // articulation kinematic chain.
    for (i, visual) in link.visuals.iter().enumerate() {
        let keep = if is_tool_visual(visual) {
            full_tools
        } else {
            full_visuals
        };
        if keep {
            emit_visual_mesh(
                w, visual, i, urdf_dir, urdf_stem, cache, materials, stats, indent + 4,
            )?;
        }
    }

    // FR-112. Collision hulls are always emitted (they're the physics
    // geometry); their viewport visibility tracks whether the matching
    // visual is present. Per-collision: if it's a tool collision and
    // `full_tools` is OFF, surface it (purpose=default). Otherwise apply
    // the link-wide rule (visible iff no visuals were emitted on this link).
    //
    // We can't perfectly pair a collision with "its" visual without a
    // matching mechanism URDF doesn't provide, so use the same path-prefix
    // heuristic on the collision filename.
    for (i, collision) in link.collisions.iter().enumerate() {
        let visible = if is_tool_collision(collision) {
            !full_tools
        } else {
            !full_visuals
        };
        emit_collision_mesh(
            w,
            collision,
            i,
            urdf_dir,
            cache,
            stats,
            indent + 4,
            visible,
        )?;
    }

    // FR-160 / FR-161 / FR-162. When the link has any collision geometry,
    // emit an `IsaacContactSensor` child prim. The OmniGraph
    // `IsaacReadContactSensor` node's `inputs:csPrim` rel must point at
    // THIS sensor prim (not the link Xform); without it Isaac warns
    // "no valid sensor reading, is the sensor enabled?" and the bridge
    // never publishes. Sensor name is fixed (`ContactSensor_node`) so
    // the OmniGraph emission can build a deterministic path.
    if has_collision {
        emit_link_contact_sensor_prim(w, indent + 4)?;
    }

    writeln!(w, "{}}}", pad)?;
    writeln!(w)?;
    Ok(())
}

/// FR-160. Emit a per-link `IsaacContactSensor` child prim. Lives at
/// `<link>/ContactSensor_node` so the OmniGraph `csPrim` rel resolves
/// deterministically. `radius = -1` means the sensor reports any contact
/// on the parent body (no spatial filter); `enabled = 1` is required —
/// Isaac defaults to disabled and silently drops events otherwise.
fn emit_link_contact_sensor_prim(
    w: &mut impl Write,
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    writeln!(w, "{}def IsaacContactSensor \"ContactSensor_node\"", pad)?;
    writeln!(w, "{}{{", pad)?;
    writeln!(w, "{}    bool enabled = 1", pad)?;
    // -1 = "use parent rigid body's full extent", any contact reports.
    writeln!(w, "{}    float radius = -1", pad)?;
    // 0 = no minimum threshold; filtering is the aggregator's job (BL-24, FR-168).
    writeln!(w, "{}    float minThreshold = 0", pad)?;
    writeln!(w, "{}    float maxThreshold = 100000", pad)?;
    // Sensor's local pose at the link origin — position 0/0/0 is fine,
    // PhysX measures contacts on the parent body, the sensor just reads.
    writeln!(w, "{}    float3 translate = (0, 0, 0)", pad)?;
    writeln!(w, "{}}}", pad)?;
    writeln!(w)?;
    Ok(())
}

fn emit_mass_api(
    w: &mut impl Write,
    inertial: &Inertial,
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    writeln!(w, "{}float physics:mass = {}", pad, f(inertial.mass))?;
    writeln!(
        w,
        "{}point3f physics:centerOfMass = ({}, {}, {})",
        pad,
        f(inertial.origin.xyz[0]),
        f(inertial.origin.xyz[1]),
        f(inertial.origin.xyz[2])
    )?;
    writeln!(
        w,
        "{}float3 physics:diagonalInertia = ({}, {}, {})",
        pad,
        f(inertial.inertia.ixx),
        f(inertial.inertia.iyy),
        f(inertial.inertia.izz)
    )?;
    Ok(())
}

/// Realistic steel-arm mass + diagonal inertia (kg, kg·m²) for KR240-class
/// industrial robots. Keyed on the URDF link-name suffix that the dresser
/// preserves (`_base_link`, `_link_1`..`_link_6`). Returns `None` for
/// any link we don't recognise — those keep the URDF inertial.
fn steel_arm_inertia(link_name: &str) -> Option<(f64, [f64; 3])> {
    let lower = link_name.to_ascii_lowercase();
    if lower.ends_with("_base_link") || lower.ends_with("base_link") {
        Some((250.0, [15.0, 15.0, 10.0]))
    } else if lower.ends_with("_link_1") {
        Some((200.0, [10.0, 10.0, 8.0]))
    } else if lower.ends_with("_link_2") {
        Some((150.0, [12.0, 12.0, 4.0]))
    } else if lower.ends_with("_link_3") {
        Some((120.0, [8.0, 8.0, 3.0]))
    } else if lower.ends_with("_link_4") {
        Some((60.0, [3.0, 3.0, 1.5]))
    } else if lower.ends_with("_link_5") {
        Some((30.0, [1.0, 1.0, 0.5]))
    } else if lower.ends_with("_link_6") {
        // link_6 + the bolted-on tool flange: ~35 kg combined
        Some((35.0, [0.5, 0.5, 0.4]))
    } else {
        None
    }
}

/// Emit PhysicsMassAPI attributes with overridden mass + diagonalInertia.
/// CenterOfMass is preserved from the URDF when present (otherwise origin).
fn emit_mass_api_override(
    w: &mut impl Write,
    inertial: Option<&Inertial>,
    mass: f64,
    diag: [f64; 3],
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    writeln!(w, "{}float physics:mass = {}", pad, f(mass))?;
    let com = inertial.map(|i| i.origin.xyz).unwrap_or([0.0, 0.0, 0.0]);
    writeln!(
        w,
        "{}point3f physics:centerOfMass = ({}, {}, {})",
        pad,
        f(com[0]),
        f(com[1]),
        f(com[2])
    )?;
    writeln!(
        w,
        "{}float3 physics:diagonalInertia = ({}, {}, {})",
        pad,
        f(diag[0]),
        f(diag[1]),
        f(diag[2])
    )?;
    Ok(())
}

fn emit_visual_mesh(
    w: &mut impl Write,
    visual: &Visual,
    index: usize,
    urdf_dir: &Path,
    urdf_stem: &str,
    cache: &mut MeshCache,
    materials: &mut MaterialRegistry,
    stats: &mut MeshStats,
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);

    // Resolve material before opening the prim so we can attach MaterialBindingAPI.
    let material_prim_name = if let Geometry::Mesh { filename, .. } = &visual.geometry {
        let obj_path = urdf_dir.join(filename);
        let mesh = cache.load(urdf_dir, filename)?;
        materials.register_for_mesh(mesh, &obj_path, urdf_stem)
    } else {
        None
    };

    if material_prim_name.is_some() {
        writeln!(w, "{}def Mesh \"visual_{}\" (", pad, index)?;
        writeln!(
            w,
            "{}    prepend apiSchemas = [\"MaterialBindingAPI\"]",
            pad
        )?;
        writeln!(w, "{})", pad)?;
    } else {
        writeln!(w, "{}def Mesh \"visual_{}\"", pad, index)?;
    }
    writeln!(w, "{}{{", pad)?;
    if let Some(name) = &material_prim_name {
        writeln!(
            w,
            "{}    rel material:binding = </World/Looks/{}>",
            pad, name
        )?;
    }
    emit_geometry_inline(w, &visual.geometry, urdf_dir, cache, stats, indent + 4)?;
    emit_origin_xform(w, &visual.origin, indent + 4)?;
    writeln!(w, "{}}}", pad)?;
    Ok(())
}

fn emit_collision_mesh(
    w: &mut impl Write,
    collision: &Collision,
    index: usize,
    urdf_dir: &Path,
    cache: &mut MeshCache,
    stats: &mut MeshStats,
    indent: usize,
    visible: bool,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    writeln!(w, "{}def Mesh \"collision_{}\" (", pad, index)?;
    writeln!(
        w,
        "{}    prepend apiSchemas = [\"PhysicsCollisionAPI\", \"PhysicsMeshCollisionAPI\"]",
        pad
    )?;
    writeln!(w, "{})", pad)?;
    writeln!(w, "{}{{", pad)?;
    // purpose: "guide" hides the hull behind the visual mesh; "default"
    // keeps it visible. Lean robots want hulls visible; full-fidelity
    // robots and fixtures hide them so the visual mesh wins the draw.
    if !visible {
        writeln!(w, "{}    uniform token purpose = \"guide\"", pad)?;
    }
    writeln!(
        w,
        "{}    uniform token physics:approximation = \"convexHull\"",
        pad
    )?;
    emit_geometry_inline(w, &collision.geometry, urdf_dir, cache, stats, indent + 4)?;
    emit_origin_xform(w, &collision.origin, indent + 4)?;
    writeln!(w, "{}}}", pad)?;
    Ok(())
}

/// Emit USDA mesh attributes inline. For URDF Mesh geometry, parses the OBJ
/// and emits `points` / `faceVertexIndices` / `faceVertexCounts` directly so
/// Isaac Sim doesn't need to invoke the asset converter at load time.
fn emit_geometry_inline(
    w: &mut impl Write,
    geometry: &Geometry,
    urdf_dir: &Path,
    cache: &mut MeshCache,
    stats: &mut MeshStats,
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    match geometry {
        Geometry::Mesh { filename, scale } => {
            let mesh = cache.load(urdf_dir, filename)?;
            let scale = *scale;
            stats.mesh_count += 1;
            stats.total_vertices += mesh.points.len();
            stats.total_faces += mesh.face_vertex_counts.len();

            // points
            write!(w, "{}point3f[] points = [", pad)?;
            for (i, p) in mesh.points.iter().enumerate() {
                if i > 0 {
                    write!(w, ", ")?;
                }
                write!(
                    w,
                    "({}, {}, {})",
                    f(p[0] * scale[0]),
                    f(p[1] * scale[1]),
                    f(p[2] * scale[2])
                )?;
            }
            writeln!(w, "]")?;

            // faceVertexIndices
            write!(w, "{}int[] faceVertexIndices = [", pad)?;
            for (i, idx) in mesh.face_vertex_indices.iter().enumerate() {
                if i > 0 {
                    write!(w, ", ")?;
                }
                write!(w, "{}", idx)?;
            }
            writeln!(w, "]")?;

            // faceVertexCounts
            write!(w, "{}int[] faceVertexCounts = [", pad)?;
            for (i, c) in mesh.face_vertex_counts.iter().enumerate() {
                if i > 0 {
                    write!(w, ", ")?;
                }
                write!(w, "{}", c)?;
            }
            writeln!(w, "]")?;
        }
        Geometry::Box { size } => {
            writeln!(
                w,
                "{}# inline box: size=({}, {}, {})",
                pad,
                f(size[0]),
                f(size[1]),
                f(size[2])
            )?;
        }
        Geometry::Cylinder { radius, length } => {
            writeln!(
                w,
                "{}# inline cylinder: radius={}, length={}",
                pad,
                f(*radius),
                f(*length)
            )?;
        }
        Geometry::Sphere { radius } => {
            writeln!(w, "{}# inline sphere: radius={}", pad, f(*radius))?;
        }
    }
    Ok(())
}

/// Emit a `matrix4d xformOp:transform` attribute for a 4x4 pose matrix.
/// This single-matrix form is preferred for PhysX articulation links because
/// some renderers ignore split translate/orient on articulated bodies.
fn emit_xform_transform(
    w: &mut impl Write,
    m: &Mat4,
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    // USD `matrix4d` is column-major in the type sense but USDA writes it
    // as row vectors of (col, col, col, col). The transpose is taken so the
    // emitted text matches the convention Isaac/Hydra parses correctly.
    writeln!(
        w,
        "{}matrix4d xformOp:transform = ( ({}, {}, {}, {}), ({}, {}, {}, {}), ({}, {}, {}, {}), ({}, {}, {}, {}) )",
        pad,
        f(m[0][0]), f(m[1][0]), f(m[2][0]), f(m[3][0]),
        f(m[0][1]), f(m[1][1]), f(m[2][1]), f(m[3][1]),
        f(m[0][2]), f(m[1][2]), f(m[2][2]), f(m[3][2]),
        f(m[0][3]), f(m[1][3]), f(m[2][3]), f(m[3][3]),
    )?;
    writeln!(
        w,
        "{}uniform token[] xformOpOrder = [\"xformOp:transform\"]",
        pad
    )?;
    Ok(())
}

fn emit_origin_xform(
    w: &mut impl Write,
    origin: &Origin,
    indent: usize,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    let quat = rpy_rad_to_quat(origin.rpy);
    writeln!(
        w,
        "{}double3 xformOp:translate = ({}, {}, {})",
        pad,
        f(origin.xyz[0]),
        f(origin.xyz[1]),
        f(origin.xyz[2])
    )?;
    writeln!(
        w,
        "{}quatd xformOp:orient = ({}, {}, {}, {})",
        pad,
        f(quat[3]),
        f(quat[0]),
        f(quat[1]),
        f(quat[2])
    )?;
    writeln!(
        w,
        "{}uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]",
        pad
    )?;
    Ok(())
}

fn emit_joint(
    w: &mut impl Write,
    joint: &Joint,
    robot_id: &str,
    indent: usize,
    actuated_idx: Option<usize>,
    home_pose_override: Option<&[f64]>,
) -> Result<(), FittingRoomError> {
    let pad = " ".repeat(indent);
    let safe_name = sanitize_prim_name(&joint.name);
    let parent = sanitize_prim_name(&joint.parent);
    let child = sanitize_prim_name(&joint.child);

    let prim_type = match joint.joint_type {
        JointType::Revolute | JointType::Continuous => "PhysicsRevoluteJoint",
        JointType::Prismatic => "PhysicsPrismaticJoint",
        JointType::Fixed => "PhysicsFixedJoint",
        JointType::Floating | JointType::Planar => {
            log::warn!(
                "Joint '{}' has unsupported type '{}'; skipping",
                joint.name,
                joint.joint_type.as_str()
            );
            return Ok(());
        }
    };

    // Actuated joints get `PhysicsDriveAPI:<type>` so Isaac/PhysX exposes a
    // position target via the articulation. Without it the joint is "free"
    // and only moves under gravity — there's no UI handle to drive it.
    let drive_token = match joint.joint_type {
        JointType::Revolute | JointType::Continuous => Some("angular"),
        JointType::Prismatic => Some("linear"),
        _ => None,
    };
    if let Some(tok) = drive_token {
        writeln!(w, "{}def {} \"{}\" (", pad, prim_type, safe_name)?;
        writeln!(
            w,
            "{}    prepend apiSchemas = [\"PhysicsDriveAPI:{}\"]",
            pad, tok
        )?;
        writeln!(w, "{})", pad)?;
    } else {
        writeln!(w, "{}def {} \"{}\"", pad, prim_type, safe_name)?;
    }
    writeln!(w, "{}{{", pad)?;
    writeln!(
        w,
        "{}    rel physics:body0 = </World/robots/{}/{}>",
        pad, robot_id, parent
    )?;
    writeln!(
        w,
        "{}    rel physics:body1 = </World/robots/{}/{}>",
        pad, robot_id, child
    )?;

    if matches!(
        joint.joint_type,
        JointType::Revolute | JointType::Continuous | JointType::Prismatic
    ) {
        let axis_token = canonical_axis_token(&joint.axis);
        writeln!(
            w,
            "{}    uniform token physics:axis = \"{}\"",
            pad, axis_token
        )?;

        if matches!(joint.joint_type, JointType::Revolute | JointType::Prismatic) {
            if let Some(limit) = &joint.limit {
                // PhysX/USD: revolute limits in degrees, prismatic in metres
                let (lower, upper) = match joint.joint_type {
                    JointType::Revolute => (limit.lower.to_degrees(), limit.upper.to_degrees()),
                    _ => (limit.lower, limit.upper),
                };
                writeln!(w, "{}    float physics:lowerLimit = {}", pad, f(lower))?;
                writeln!(w, "{}    float physics:upperLimit = {}", pad, f(upper))?;
            }
        }

        // Drive parameters. Stiffness/damping picked so the joint tracks a
        // commanded target reasonably (industrial-arm scale). Effort from URDF
        // bounds maxForce so PhysX can't exceed the URDF rating; if URDF has
        // no limit, fall back to a high default.
        if let Some(tok) = drive_token {
            let max_force = joint
                .limit
                .as_ref()
                .map(|l| l.effort.max(1.0))
                .unwrap_or(10000.0);
            // Per recommendation Fix 3: starting baseline drive gains.
            // Stiffness 1.5e5 lowers natural frequency well within the
            // 60–120 Hz solver stability envelope; damping 100 is the
            // recommended starting value to suppress high-frequency
            // vibrations without over-damping joint motion.
            writeln!(
                w,
                "{}    float drive:{}:physics:stiffness = {}",
                pad,
                tok,
                f(1.5e5)
            )?;
            writeln!(
                w,
                "{}    float drive:{}:physics:damping = {}",
                pad,
                tok,
                f(100.0)
            )?;
            writeln!(
                w,
                "{}    float drive:{}:physics:maxForce = {}",
                pad,
                tok,
                f(max_force)
            )?;
            // Home-pose target: matches the FK bind pose so the arm sits at
            // the configured home and stays there when Play starts. Revolute
            // targets in degrees, prismatic in metres. FR-118: per-robot
            // override threaded in from the manifest, falls back to KUKA
            // stowed default when absent.
            let target = match joint.joint_type {
                JointType::Revolute => actuated_idx
                    .map(|i| home_angle_for_joint_index(home_pose_override, i).to_degrees())
                    .unwrap_or(0.0),
                _ => 0.0,
            };
            writeln!(
                w,
                "{}    float drive:{}:physics:targetPosition = {}",
                pad,
                tok,
                f(target)
            )?;
        }
    }

    let origin = &joint.origin;
    if origin.xyz != [0.0; 3] || origin.rpy != [0.0; 3] {
        let quat = rpy_rad_to_quat(origin.rpy);
        writeln!(
            w,
            "{}    point3f physics:localPos0 = ({}, {}, {})",
            pad,
            f(origin.xyz[0]),
            f(origin.xyz[1]),
            f(origin.xyz[2])
        )?;
        writeln!(
            w,
            "{}    quatf physics:localRot0 = ({}, {}, {}, {})",
            pad,
            f(quat[3]),
            f(quat[0]),
            f(quat[1]),
            f(quat[2])
        )?;
    }

    writeln!(w, "{}}}", pad)?;
    writeln!(w)?;
    Ok(())
}

fn emit_materials(
    w: &mut impl Write,
    registry: &MaterialRegistry,
) -> Result<(), FittingRoomError> {
    if registry.materials.is_empty() {
        return Ok(());
    }
    writeln!(w, "    def Scope \"Looks\"")?;
    writeln!(w, "    {{")?;
    for (name, mat) in &registry.materials {
        writeln!(w, "        def Material \"{}\"", name)?;
        writeln!(w, "        {{")?;
        writeln!(
            w,
            "            token outputs:surface.connect = </World/Looks/{}/Shader.outputs:surface>",
            name
        )?;
        writeln!(w)?;
        writeln!(w, "            def Shader \"Shader\"")?;
        writeln!(w, "            {{")?;
        writeln!(w, "                uniform token info:id = \"UsdPreviewSurface\"")?;
        writeln!(
            w,
            "                color3f inputs:diffuseColor = ({}, {}, {})",
            f(mat.diffuse[0]),
            f(mat.diffuse[1]),
            f(mat.diffuse[2])
        )?;
        writeln!(
            w,
            "                float inputs:metallic = {}",
            f(mat.metallic())
        )?;
        writeln!(
            w,
            "                float inputs:roughness = {}",
            f(mat.roughness())
        )?;
        writeln!(
            w,
            "                float inputs:opacity = {}",
            f(mat.opacity)
        )?;
        writeln!(w, "                token outputs:surface")?;
        writeln!(w, "            }}")?;
        writeln!(w, "        }}")?;
        writeln!(w)?;
    }
    writeln!(w, "    }}")?;
    writeln!(w)?;
    Ok(())
}

// FR-150, FR-151, FR-152, FR-153, FR-160, FR-161, FR-162
//
// Emit OmniGraph ActionGraphs for ROS2 control of every robot, plus a
// scene-level /clock publisher. Each robot gets:
//   * ROS2PublishJointState on `/<cell_id>/<robot_id>/joint_states`
//   * ROS2SubscribeJointState on `/<cell_id>/<robot_id>/joint_command`
//   * ArticulationController fed from the subscribed targets
//   * IsaacReadSimulationTime providing timestamps
//   * One IsaacReadContactSensor + ROS2PublishContactSensor pair per
//     collision-bearing link (FR-160/161/162). The publisher's execIn is
//     wired to the sensor's execOut, NOT OnPlaybackTick — so the topic is
//     silent until a contact event lands in the current frame.
//
// Topic schema is two-segment: `/<cell_id>/<robot_id>/<channel>`. The
// `cell_id` is the manifest's `name` field (sanitised the same way
// `robot_id` is). See `ros_namespace`.
fn emit_action_graphs(
    w: &mut impl Write,
    cell_id: &str,
    robots: &[RobotPlacement],
) -> Result<(), FittingRoomError> {
    if robots.is_empty() {
        return Ok(());
    }

    let cell_ns = sanitize_ros_segment(cell_id);
    emit_clock_action_graph(w, &cell_ns)?;

    for placement in robots {
        let robot = match parse_urdf(&placement.dressed_urdf) {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "Skipping ActionGraph for {}: URDF parse failed: {}",
                    placement.robot_id,
                    e
                );
                continue;
            }
        };
        let joint_names: Vec<String> = robot
            .joints
            .iter()
            .filter(|j| !matches!(j.joint_type, JointType::Fixed))
            .map(|j| j.name.clone())
            .collect();
        if joint_names.is_empty() {
            continue;
        }
        let collision_links = collect_collision_link_names(&robot);
        emit_robot_action_graph(
            w,
            &cell_ns,
            &placement.robot_id,
            &joint_names,
            &collision_links,
        )?;
    }
    Ok(())
}

/// Sanitise one segment of a ROS 2 namespace. The output is constrained
/// to match the rmw token regex `[A-Za-z_][A-Za-z0-9_]*` so the emitted
/// USDA validates equally under FastDDS (Isaac default) **and**
/// `rmw_cyclonedds_cpp` (the Cyclone DDS RMW), per
/// `rmw/validate_namespace.h` and `rmw/validate_topic_name.h`. Rules:
///   * Digit-leading raw values get an `r_` prefix (e.g. `113190R01` →
///     `r_113190R01`), preserving every original character.
///   * Non-alphanumeric characters become `_`.
///   * If the result is empty (e.g. caller passed a blank string),
///     fall back to `r_anon` so we never emit an empty segment.
/// Used for both the `cell_id` and the `robot_id` segments of the
/// `<cell_id>/<robot_id>/<channel>` topic schema. Idempotent — calling it
/// twice on the same input produces the same output. Multi-segment
/// composition is the caller's job (see `ros_namespace`); each segment
/// is validated independently so the resulting full path passes both
/// rmw validators end-to-end.
fn sanitize_ros_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len() + 2);
    let needs_prefix = segment
        .chars()
        .next()
        .map(|c| c.is_ascii_digit())
        .unwrap_or(true);
    if needs_prefix {
        out.push_str("r_");
    }
    for c in segment.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "r_anon".to_string();
    }
    out
}

/// Compose the two-segment ROS 2 namespace `<cell_id>/<robot_id>` from the
/// raw manifest values. Each segment passes through `sanitize_ros_segment`.
fn ros_namespace(cell_id: &str, robot_id: &str) -> String {
    format!(
        "{}/{}",
        sanitize_ros_segment(cell_id),
        sanitize_ros_segment(robot_id)
    )
}

/// FR-162. Sanitised names of links carrying at least one `<collision>`
/// block. Derived purely from the URDF — no manifest knob.
fn collect_collision_link_names(robot: &UrdfRobot) -> Vec<String> {
    robot
        .links
        .iter()
        .filter(|link| !link.collisions.is_empty())
        .map(|link| sanitize_prim_name(&link.name))
        .collect()
}

// OmniGraph emission notes (Isaac 5.1):
// - The graph prim is `def OmniGraph "...."` (not a plain `def` with
//   apiSchemas = ["OmniGraphAPI"]). Plain defs with the API schema can
//   load as half-instantiated graphs whose node rels resolve to -1
//   handles, causing access-violation crashes when the bridge ticks.
// - Attribute connections (`.connect`) must use `add` list-edit op so
//   the OmniGraph rel walker finds them. `prepend` and the default
//   (un-listed) form are both unreliable across Kit minor versions.
// - Rel-typed inputs like `inputs:targetPrim` must be declared once with
//   their target via `add rel <name> = <path>` — declaring `custom rel`
//   first and then `prepend rel ... = ...` separately leaves the bridge
//   resolving the first (empty) declaration, returning -1.

fn emit_clock_action_graph(
    w: &mut impl Write,
    cell_ns: &str,
) -> Result<(), FittingRoomError> {
    writeln!(w, "    def OmniGraph \"ActionGraph_clock\"")?;
    writeln!(w, "    {{")?;
    writeln!(w, "        token evaluationMode = \"Automatic\"")?;
    writeln!(w, "        token evaluator:type = \"execution\"")?;
    writeln!(w, "        token fabricCacheBacking = \"Shared\"")?;
    writeln!(w, "        int2 fileFormatVersion = (1, 9)")?;
    writeln!(w, "        token pipelineStage = \"pipelineStageSimulation\"")?;
    writeln!(w)?;
    writeln!(w, "        def OmniGraphNode \"OnPlaybackTick\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"omni.graph.action.OnPlaybackTick\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint outputs:tick (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;
    writeln!(w, "        def OmniGraphNode \"IsaacReadSimulationTime\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.core.nodes.IsaacReadSimulationTime\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom bool inputs:resetOnStop = 0")?;
    writeln!(w, "            custom double outputs:simulationTime")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;
    writeln!(w, "        def OmniGraphNode \"ROS2PublishClock\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.ros2.bridge.ROS2PublishClock\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint inputs:execIn (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(
        w,
        "            add uint inputs:execIn.connect = </World/ActionGraph_clock/OnPlaybackTick.outputs:tick>"
    )?;
    // FR-152: clock topic is cell-scoped (`/<cell_id>/clock`) so multiple
    // cells sharing one ROS_DOMAIN_ID don't collide on `/clock`.
    writeln!(w, "            custom string inputs:nodeNamespace = \"{}\"", cell_ns)?;
    writeln!(w, "            custom string inputs:topicName = \"clock\"")?;
    writeln!(w, "            custom double inputs:timeStamp = 0")?;
    writeln!(
        w,
        "            add double inputs:timeStamp.connect = </World/ActionGraph_clock/IsaacReadSimulationTime.outputs:simulationTime>"
    )?;
    writeln!(w, "        }}")?;
    writeln!(w, "    }}")?;
    writeln!(w)?;
    Ok(())
}

fn emit_robot_action_graph(
    w: &mut impl Write,
    cell_ns: &str,
    robot_id: &str,
    joint_names: &[String],
    collision_link_names: &[String],
) -> Result<(), FittingRoomError> {
    let safe_id = sanitize_prim_name(robot_id);
    let graph_name = format!("ActionGraph_{}", safe_id);
    let articulation_path = format!("/World/robots/{}", safe_id);
    // FR-151: two-segment namespace `<cell_id>/<robot_id>`. Both segments
    // are sanitised through `sanitize_ros_segment` (digit-leading values
    // get an `r_` prefix; everything else is alphanumerics + underscores).
    let ns = ros_namespace(cell_ns, robot_id);
    let ns = ns.as_str();

    let joint_token_array = joint_names
        .iter()
        .map(|j| format!("\"{}\"", j))
        .collect::<Vec<_>>()
        .join(", ");

    writeln!(w, "    def OmniGraph \"{}\"", graph_name)?;
    writeln!(w, "    {{")?;
    writeln!(w, "        token evaluationMode = \"Automatic\"")?;
    writeln!(w, "        token evaluator:type = \"execution\"")?;
    writeln!(w, "        token fabricCacheBacking = \"Shared\"")?;
    writeln!(w, "        int2 fileFormatVersion = (1, 9)")?;
    writeln!(w, "        token pipelineStage = \"pipelineStageSimulation\"")?;
    writeln!(w)?;

    // OnPlaybackTick — drives the per-frame execution chain.
    writeln!(w, "        def OmniGraphNode \"OnPlaybackTick\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"omni.graph.action.OnPlaybackTick\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint outputs:tick (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;

    writeln!(w, "        def OmniGraphNode \"IsaacReadSimulationTime\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.core.nodes.IsaacReadSimulationTime\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom bool inputs:resetOnStop = 0")?;
    writeln!(w, "            custom double outputs:simulationTime")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;

    writeln!(w, "        def OmniGraphNode \"PublishJointState\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.ros2.bridge.ROS2PublishJointState\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint inputs:execIn (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(
        w,
        "            add uint inputs:execIn.connect = </World/{}/OnPlaybackTick.outputs:tick>",
        graph_name
    )?;
    // Single-statement rel: declares custom + sets target in one pass so
    // the resolver doesn't see two competing declarations.
    writeln!(
        w,
        "            add rel inputs:targetPrim = <{}>",
        articulation_path
    )?;
    writeln!(
        w,
        "            custom string inputs:nodeNamespace = \"{}\"",
        ns
    )?;
    writeln!(
        w,
        "            custom string inputs:topicName = \"joint_states\""
    )?;
    writeln!(w, "            custom double inputs:timeStamp = 0")?;
    writeln!(
        w,
        "            add double inputs:timeStamp.connect = </World/{}/IsaacReadSimulationTime.outputs:simulationTime>",
        graph_name
    )?;
    writeln!(w, "        }}")?;
    writeln!(w)?;

    writeln!(w, "        def OmniGraphNode \"SubscribeJointState\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.ros2.bridge.ROS2SubscribeJointState\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint inputs:execIn (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(
        w,
        "            add uint inputs:execIn.connect = </World/{}/OnPlaybackTick.outputs:tick>",
        graph_name
    )?;
    writeln!(
        w,
        "            custom string inputs:nodeNamespace = \"{}\"",
        ns
    )?;
    writeln!(
        w,
        "            custom string inputs:topicName = \"joint_command\""
    )?;
    writeln!(w, "            custom uint outputs:execOut (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(w, "            custom token[] outputs:jointNames")?;
    writeln!(w, "            custom double[] outputs:positionCommand")?;
    writeln!(w, "            custom double[] outputs:velocityCommand")?;
    writeln!(w, "            custom double[] outputs:effortCommand")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;

    writeln!(w, "        def OmniGraphNode \"ArticulationController\"")?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.core.nodes.IsaacArticulationController\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint inputs:execIn (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(
        w,
        "            add uint inputs:execIn.connect = </World/{}/SubscribeJointState.outputs:execOut>",
        graph_name
    )?;
    writeln!(
        w,
        "            add rel inputs:targetPrim = <{}>",
        articulation_path
    )?;
    writeln!(
        w,
        "            custom token[] inputs:jointNames = [{}]",
        joint_token_array
    )?;
    writeln!(
        w,
        "            add token[] inputs:jointNames.connect = </World/{}/SubscribeJointState.outputs:jointNames>",
        graph_name
    )?;
    writeln!(w, "            custom double[] inputs:positionCommand")?;
    writeln!(
        w,
        "            add double[] inputs:positionCommand.connect = </World/{}/SubscribeJointState.outputs:positionCommand>",
        graph_name
    )?;
    writeln!(w, "            custom double[] inputs:velocityCommand")?;
    writeln!(
        w,
        "            add double[] inputs:velocityCommand.connect = </World/{}/SubscribeJointState.outputs:velocityCommand>",
        graph_name
    )?;
    writeln!(w, "            custom double[] inputs:effortCommand")?;
    writeln!(
        w,
        "            add double[] inputs:effortCommand.connect = </World/{}/SubscribeJointState.outputs:effortCommand>",
        graph_name
    )?;
    writeln!(w, "        }}")?;
    writeln!(w)?;

    // FR-160 / FR-161 / FR-162. Per-link contact sensor + ROS 2 publisher
    // pairs, inlined into this same per-robot graph so we reuse the
    // existing OnPlaybackTick. Wiring is automatic (URDF-derived); the
    // publisher's execIn is the sensor's execOut, NOT the tick — so the
    // topic stays silent until a contact event lands.
    for link_name in collision_link_names {
        emit_contact_sensor_pair(w, ns, &graph_name, &safe_id, link_name)?;
    }

    writeln!(w, "    }}")?;
    writeln!(w)?;
    Ok(())
}

/// FR-160 / FR-161 / FR-162. Emit one `IsaacReadContactSensor` + a
/// generic `ROS2Publisher` pair for a single collision link, inside the
/// already-open per-robot OmniGraph.
///
/// Wire shape:
///   OnPlaybackTick.tick → ContactSensor_<link>.execIn
///   ContactSensor_<link>.execOut → PublishContact_<link>.execIn   (FR-161 silence)
///   ContactSensor_<link>.inContact → PublishContact_<link>.data   (`std_msgs/msg/Bool`)
///
/// We deliberately use the generic `isaacsim.ros2.bridge.ROS2Publisher`
/// rather than `ROS2PublishContactSensor` because the latter does not
/// exist in Isaac Sim 5.1 — Isaac silently drops the node, the topic
/// never advertises, and `ros2 topic list` never sees the contact
/// channel. The generic publisher supports any registered ROS 2 message
/// type by name; we ship a `std_msgs/msg/Bool` payload carrying the
/// sensor's `inContact` flag. Rich force/normal context comes from the
/// cell-aggregator pod (BL-24, FR-213) which subscribes to these Bool
/// topics, queries TF + PhysX for the impact frame on the rising edge,
/// and re-publishes a `cell_sim_contact_msgs/msg/CollisionEvent` on
/// `/<cell>/contacts/v1`.
///
/// Topic FQT: `/<cell_id>/<robot_id>/contacts/<link_name>` —
/// `inputs:nodeNamespace = "<cell>/<robot>"` + `inputs:topicName =
/// "contacts/<link>"`. Matches the FR-151 + FR-160 contract.
fn emit_contact_sensor_pair(
    w: &mut impl Write,
    namespace: &str,
    graph_name: &str,
    safe_robot_id: &str,
    link_name: &str,
) -> Result<(), FittingRoomError> {
    let sensor_node = format!("ContactSensor_{}", link_name);
    let publisher_node = format!("PublishContact_{}", link_name);
    // Path to the per-link `IsaacContactSensor` USD prim emitted by
    // `emit_link_contact_sensor_prim`. The OmniGraph's `csPrim` rel
    // MUST point at this sensor prim, not the link Xform — pointing at
    // the link gives Isaac's "no valid sensor reading, is the sensor
    // enabled?" warning and silently drops events.
    let sensor_prim_path = format!(
        "/World/robots/{}/{}/ContactSensor_node",
        safe_robot_id, link_name
    );

    // Sensor — reads contact events from PhysX for the link prim.
    // PhysX must have PhysxContactReportAPI on the rigid body for events
    // to surface here (applied in emit_link when link.collisions is non-empty).
    writeln!(w, "        def OmniGraphNode \"{}\"", sensor_node)?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.sensors.physics.IsaacReadContactSensor\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint inputs:execIn (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(
        w,
        "            add uint inputs:execIn.connect = </World/{}/OnPlaybackTick.outputs:tick>",
        graph_name
    )?;
    writeln!(
        w,
        "            add rel inputs:csPrim = <{}>",
        sensor_prim_path
    )?;
    writeln!(w, "            custom uint outputs:execOut (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(w, "            custom bool outputs:inContact")?;
    writeln!(w, "        }}")?;
    writeln!(w)?;

    // Generic ROS2Publisher emitting `std_msgs/msg/Bool` carrying the
    // sensor's inContact flag. The exec chain (sensor → publisher)
    // gives the FR-161 silence guarantee; the data input wires the
    // payload field by attribute connection.
    writeln!(w, "        def OmniGraphNode \"{}\"", publisher_node)?;
    writeln!(w, "        {{")?;
    writeln!(
        w,
        "            token node:type = \"isaacsim.ros2.bridge.ROS2Publisher\""
    )?;
    writeln!(w, "            int node:typeVersion = 1")?;
    writeln!(w, "            custom uint inputs:execIn (")?;
    writeln!(w, "                customData = {{ bool isExecution = 1 }}")?;
    writeln!(w, "            )")?;
    writeln!(
        w,
        "            add uint inputs:execIn.connect = </World/{}/{}.outputs:execOut>",
        graph_name, sensor_node
    )?;
    writeln!(
        w,
        "            custom string inputs:messagePackage = \"std_msgs\""
    )?;
    writeln!(
        w,
        "            custom string inputs:messageSubfolder = \"msg\""
    )?;
    writeln!(
        w,
        "            custom string inputs:messageName = \"Bool\""
    )?;
    writeln!(
        w,
        "            custom string inputs:nodeNamespace = \"{}\"",
        namespace
    )?;
    writeln!(
        w,
        "            custom string inputs:topicName = \"contacts/{}\"",
        link_name
    )?;
    writeln!(w, "            custom bool inputs:data = 0")?;
    writeln!(
        w,
        "            add bool inputs:data.connect = </World/{}/{}.outputs:inContact>",
        graph_name, sensor_node
    )?;
    writeln!(w, "        }}")?;
    writeln!(w)?;
    Ok(())
}

/// Floor-plan emission. Currently unused — FR-107 disables it in the
/// compose pipeline because Hydra (CAR-811) won't render BasisCurves /
/// thin flat meshes reliably. Kept around for BL-09 (re-enable once
/// the renderer fix lands or we decide on an alternative geometry).
#[allow(dead_code)]
fn emit_floor_plan(w: &mut impl Write, fp: &FloorPlan) -> Result<(), FittingRoomError> {
    if fp.segments.is_empty() && fp.footprint_segments.is_empty() {
        return Ok(());
    }

    // Workaround for Hydra rendering BasisCurves as empty geometry (CAR-811).
    // Build a flat Mesh: each line segment becomes a thin rectangle (4 verts,
    // 2 triangles) lying on the Z=0 plane, with the rectangle's narrow axis
    // perpendicular to the segment.
    writeln!(w, "    def Xform \"floor_plan\"")?;
    writeln!(w, "    {{")?;
    writeln!(
        w,
        "        double3 xformOp:translate = (0, 0, 0)"
    )?;
    writeln!(
        w,
        "        quatd xformOp:orient = (1, 0, 0, 0)"
    )?;
    writeln!(
        w,
        "        uniform token[] xformOpOrder = [\"xformOp:translate\", \"xformOp:orient\"]"
    )?;

    if !fp.segments.is_empty() {
        emit_segment_mesh(w, "lines", &fp.segments, [0.5, 0.5, 0.5], 8)?;
    }
    if !fp.footprint_segments.is_empty() {
        emit_segment_mesh(w, "footprints", &fp.footprint_segments, [0.7, 0.4, 0.0], 8)?;
    }

    writeln!(w, "    }}")?;
    writeln!(w)?;
    Ok(())
}

/// Render a list of 2D line segments as a flat Mesh (thin rectangles on Z=0).
/// Width of each strip is fixed at 0.02 m (2 cm) — visible at cell scale,
/// invisible from a robot's-eye view. Helper for `emit_floor_plan`; both
/// are dormant pending BL-09.
#[allow(dead_code)]
fn emit_segment_mesh(
    w: &mut impl Write,
    name: &str,
    segments: &[[[f64; 3]; 2]],
    color: [f64; 3],
    indent: usize,
) -> Result<(), FittingRoomError> {
    const HALF_WIDTH: f64 = 0.01; // half of 2 cm
    let pad = " ".repeat(indent);

    writeln!(w, "{}def Mesh \"{}\"", pad, name)?;
    writeln!(w, "{}{{", pad)?;
    writeln!(w, "{}    uniform token purpose = \"guide\"", pad)?;
    writeln!(
        w,
        "{}    color3f[] primvars:displayColor = [({}, {}, {})] (",
        pad,
        f(color[0]),
        f(color[1]),
        f(color[2])
    )?;
    writeln!(w, "{}        interpolation = \"constant\"", pad)?;
    writeln!(w, "{}    )", pad)?;

    // Build verts + faces in memory
    let mut points: Vec<[f64; 3]> = Vec::with_capacity(segments.len() * 4);
    let mut indices: Vec<u32> = Vec::with_capacity(segments.len() * 6);

    for seg in segments {
        let a = seg[0];
        let b = seg[1];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            continue;
        }
        // Perpendicular in XY plane, length HALF_WIDTH
        let nx = -dy / len * HALF_WIDTH;
        let ny = dx / len * HALF_WIDTH;

        let base = points.len() as u32;
        points.push([a[0] + nx, a[1] + ny, a[2]]); // 0
        points.push([a[0] - nx, a[1] - ny, a[2]]); // 1
        points.push([b[0] - nx, b[1] - ny, b[2]]); // 2
        points.push([b[0] + nx, b[1] + ny, b[2]]); // 3
        // Triangle 1: 0, 1, 2
        indices.extend_from_slice(&[base, base + 1, base + 2]);
        // Triangle 2: 0, 2, 3
        indices.extend_from_slice(&[base, base + 2, base + 3]);
    }

    let face_count = indices.len() / 3;

    write!(w, "{}    point3f[] points = [", pad)?;
    for (i, p) in points.iter().enumerate() {
        if i > 0 {
            write!(w, ", ")?;
        }
        write!(w, "({}, {}, {})", f(p[0]), f(p[1]), f(p[2]))?;
    }
    writeln!(w, "]")?;

    write!(w, "{}    int[] faceVertexCounts = [", pad)?;
    for i in 0..face_count {
        if i > 0 {
            write!(w, ", ")?;
        }
        write!(w, "3")?;
    }
    writeln!(w, "]")?;

    write!(w, "{}    int[] faceVertexIndices = [", pad)?;
    for (i, idx) in indices.iter().enumerate() {
        if i > 0 {
            write!(w, ", ")?;
        }
        write!(w, "{}", idx)?;
    }
    writeln!(w, "]")?;

    writeln!(w, "{}}}", pad)?;
    Ok(())
}

fn find_root_link(robot: &UrdfRobot) -> Option<&str> {
    let mut children: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for j in &robot.joints {
        children.insert(j.child.as_str());
    }
    robot
        .links
        .iter()
        .find(|l| !children.contains(l.name.as_str()))
        .map(|l| l.name.as_str())
}

/// Hard-coded home pose for 6-axis KUKA-style industrial arms (radians).
/// Stows the arm folded upright so it doesn't intersect the floor or its base.
/// Indexed by *actuated* joint occurrence order (skipping fixed joints), which
/// matches URDF kinematic-chain order for these robots.
const KUKA_HOME_POSE_RAD: [f64; 6] = [
    0.0,
    -std::f64::consts::FRAC_PI_2,  // a2 = -90°
    std::f64::consts::FRAC_PI_2,   //  a3 = +90°
    0.0,
    0.0,
    0.0,
];

/// Resolve the home angle (in radians) for an actuated-joint index.
/// FR-118: per-robot `home_pose_deg` overrides take precedence; absent
/// or out-of-range entries fall back to the KUKA stowed default.
fn home_angle_for_joint_index(override_deg: Option<&[f64]>, idx: usize) -> f64 {
    if let Some(arr) = override_deg {
        return arr.get(idx).copied().unwrap_or(0.0).to_radians();
    }
    KUKA_HOME_POSE_RAD.get(idx).copied().unwrap_or(0.0)
}

/// Compute each link's bind-pose transform relative to the URDF root link,
/// at the configured home pose. The root link maps to the identity matrix.
///
/// PhysX's articulation pose initialiser expects each link prim to be at
/// its FK pose at scene load — without this, joint frames are "disjointed"
/// and the warning floods every simulation step.
///
/// FR-117 / FR-118 (home pose). When `home_pose_override` is `Some`, its
/// entries (degrees, indexed by actuated-joint occurrence) drive the
/// per-joint angle baked into each link's bind pose; otherwise the
/// KUKA stowed default is used. Fixtures pass `None`.
fn compute_link_bind_poses(
    robot: &UrdfRobot,
    home_pose_override: Option<&[f64]>,
) -> std::collections::HashMap<String, Mat4> {
    use std::collections::HashMap;

    let mut by_name: HashMap<String, Mat4> = HashMap::new();
    let root = match find_root_link(robot) {
        Some(r) => r.to_string(),
        None => return by_name,
    };
    by_name.insert(root.clone(), math3::IDENTITY);

    // Build a stable actuated-joint index so we can look up the home angle.
    // URDF declaration order matches the kinematic chain for these robots.
    let mut actuated_idx_by_joint: HashMap<&str, usize> = HashMap::new();
    let mut next_idx = 0usize;
    for j in &robot.joints {
        if !matches!(j.joint_type, JointType::Fixed) {
            actuated_idx_by_joint.insert(j.name.as_str(), next_idx);
            next_idx += 1;
        }
    }

    // Index joints by parent link name for tree traversal.
    let mut children_of: HashMap<&str, Vec<&urdf_robo_dresser::Joint>> = HashMap::new();
    for j in &robot.joints {
        children_of.entry(j.parent.as_str()).or_default().push(j);
    }

    // BFS from the root.
    let mut stack: Vec<String> = vec![root];
    while let Some(parent) = stack.pop() {
        let parent_pose = by_name.get(&parent).copied().unwrap_or(math3::IDENTITY);
        if let Some(joints) = children_of.get(parent.as_str()) {
            for joint in joints {
                let joint_local =
                    math3::from_translation_rpy(joint.origin.xyz, joint.origin.rpy);
                // Apply the home angle as an additional rotation about the
                // joint's axis, expressed in the joint's local frame.
                let home_local = if let Some(&idx) = actuated_idx_by_joint.get(joint.name.as_str())
                {
                    match joint.joint_type {
                        JointType::Revolute | JointType::Continuous => {
                            let a = home_angle_for_joint_index(home_pose_override, idx);
                            if a == 0.0 {
                                math3::IDENTITY
                            } else {
                                math3::rotation_about_axis(joint.axis, a)
                            }
                        }
                        _ => math3::IDENTITY,
                    }
                } else {
                    math3::IDENTITY
                };
                let with_home = math3::multiply(&joint_local, &home_local);
                let child_pose = math3::multiply(&parent_pose, &with_home);
                by_name.insert(joint.child.clone(), child_pose);
                stack.push(joint.child.clone());
            }
        }
    }

    by_name
}

/// Detect whether a Visual/Collision references a tool mesh by inspecting
/// the OBJ filename. The dresser writes tool meshes under `meshes/tool/`
/// and robot meshes under `meshes/robot/` inside the dressed URDF
/// directory; checking either separator covers Windows + POSIX-style
/// authoring without needing to canonicalise the path.
fn is_tool_mesh_path(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    lower.contains("meshes/tool/") || lower.contains("meshes\\tool\\")
}

fn is_tool_visual(visual: &Visual) -> bool {
    matches!(&visual.geometry, Geometry::Mesh { filename, .. } if is_tool_mesh_path(filename))
}

fn is_tool_collision(collision: &Collision) -> bool {
    matches!(&collision.geometry, Geometry::Mesh { filename, .. } if is_tool_mesh_path(filename))
}

fn sanitize_prim_name(name: &str) -> String {
    // Avoid "World" as a child prim name — it shadows the stage root and
    // confuses PhysX's transform hierarchy walk.
    if name == "World" {
        return "world_link".to_string();
    }
    let mut out = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if i == 0 {
            if c.is_ascii_alphabetic() || c == '_' {
                out.push(c);
            } else {
                out.push('_');
                if c.is_ascii_alphanumeric() {
                    out.push(c);
                }
            }
        } else if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    out
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn f(v: f64) -> String {
    if !v.is_finite() {
        return "0".to_string();
    }
    let abs = v.abs();
    // For very large or very small magnitudes, emit scientific notation —
    // otherwise `{:.6}` produces 30+ digit integer literals (URDF effort
    // defaults of f32::MAX expand to 3.4028e38 → the USD parser warns
    // "Integer literal out of range" and re-parses as double).
    if abs != 0.0 && (abs >= 1.0e15 || abs < 1.0e-6) {
        return format!("{:e}", v);
    }
    let s = format!("{:.6}", v);
    if s.contains('.') {
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() || trimmed == "-" {
            "0".to_string()
        } else {
            trimmed.to_string()
        }
    } else {
        s
    }
}

fn canonical_axis_token(axis: &[f64; 3]) -> String {
    let eps = 1e-6;
    let near = |a: f64, b: f64| (a - b).abs() < eps;
    if (near(axis[0], 1.0) || near(axis[0], -1.0)) && near(axis[1], 0.0) && near(axis[2], 0.0) {
        "X".to_string()
    } else if near(axis[0], 0.0) && (near(axis[1], 1.0) || near(axis[1], -1.0)) && near(axis[2], 0.0) {
        "Y".to_string()
    } else if near(axis[0], 0.0) && near(axis[1], 0.0) && (near(axis[2], 1.0) || near(axis[2], -1.0)) {
        "Z".to_string()
    } else {
        "Z".to_string()
    }
}

pub fn abc_deg_to_quat(abc_deg: [f64; 3]) -> [f64; 4] {
    let a = abc_deg[0].to_radians();
    let b = abc_deg[1].to_radians();
    let c = abc_deg[2].to_radians();
    rpy_zyx_to_quat(c, b, a)
}

pub fn rpy_rad_to_quat(rpy: [f64; 3]) -> [f64; 4] {
    let (roll, pitch, yaw) = (rpy[0], rpy[1], rpy[2]);
    rpy_zyx_to_quat(roll, pitch, yaw)
}

fn rpy_zyx_to_quat(roll: f64, pitch: f64, yaw: f64) -> [f64; 4] {
    let cr = (roll * 0.5).cos();
    let sr = (roll * 0.5).sin();
    let cp = (pitch * 0.5).cos();
    let sp = (pitch * 0.5).sin();
    let cy = (yaw * 0.5).cos();
    let sy = (yaw * 0.5).sin();

    let qw = cr * cp * cy + sr * sp * sy;
    let qx = sr * cp * cy - cr * sp * sy;
    let qy = cr * sp * cy + sr * cp * sy;
    let qz = cr * cp * sy - sr * sp * cy;
    // Canonicalise so w >= 0 (PhysX prefers this; mathematically equivalent rotation).
    if qw < 0.0 {
        [-qx, -qy, -qz, -qw]
    } else {
        [qx, qy, qz, qw]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    // `Write` is referenced implicitly by `writeln!` macros below; the
    // compiler doesn't see it as a name lookup so we don't import it
    // explicitly.

    fn write_minimal_urdf(path: &Path, name: &str, n_links: usize) {
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir.join("meshes")).unwrap();
        for i in 0..n_links {
            let mesh_path = dir.join(format!("meshes/link_{}.obj", i));
            let mut f = std::fs::File::create(&mesh_path).unwrap();
            writeln!(f, "v 0 0 0").unwrap();
            writeln!(f, "v 1 0 0").unwrap();
            writeln!(f, "v 0 1 0").unwrap();
            writeln!(f, "f 1 2 3").unwrap();
        }

        let mut s = format!("<?xml version=\"1.0\"?>\n<robot name=\"{}\">\n", name);
        for i in 0..n_links {
            s.push_str(&format!(
                "  <link name=\"link_{}\">\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/link_{}.obj\"/>\n      </geometry>\n    </visual>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/link_{}.obj\"/>\n      </geometry>\n    </collision>\n  </link>\n",
                i, i, i
            ));
        }
        for i in 0..n_links.saturating_sub(1) {
            s.push_str(&format!(
                "  <joint name=\"joint_{}\" type=\"revolute\">\n    <parent link=\"link_{}\"/>\n    <child link=\"link_{}\"/>\n    <origin xyz=\"0 0 0\" rpy=\"0 0 0\"/>\n    <axis xyz=\"0 0 1\"/>\n    <limit lower=\"-1.5\" upper=\"1.5\" effort=\"100\" velocity=\"1\"/>\n  </joint>\n",
                i, i, i + 1
            ));
        }
        s.push_str("</robot>\n");
        std::fs::File::create(path)
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }

    fn write_fixed_only_urdf(path: &Path, name: &str) {
        let s = format!(
            "<?xml version=\"1.0\"?>\n<robot name=\"{}\">\n  <link name=\"base\"/>\n  <link name=\"plate\"/>\n  <joint name=\"j_fix\" type=\"fixed\">\n    <parent link=\"base\"/>\n    <child link=\"plate\"/>\n    <origin xyz=\"0 0 0\" rpy=\"0 0 0\"/>\n  </joint>\n</robot>\n",
            name
        );
        std::fs::File::create(path)
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }

    fn write_dummy_manifest(path: &Path) {
        std::fs::write(path, "name: \"test\"\nrobots: []\nfixtures: []\n").unwrap();
    }

    fn list_zip_entries(zip_path: &Path) -> Vec<String> {
        let f = File::open(zip_path).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        (0..z.len())
            .map(|i| z.by_index(i).unwrap().name().to_string())
            .collect()
    }

    fn read_zip_entry(zip_path: &Path, entry: &str) -> String {
        let f = File::open(zip_path).unwrap();
        let mut z = zip::ZipArchive::new(f).unwrap();
        let mut e = z.by_name(entry).unwrap();
        let mut s = String::new();
        e.read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn single_link_inline_mesh() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("r1.urdf");
        write_minimal_urdf(&urdf, "r1", 1);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "t".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "ROBO_A".into(),
                dressed_urdf: urdf,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        // Use the full-robots compose so both visual + collision mesh
        // emission paths are exercised.
        let opts = ComposeOptions {
            full_robots: true,
            full_tools: true,
        };
        let report =
            compose_stage_with_options(&scene, &manifest, &out, &opts).unwrap();
        assert_eq!(report.robot_count, 1);
        assert_eq!(report.mesh_count, 2); // 1 visual + 1 collision

        let usda = read_zip_entry(&out, "scene.usda");
        assert!(usda.contains("def Xform \"ROBO_A\""));
        assert!(usda.contains("PhysicsArticulationRootAPI"));
        assert!(usda.contains("def Xform \"link_0\""));
        // Articulation requires every link — root included — to be a rigid body.
        assert!(usda.contains("PhysicsRigidBodyAPI"));
        // Geometry is inlined per visual/collision, no /World/Meshes scope.
        assert!(!usda.contains("def Scope \"Meshes\""));
        assert!(usda.contains("point3f[] points = ["));
        assert!(usda.contains("int[] faceVertexIndices = ["));
        assert!(usda.contains("int[] faceVertexCounts = ["));

        let entries = list_zip_entries(&out);
        assert!(entries.contains(&"scene.yaml".to_string()));
        assert!(entries.contains(&"scene.usda".to_string()));
        // No meshes/ folder anymore
        assert!(!entries.iter().any(|e| e.starts_with("meshes/")));
    }

    #[test]
    fn lean_robots_drop_visuals_keep_collisions() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("r1.urdf");
        write_minimal_urdf(&urdf, "r1", 1);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "lean".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "ROBO".into(),
                dressed_urdf: urdf,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        // Default ComposeOptions has full_robots = false → lean path.
        let opts = ComposeOptions::default();
        let report =
            compose_stage_with_options(&scene, &manifest, &out, &opts).unwrap();
        assert_eq!(report.mesh_count, 1, "only collision mesh expected");

        let usda = read_zip_entry(&out, "scene.usda");
        // No `visual_*` prim, articulation root and joint still present.
        assert!(!usda.contains("def Mesh \"visual_"));
        assert!(usda.contains("def Mesh \"collision_"));
        assert!(usda.contains("PhysicsArticulationRootAPI"));
        // Hull purpose flipped to default (visible) in lean mode — the
        // viewport-hiding `purpose = "guide"` line should be absent.
        assert!(!usda.contains("uniform token purpose = \"guide\""));
    }

    // FR-119
    #[test]
    fn pedestal_raises_robot_and_emits_cylinder() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("r1.urdf");
        write_minimal_urdf(&urdf, "r1", 1);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "ped".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "ROBO".into(),
                dressed_urdf: urdf,
                position: [1.0, 2.0, 3.0],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: Some(crate::scene::Pedestal {
                    height_mm: 500.0,
                    diameter_mm: 800.0,
                }),
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        compose_stage_with_options(&scene, &manifest, &out, &ComposeOptions::default()).unwrap();
        let usda = read_zip_entry(&out, "scene.usda");

        // Static pedestal cylinder: 800mm dia → r=0.4, 500mm tall → h=0.5,
        // centred at z=0.25 so it spans floor (0) to height (0.5).
        assert!(usda.contains("def Scope \"pedestals\""));
        assert!(usda.contains("def Xform \"pedestal_ROBO\""));
        assert!(usda.contains("def Cylinder \"socket\""));
        assert!(usda.contains("PhysicsCollisionAPI"));
        assert!(usda.contains("double radius = 0.4"));
        assert!(usda.contains("double height = 0.5"));
        assert!(usda.contains("double3 xformOp:translate = (1, 2, 0.25)"));

        // Robot foot raised by the pedestal height: z 3.0 → 3.5, on both the
        // Xform translate and the world_anchor localPos0.
        assert!(usda.contains("double3 xformOp:translate = (1, 2, 3.5)"));
        assert!(usda.contains("point3f physics:localPos0 = (1, 2, 3.5)"));
    }

    // FR-119
    #[test]
    fn no_pedestal_means_no_scope_and_unchanged_z() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("r1.urdf");
        write_minimal_urdf(&urdf, "r1", 1);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "flat".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "ROBO".into(),
                dressed_urdf: urdf,
                position: [1.0, 2.0, 3.0],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        compose_stage_with_options(&scene, &manifest, &out, &ComposeOptions::default()).unwrap();
        let usda = read_zip_entry(&out, "scene.usda");

        assert!(!usda.contains("def Scope \"pedestals\""));
        assert!(usda.contains("double3 xformOp:translate = (1, 2, 3)"));
    }

    /// Build a one-link URDF with two visuals: one robot mesh under
    /// `./meshes/robot/`, one tool mesh under `./meshes/tool/`. Used to
    /// verify the `--full-tools=false` toggle independently of `--full-robots`.
    fn write_robot_with_tool_urdf(path: &Path) {
        let dir = path.parent().unwrap();
        std::fs::create_dir_all(dir.join("meshes/robot")).unwrap();
        std::fs::create_dir_all(dir.join("meshes/tool")).unwrap();
        for sub in ["robot", "tool"] {
            let p = dir.join(format!("meshes/{}/m.obj", sub));
            let mut f = std::fs::File::create(&p).unwrap();
            writeln!(f, "v 0 0 0").unwrap();
            writeln!(f, "v 1 0 0").unwrap();
            writeln!(f, "v 0 1 0").unwrap();
            writeln!(f, "f 1 2 3").unwrap();
        }
        let s = "<?xml version=\"1.0\"?>\n<robot name=\"rt\">\n  <link name=\"link_0\">\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/robot/m.obj\"/>\n      </geometry>\n    </visual>\n    <visual>\n      <geometry>\n        <mesh filename=\"./meshes/tool/m.obj\"/>\n      </geometry>\n    </visual>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/robot/m.obj\"/>\n      </geometry>\n    </collision>\n    <collision>\n      <geometry>\n        <mesh filename=\"./meshes/tool/m.obj\"/>\n      </geometry>\n    </collision>\n  </link>\n</robot>\n";
        std::fs::File::create(path)
            .unwrap()
            .write_all(s.as_bytes())
            .unwrap();
    }

    #[test]
    fn full_tools_off_drops_only_tool_visuals() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("rt.urdf");
        write_robot_with_tool_urdf(&urdf);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "tools-off".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "RT".into(),
                dressed_urdf: urdf,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        // full_robots=true keeps the robot visual; full_tools=false drops
        // the tool visual. We expect exactly one `def Mesh "visual_*"` (the
        // robot one at index 0) and both collisions emitted.
        let opts = ComposeOptions {
            full_robots: true,
            full_tools: false,
        };
        let _report =
            compose_stage_with_options(&scene, &manifest, &out, &opts).unwrap();

        let usda = read_zip_entry(&out, "scene.usda");
        assert!(
            usda.contains("def Mesh \"visual_0\""),
            "robot visual should be emitted"
        );
        assert!(
            !usda.contains("def Mesh \"visual_1\""),
            "tool visual should be suppressed by --full-tools=false"
        );
        // Both collisions still present (physics geometry is non-optional).
        assert!(usda.contains("def Mesh \"collision_0\""));
        assert!(usda.contains("def Mesh \"collision_1\""));
    }

    #[test]
    fn revolute_joint_emits_physics_revolute() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("r2.urdf");
        write_minimal_urdf(&urdf, "r2", 2);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "t".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "R".into(),
                dressed_urdf: urdf,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        compose_stage(&scene, &manifest, &out).unwrap();
        let usda = read_zip_entry(&out, "scene.usda");
        assert!(usda.contains("def PhysicsRevoluteJoint \"joint_0\""));
        assert!(usda.contains("rel physics:body0 = </World/robots/R/link_0>"));
        assert!(usda.contains("rel physics:body1 = </World/robots/R/link_1>"));
        // Revolute limits are emitted in degrees (URDF stores radians).
        assert!(usda.contains("physics:lowerLimit = -85.943669"));
        assert!(usda.contains("physics:upperLimit = 85.943669"));
        // Non-root link gets RigidBodyAPI; root (link_0) does not
        assert!(usda.contains("PhysicsRigidBodyAPI"));
    }

    #[test]
    fn fixture_with_actuated_joint_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("bad_fixture.urdf");
        write_minimal_urdf(&urdf, "bad", 2);
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "t".into(),
            description: "".into(),
            robots: vec![],
            fixtures: vec![FixturePlacement {
                id: "F1".into(),
                urdf,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
            }],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        let err = compose_stage(&scene, &manifest, &out).unwrap_err();
        match err {
            FittingRoomError::Validation(msg) => assert!(msg.contains("non-fixed joint")),
            other => panic!("expected Validation, got {:?}", other),
        }
    }

    #[test]
    fn fixture_with_fixed_joint_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("fix.urdf");
        write_fixed_only_urdf(&urdf, "fix");
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "t".into(),
            description: "".into(),
            robots: vec![],
            fixtures: vec![FixturePlacement {
                id: "FX1".into(),
                urdf,
                position: [10.0, 20.0, 0.0],
                orientation_deg: [0.0; 3],
            }],
            floor_plan: None,
        };
        let out = dir.path().join("scene.zip");
        let report = compose_stage(&scene, &manifest, &out).unwrap();
        assert_eq!(report.fixture_count, 1);
        let usda = read_zip_entry(&out, "scene.usda");
        assert!(usda.contains("def Scope \"fixtures\""));
        assert!(usda.contains("def Xform \"FX1\""));
    }

    #[test]
    fn unsupported_format_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "t".into(),
            description: "".into(),
            robots: vec![],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("scene.usda");
        let err = compose_stage(&scene, &manifest, &out).unwrap_err();
        assert!(matches!(err, FittingRoomError::UnsupportedFormat(_)));
    }

    #[test]
    fn floor_plan_emits_basiscurves() {
        use crate::floor_plan::FloorPlan;
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("scene.yaml");
        write_dummy_manifest(&manifest);

        let scene = SceneDefinition {
            name: "t".into(),
            description: "".into(),
            robots: vec![],
            fixtures: vec![],
            floor_plan: Some(FloorPlan {
                segments: vec![
                    [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0]],
                    [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
                ],
                footprint_segments: vec![[[2.0, 2.0, 0.0], [3.0, 2.0, 0.0]]],
            }),
        };
        let out = dir.path().join("scene.zip");
        let report = compose_stage(&scene, &manifest, &out).unwrap();
        assert_eq!(report.floor_plan_segments, 3);
        let usda = read_zip_entry(&out, "scene.usda");
        // Floor plan emission is currently disabled (Hydra CAR-811). The
        // manifest-derived segment count is still reported, but there is
        // no `floor_plan` prim in the output.
        assert!(!usda.contains("def Xform \"floor_plan\""));
    }

    #[test]
    fn manifest_yaml_in_zip() {
        let dir = tempfile::tempdir().unwrap();
        let urdf = dir.path().join("r.urdf");
        write_minimal_urdf(&urdf, "r", 1);
        let manifest = dir.path().join("k8urw11-cell.yaml");
        std::fs::write(&manifest, "name: \"K8URW11\"\nrobots: []\nfixtures: []\n").unwrap();

        let scene = SceneDefinition {
            name: "K8URW11".into(),
            description: "".into(),
            robots: vec![RobotPlacement {
                robot_id: "R".into(),
                dressed_urdf: urdf,
                position: [0.0; 3],
                orientation_deg: [0.0; 3],
                home_pose_deg: None,
                pedestal: None,
            }],
            fixtures: vec![],
            floor_plan: None,
        };
        let out = dir.path().join("k8urw11-cell.zip");
        let report = compose_stage(&scene, &manifest, &out).unwrap();
        assert_eq!(report.manifest_name, "k8urw11-cell.yaml");
        assert_eq!(report.usda_name, "k8urw11-cell.usda");

        let yaml = read_zip_entry(&out, "k8urw11-cell.yaml");
        assert!(yaml.contains("K8URW11"));
    }

    #[test]
    fn abc_to_quat_zero() {
        let q = abc_deg_to_quat([0.0, 0.0, 0.0]);
        assert!(q[0].abs() < 1e-9);
        assert!(q[1].abs() < 1e-9);
        assert!(q[2].abs() < 1e-9);
        assert!((q[3] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn sanitize_handles_special_chars() {
        assert_eq!(sanitize_prim_name("113190R01"), "_113190R01");
        assert_eq!(sanitize_prim_name("A-B"), "A_B");
        assert_eq!(sanitize_prim_name("normal_name"), "normal_name");
    }

    #[test]
    fn canonical_axis_z() {
        assert_eq!(canonical_axis_token(&[0.0, 0.0, 1.0]), "Z");
        assert_eq!(canonical_axis_token(&[1.0, 0.0, 0.0]), "X");
        assert_eq!(canonical_axis_token(&[0.0, 1.0, 0.0]), "Y");
    }
}
