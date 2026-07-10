use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::error::FittingRoomError;

/// Process-wide state shared across handlers. Wrapped in `Arc` for cheap
/// cloning into each request.
#[derive(Debug, Clone)]
pub struct AppState {
    pub asset_root: Arc<PathBuf>,
    /// When `Some`, serve SPA assets from disk (live development).
    /// When `None`, serve the SPA embedded into the binary at compile time.
    pub web_root: Option<Arc<PathBuf>>,
}

impl AppState {
    pub fn new(asset_root: PathBuf, web_root: Option<PathBuf>) -> Self {
        Self {
            asset_root: Arc::new(asset_root),
            web_root: web_root.map(Arc::new),
        }
    }

    /// Resolve a user-supplied relative path against `asset_root`, refusing
    /// anything that escapes the sandbox via `..` or absolute paths.
    pub fn resolve_asset(&self, rel: &str) -> Result<PathBuf, FittingRoomError> {
        let candidate = Path::new(rel);
        if candidate.is_absolute() {
            return Err(FittingRoomError::Validation(format!(
                "asset path must be relative: {}",
                rel
            )));
        }
        // Reject explicit traversal segments. Some callers may have URL-encoded
        // dot-dot, so we both decode-once and check the literal component list.
        for comp in candidate.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(FittingRoomError::Validation(format!(
                    "asset path may not contain '..': {}",
                    rel
                )));
            }
        }
        let joined = self.asset_root.join(candidate);
        // Canonicalise both ends and verify joined is a descendant of root.
        // `canonicalize` requires the file to exist. When the file isn't
        // there yet (e.g. a fresh `save`), canonicalise the deepest
        // existing ancestor and re-attach the trailing components, so
        // descendants of asset_root pass the containment check even
        // when symlinks (like /tmp -> /private/tmp on macOS) put
        // asset_root on a longer canonical path.
        let canon_root = std::fs::canonicalize(self.asset_root.as_path())
            .unwrap_or_else(|_| self.asset_root.as_path().to_path_buf());
        let canon_joined = canonicalize_existing_prefix(&joined);
        if !canon_joined.starts_with(&canon_root) {
            return Err(FittingRoomError::Validation(format!(
                "asset path escapes asset_root: {}",
                rel
            )));
        }
        Ok(canon_joined)
    }
}

/// Canonicalise the deepest existing prefix of `p` and re-attach the
/// trailing components. Useful for "the file might not exist yet".
fn canonicalize_existing_prefix(p: &Path) -> PathBuf {
    let mut existing = p.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !existing.exists() {
        let name = match existing.file_name() {
            Some(n) => n.to_os_string(),
            None => break,
        };
        tail.push(name);
        if !existing.pop() {
            break;
        }
    }
    let canon = std::fs::canonicalize(&existing).unwrap_or(existing);
    let mut out = canon;
    for n in tail.into_iter().rev() {
        out.push(n);
    }
    out
}
